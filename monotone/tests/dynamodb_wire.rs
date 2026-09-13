//! Wire-level tests for the DynamoDB backend.
//!
//! A scripted HTTP client records every request and answers with canned
//! DynamoDB responses. This checks request contents and error classification
//! that DynamoDB Local cannot show: consistent-read flags, table status
//! polling, and service error types. No network is used.

#![cfg(feature = "dynamodb")]

use std::sync::{Arc, Mutex};

use aws_sdk_dynamodb::Client;
use aws_sdk_dynamodb::config::{BehaviorVersion, Credentials, Region};
use aws_smithy_http_client::test_util::infallible_client_fn;
use monotone::MonotonicCounter;
use monotone::core::encode::{Attr, Item};
use monotone::dynamodb::{self, Counter, Error, table};
use serde_json::{Value, json};

#[derive(Clone, Debug)]
struct Recorded {
    target: String,
    uri: String,
    body: Value,
}

type Responder = dyn Fn(&str, usize) -> (u16, Value) + Send + Sync;

/// A client whose HTTP layer records requests and replies via `respond`,
/// which receives the operation name and how many times it has been called.
fn scripted(
    endpoint: Option<&str>,
    respond: Arc<Responder>,
) -> (Client, Arc<Mutex<Vec<Recorded>>>) {
    let log: Arc<Mutex<Vec<Recorded>>> = Arc::default();
    let log2 = log.clone();
    let http = infallible_client_fn(move |req| {
        let target = req
            .headers()
            .get("x-amz-target")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .trim_start_matches("DynamoDB_20120810.")
            .to_owned();
        let body = req
            .body()
            .bytes()
            .map(|b| serde_json::from_slice(b).unwrap())
            .unwrap_or(Value::Null);
        let mut log = log2.lock().unwrap();
        let nth = log.iter().filter(|r| r.target == target).count();
        log.push(Recorded {
            target: target.clone(),
            uri: req.uri().to_string(),
            body,
        });
        let (status, reply) = respond(&target, nth);
        http::Response::builder()
            .status(status)
            .header("content-type", "application/x-amz-json-1.0")
            .body(reply.to_string())
            .unwrap()
    });
    let mut config = aws_sdk_dynamodb::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("eu-west-1"))
        .credentials_provider(Credentials::for_tests())
        .http_client(http);
    if let Some(url) = endpoint {
        config = config.endpoint_url(url);
    }
    (Client::from_conf(config.build()), log)
}

fn service_error(kind: &str, message: &str) -> (u16, Value) {
    (
        400,
        json!({"__type": format!("com.amazonaws.dynamodb.v20120810#{kind}"), "message": message}),
    )
}

fn table_description(status: &str) -> Value {
    json!({"Table": {"TableName": "t", "TableStatus": status, "KeySchema": [{"AttributeName": "ID", "KeyType": "HASH"}]}})
}

fn requests(log: &Arc<Mutex<Vec<Recorded>>>) -> Vec<Recorded> {
    log.lock().unwrap().clone()
}

#[tokio::test]
async fn get_item_is_strongly_consistent() {
    let (client, log) = scripted(Some("http://local"), Arc::new(|_, _| (200, json!({}))));
    assert!(
        dynamodb::read_item(&client, "t", "x")
            .await
            .unwrap()
            .is_none()
    );
    let reqs = requests(&log);
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].target, "GetItem");
    assert_eq!(
        reqs[0].body,
        json!({"TableName": "t", "Key": {"ID": {"S": "x"}}, "ConsistentRead": true})
    );
}

#[tokio::test]
async fn put_item_carries_condition_on_the_wire() {
    let (client, log) = scripted(Some("http://local"), Arc::new(|_, _| (200, json!({}))));
    let item = Item::from([
        ("ID".to_string(), Attr::S("x".into())),
        ("Type".to_string(), Attr::S("COUNTER".into())),
        ("Version".to_string(), Attr::N("5".into())),
        ("Value".to_string(), Attr::N("5".into())),
    ]);
    dynamodb::put_if_version(&client, "t", &item, 4)
        .await
        .unwrap();
    let body = &requests(&log)[0].body;
    assert_eq!(
        body["ConditionExpression"],
        "Version = :version OR attribute_not_exists(Version)"
    );
    assert_eq!(
        body["ExpressionAttributeValues"],
        json!({":version": {"N": "4"}})
    );
    assert_eq!(
        body["Item"],
        json!({"ID": {"S": "x"}, "Type": {"S": "COUNTER"}, "Version": {"N": "5"}, "Value": {"N": "5"}})
    );
}

#[tokio::test]
async fn service_errors_are_classified_by_type() {
    let (client, _) = scripted(
        Some("http://local"),
        Arc::new(|op, _| match op {
            "PutItem" => service_error(
                "ConditionalCheckFailedException",
                "The conditional request failed",
            ),
            "DescribeTable" => {
                service_error("ResourceNotFoundException", "Requested resource not found")
            }
            "CreateTable" => service_error("ResourceInUseException", "Table already exists: t"),
            "GetItem" => service_error("ProvisionedThroughputExceededException", "slow down"),
            other => panic!("unexpected {other}"),
        }),
    );
    let client = Client::from_conf(
        client
            .config()
            .to_builder()
            .retry_config(aws_sdk_dynamodb::config::retry::RetryConfig::disabled())
            .build(),
    );

    let item = Item::from([("ID".to_string(), Attr::S("x".into()))]);
    assert!(matches!(
        dynamodb::put_if_version(&client, "t", &item, 0).await,
        Err(Error::ConditionalUpdateFailed)
    ));
    assert!(
        matches!(table::describe_table(&client, "t").await, Err(Error::TableNotFound(t)) if t == "t")
    );
    assert!(
        matches!(table::create_table(&client, "t", 1, 1).await, Err(Error::TableAlreadyExists(t)) if t == "t")
    );

    let err = dynamodb::read_item(&client, "t", "x").await.unwrap_err();
    match &err {
        Error::Sdk {
            operation, message, ..
        } => {
            assert_eq!(*operation, "GetItem");
            assert!(
                message.contains("ProvisionedThroughputExceeded"),
                "{message}"
            );
        }
        other => panic!("expected Sdk, got {other:?}"),
    }
    assert!(std::error::Error::source(&err).is_some());
}

#[tokio::test]
async fn non_conditional_put_failure_aborts_without_retry() {
    let (client, log) = scripted(
        Some("http://local"),
        Arc::new(|op, _| match op {
            "GetItem" => (200, json!({})),
            "PutItem" => service_error("ValidationException", "bad"),
            other => panic!("unexpected {other}"),
        }),
    );
    let err = Counter::new(client, "t", "x")
        .next_value()
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            Error::Sdk {
                operation: "PutItem",
                ..
            }
        ),
        "{err:?}"
    );
    let ops: Vec<_> = requests(&log).into_iter().map(|r| r.target).collect();
    assert_eq!(ops, vec!["GetItem", "PutItem"]);
}

#[tokio::test(start_paused = true)]
async fn wait_for_table_polls_until_active() {
    let (client, log) = scripted(
        Some("http://local"),
        Arc::new(|op, nth| {
            assert_eq!(op, "DescribeTable");
            (
                200,
                table_description(if nth < 2 { "CREATING" } else { "ACTIVE" }),
            )
        }),
    );
    let desc = table::wait_for_table(&client, "t").await.unwrap();
    assert_eq!(desc.table_status().map(|s| s.as_str()), Some("ACTIVE"));
    assert_eq!(requests(&log).len(), 3);
}

#[tokio::test]
async fn create_table_if_needed_creates_a_missing_table() {
    let (client, log) = scripted(
        Some("http://local"),
        Arc::new(|op, nth| match (op, nth) {
            ("DescribeTable", 0) => {
                service_error("ResourceNotFoundException", "Requested resource not found")
            }
            ("CreateTable", _) => (
                200,
                json!({"TableDescription": {"TableName": "t", "TableStatus": "CREATING"}}),
            ),
            ("DescribeTable", _) => (200, table_description("ACTIVE")),
            (other, _) => panic!("unexpected {other}"),
        }),
    );
    table::create_table_if_needed(&client, "t", 1, 1)
        .await
        .unwrap();
    let reqs = requests(&log);
    let ops: Vec<_> = reqs.iter().map(|r| r.target.as_str()).collect();
    assert_eq!(ops, vec!["DescribeTable", "CreateTable", "DescribeTable"]);
    let create = &reqs[1].body;
    assert_eq!(
        create["KeySchema"],
        json!([{"AttributeName": "ID", "KeyType": "HASH"}])
    );
    assert_eq!(
        create["AttributeDefinitions"],
        json!([{"AttributeName": "ID", "AttributeType": "S"}])
    );
    assert_eq!(
        create["ProvisionedThroughput"],
        json!({"ReadCapacityUnits": 1, "WriteCapacityUnits": 1})
    );
}

#[tokio::test]
async fn create_table_if_needed_leaves_an_existing_table_alone() {
    let (client, log) = scripted(
        Some("http://local"),
        Arc::new(|_, _| (200, table_description("ACTIVE"))),
    );
    table::create_table_if_needed(&client, "t", 1, 1)
        .await
        .unwrap();
    let ops: Vec<_> = requests(&log).into_iter().map(|r| r.target).collect();
    assert_eq!(ops, vec!["DescribeTable"]);
}

#[tokio::test]
async fn create_table_if_needed_tolerates_a_concurrent_creator() {
    let (client, log) = scripted(
        Some("http://local"),
        Arc::new(|op, nth| match (op, nth) {
            ("DescribeTable", 0) => {
                service_error("ResourceNotFoundException", "Requested resource not found")
            }
            ("CreateTable", _) => {
                service_error("ResourceInUseException", "Table already exists: t")
            }
            ("DescribeTable", _) => (200, table_description("ACTIVE")),
            (other, _) => panic!("unexpected {other}"),
        }),
    );
    table::create_table_if_needed(&client, "t", 1, 1)
        .await
        .unwrap();
    let ops: Vec<_> = requests(&log).into_iter().map(|r| r.target).collect();
    assert_eq!(ops, vec!["DescribeTable", "CreateTable", "DescribeTable"]);
}

#[tokio::test]
async fn endpoint_defaults_to_the_regional_service_and_honours_an_override() {
    let (client, log) = scripted(None, Arc::new(|_, _| (200, json!({}))));
    dynamodb::read_item(&client, "t", "x").await.unwrap();
    assert!(
        requests(&log)[0]
            .uri
            .starts_with("https://dynamodb.eu-west-1.amazonaws.com"),
        "{}",
        requests(&log)[0].uri
    );

    let (client, log) = scripted(
        Some("http://localhost:8000"),
        Arc::new(|_, _| (200, json!({}))),
    );
    dynamodb::read_item(&client, "t", "x").await.unwrap();
    assert!(
        requests(&log)[0].uri.starts_with("http://localhost:8000"),
        "{}",
        requests(&log)[0].uri
    );
}

#[tokio::test]
async fn load_config_applies_the_region_override() {
    let config = dynamodb::load_config(Some("ap-southeast-2".into())).await;
    assert_eq!(config.region().map(|r| r.as_ref()), Some("ap-southeast-2"));
}
