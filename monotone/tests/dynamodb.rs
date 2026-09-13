//! Integration tests for the DynamoDB backend.
//!
//! These run against any DynamoDB endpoint named by `AWS_ENDPOINT_URL`,
//! normally DynamoDB Local:
//!
//! ```text
//! docker run -d -p 8000:8000 amazon/dynamodb-local -jar DynamoDBLocal.jar -inMemory -sharedDb
//! AWS_ENDPOINT_URL=http://localhost:8000 cargo test -p monotone --all-features
//! ```
//!
//! Without `AWS_ENDPOINT_URL` each test prints a notice and passes, unless
//! `MONOTONE_REQUIRE_INTEGRATION` is set, in which case it fails.

#![cfg(feature = "dynamodb")]

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aws_config::BehaviorVersion;
use aws_sdk_dynamodb::Client;
use aws_sdk_dynamodb::types::AttributeValue;
use monotone::core::encode::Attr;
use monotone::core::machine::RetryPolicy;
use monotone::dynamodb::{self, Counter, Error, Queue, table};
use monotone::{MonotonicCounter, MonotonicQueue, Tags, memory};

const TABLE: &str = "monotone-it";

async fn client() -> Option<Client> {
    if std::env::var_os("AWS_ENDPOINT_URL").is_none() {
        assert!(
            std::env::var_os("MONOTONE_REQUIRE_INTEGRATION").is_none(),
            "MONOTONE_REQUIRE_INTEGRATION is set but AWS_ENDPOINT_URL is not"
        );
        eprintln!("skipping DynamoDB integration test: AWS_ENDPOINT_URL is not set");
        return None;
    }
    let mut loader = aws_config::defaults(BehaviorVersion::latest());
    if std::env::var_os("AWS_ACCESS_KEY_ID").is_none() {
        loader = loader.test_credentials();
    }
    if std::env::var_os("AWS_REGION").is_none() && std::env::var_os("AWS_DEFAULT_REGION").is_none()
    {
        loader = loader.region(aws_config::Region::new("eu-west-1"));
    }
    Some(Client::new(&loader.load().await))
}

/// A client with the shared test table ready.
async fn ready() -> Option<Client> {
    let client = client().await?;
    table::create_table_if_needed(&client, TABLE, 5, 5)
        .await
        .expect("create table");
    table::wait_for_table(&client, TABLE)
        .await
        .expect("wait for table");
    Some(client)
}

fn unique(prefix: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!(
        "{prefix}-{}-{nanos}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn fast() -> RetryPolicy {
    RetryPolicy {
        retry_time: Duration::from_millis(5),
        jitter_millis: 10,
    }
}

fn tags(pairs: &[(&str, &str)]) -> Tags {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

async fn raw_item(client: &Client, id: &str) -> Option<HashMap<String, AttributeValue>> {
    client
        .get_item()
        .table_name(TABLE)
        .key("ID", AttributeValue::S(id.into()))
        .consistent_read(true)
        .send()
        .await
        .unwrap()
        .item
}

async fn put_raw(client: &Client, item: HashMap<String, AttributeValue>) {
    client
        .put_item()
        .table_name(TABLE)
        .set_item(Some(item))
        .send()
        .await
        .unwrap();
}

fn s(v: &str) -> AttributeValue {
    AttributeValue::S(v.into())
}

fn n(v: &str) -> AttributeValue {
    AttributeValue::N(v.into())
}

// ---- table provisioning ----

#[tokio::test]
async fn table_create_describe_and_errors() {
    let Some(client) = client().await else { return };
    let name = unique("tbl");

    assert!(matches!(
        table::describe_table(&client, &name).await,
        Err(Error::TableNotFound(t)) if t == name
    ));

    let created = table::create_table_if_needed(&client, &name, 1, 1)
        .await
        .unwrap();
    let active = table::wait_for_table(&client, &name).await.unwrap();
    assert_eq!(active.table_name(), Some(name.as_str()));
    let keys: Vec<_> = active
        .key_schema()
        .iter()
        .map(|k| (k.attribute_name(), k.key_type().as_str()))
        .collect();
    assert_eq!(keys, vec![("ID", "HASH")]);

    let again = table::create_table_if_needed(&client, &name, 1, 1)
        .await
        .unwrap();
    assert_eq!(again.creation_date_time(), created.creation_date_time());

    assert!(matches!(
        table::create_table(&client, &name, 1, 1).await,
        Err(Error::TableAlreadyExists(t)) if t == name
    ));
    assert!(table::list_tables(&client).await.unwrap().contains(&name));

    client
        .delete_table()
        .table_name(&name)
        .send()
        .await
        .unwrap();
}

#[tokio::test]
async fn concurrent_table_creation_is_tolerated() {
    let Some(client) = client().await else { return };
    let name = unique("race");
    let handles: Vec<_> = (0..5)
        .map(|_| {
            let (c, n) = (client.clone(), name.clone());
            tokio::spawn(async move { table::create_table_if_needed(&c, &n, 1, 1).await })
        })
        .collect();
    for h in handles {
        h.await.unwrap().expect("every racer gets the table");
    }
    client
        .delete_table()
        .table_name(&name)
        .send()
        .await
        .unwrap();
}

// ---- counter ----

#[tokio::test]
async fn counter_read_without_item_is_zero_and_creates_nothing() {
    let Some(client) = ready().await else { return };
    let c = Counter::new(client.clone(), TABLE, unique("c"));
    assert_eq!(c.get_value().await.unwrap(), 0);
    assert!(raw_item(&client, c.id()).await.is_none());
}

#[tokio::test]
async fn counter_first_increment_creates_item() {
    let Some(client) = ready().await else { return };
    let c = Counter::new(client.clone(), TABLE, unique("c"));
    assert_eq!(c.next_value().await.unwrap(), 1);
    let item = raw_item(&client, c.id()).await.unwrap();
    assert_eq!(item["Type"], s("COUNTER"));
    assert_eq!(item["Version"], n("1"));
    assert_eq!(item["Value"], n("1"));
    assert_eq!(item["ID"], s(c.id()));
}

#[tokio::test]
async fn counter_repeated_increments_and_remove() {
    let Some(client) = ready().await else { return };
    let c = Counter::new(client, TABLE, unique("c"));
    assert_eq!(c.next_value().await.unwrap(), 1);
    assert_eq!(c.next_value().await.unwrap(), 2);
    assert_eq!(c.get_value().await.unwrap(), 2);
    c.remove().await.unwrap();
    assert_eq!(c.get_value().await.unwrap(), 0);
    c.remove().await.unwrap();
}

#[tokio::test]
async fn counter_on_queue_item_and_queue_on_counter_item_are_wrong_type() {
    let Some(client) = ready().await else { return };
    let id = unique("x");
    Queue::new(client.clone(), TABLE, &id)
        .join_queue("p", None)
        .await
        .unwrap();
    let err = Counter::new(client.clone(), TABLE, &id)
        .get_value()
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::Core(monotone::Error::WrongType { expected: "COUNTER", found }) if found == "QUEUE"),
        "{err}"
    );

    let id = unique("y");
    Counter::new(client.clone(), TABLE, &id)
        .next_value()
        .await
        .unwrap();
    let err = Queue::new(client, TABLE, &id)
        .get_tickets()
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            Error::Core(monotone::Error::WrongType {
                expected: "QUEUE",
                ..
            })
        ),
        "{err}"
    );
}

#[tokio::test]
async fn counter_with_missing_or_bad_attributes_names_them() {
    let Some(client) = ready().await else { return };
    let id = unique("bad");
    put_raw(
        &client,
        HashMap::from([
            ("ID".into(), s(&id)),
            ("Type".into(), s("COUNTER")),
            ("Version".into(), n("1")),
        ]),
    )
    .await;
    let err = Counter::new(client.clone(), TABLE, &id)
        .get_value()
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Core(monotone::Error::MissingAttribute("Value"))),
        "{err}"
    );

    let id = unique("bad");
    put_raw(
        &client,
        HashMap::from([
            ("ID".into(), s(&id)),
            ("Type".into(), s("COUNTER")),
            ("Version".into(), n("1")),
            ("Value".into(), n("-3")),
        ]),
    )
    .await;
    let err = Counter::new(client, TABLE, &id)
        .next_value()
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            Error::Core(monotone::Error::InvalidAttribute { name: "Value", .. })
        ),
        "{err}"
    );
}

// ---- queue ----

#[tokio::test]
async fn queue_numbering_tokens_and_item_encoding() {
    let Some(client) = ready().await else { return };
    let q = Queue::new(client.clone(), TABLE, unique("q"));

    assert_eq!(q.get_tickets().await.unwrap(), (0, vec![]));
    assert!(raw_item(&client, q.id()).await.is_none());

    let (t1, foo) = q.join_queue("foo", None).await.unwrap();
    assert_eq!((t1, foo.counter, foo.position), (1, 1, 0));
    let item = raw_item(&client, q.id()).await.unwrap();
    assert_eq!(item["Type"], s("QUEUE"));
    assert_eq!(
        item["Items"],
        AttributeValue::Ss(vec![r#"{"process_id":"foo","counter":1,"tags":{}}"#.into()])
    );

    let (t2, bar) = q.join_queue("bar", None).await.unwrap();
    assert_eq!((t2, bar.counter, bar.position), (2, 2, 1));
    assert_eq!(q.get_ticket("foo").await.unwrap(), (2, foo.clone()));

    assert_eq!(q.leave_queue("foo").await.unwrap(), 3);
    let (t, now) = q.get_ticket("bar").await.unwrap();
    assert_eq!((t, now.position, now.counter), (3, 0, 2));

    assert_eq!(q.leave_queue("bar").await.unwrap(), 4);
    let item = raw_item(&client, q.id()).await.unwrap();
    assert!(!item.contains_key("Items"), "empty queue omits Items");
    assert_eq!(item["Value"], n("2"));
}

#[tokio::test]
async fn queue_rejoin_tags_and_lookups() {
    let Some(client) = ready().await else { return };
    let q = Queue::new(client, TABLE, unique("q"));
    let t = tags(&[("role", "leader"), ("rack", "a")]);
    let first = q.join_queue("foo", t.clone()).await.unwrap();
    assert_eq!(first.1.tags, t);
    assert_eq!(q.join_queue("foo", None).await.unwrap(), first);
    let (token, all) = q.get_tickets().await.unwrap();
    assert_eq!((token, all.len()), (1, 1));
    assert_eq!(all[0].tags, t);

    assert!(matches!(
        q.get_ticket("nobody").await,
        Err(Error::Core(monotone::Error::NotFound(p))) if p == "nobody"
    ));
    assert!(matches!(
        q.leave_queue("nobody").await,
        Err(Error::Core(monotone::Error::NotFound(_)))
    ));
    assert_eq!(q.get_tickets().await.unwrap().0, 1);
}

#[tokio::test]
async fn leave_and_lookup_on_missing_queue_return_promptly() {
    let Some(client) = ready().await else { return };
    let q = Queue::new(client, TABLE, unique("missing"));
    let leave = tokio::time::timeout(Duration::from_secs(5), q.leave_queue("foo")).await;
    assert!(
        matches!(leave, Ok(Err(Error::Core(monotone::Error::NotFound(_))))),
        "leave on a missing queue must fail promptly: {leave:?}"
    );
    assert!(matches!(
        q.get_ticket("foo").await,
        Err(Error::Core(monotone::Error::NotFound(_)))
    ));
}

#[tokio::test]
async fn queue_remove_resets_token_and_counter() {
    let Some(client) = ready().await else { return };
    let q = Queue::new(client, TABLE, unique("q"));
    q.join_queue("a", None).await.unwrap();
    q.join_queue("b", None).await.unwrap();
    q.remove().await.unwrap();
    assert_eq!(q.get_tickets().await.unwrap(), (0, vec![]));
    assert_eq!(q.join_queue("c", None).await.unwrap().1.counter, 1);
    q.remove().await.unwrap();
    q.remove().await.unwrap();
}

#[tokio::test]
async fn concurrent_joins_from_many_clients_get_distinct_counters() {
    let Some(client) = ready().await else { return };
    let id = unique("q");
    let handles: Vec<_> = (0..20)
        .map(|i| {
            let q = Queue::new(client.clone(), TABLE, &id).with_retry_policy(fast());
            tokio::spawn(async move { q.join_queue(&format!("p{i}"), None).await.unwrap() })
        })
        .collect();
    let mut counters = BTreeSet::new();
    for h in handles {
        counters.insert(h.await.unwrap().1.counter);
    }
    assert_eq!(counters, (1..=20).collect());
    let (token, tickets) = Queue::new(client, TABLE, &id).get_tickets().await.unwrap();
    assert_eq!(token, 20);
    assert_eq!(
        tickets.iter().map(|t| t.position).collect::<Vec<_>>(),
        (0..20).collect::<Vec<_>>()
    );
}

// ---- real conflicts ----

#[tokio::test]
async fn two_clients_racing_a_conditional_put_exactly_one_wins() {
    let Some(a) = ready().await else { return };
    let b = client().await.unwrap();
    let id = unique("race");

    assert!(dynamodb::read_item(&a, TABLE, &id).await.unwrap().is_none());
    assert!(dynamodb::read_item(&b, TABLE, &id).await.unwrap().is_none());

    let item = |value: &str| {
        monotone::core::encode::Item::from([
            ("ID".to_string(), Attr::S(id.clone())),
            ("Type".to_string(), Attr::S("COUNTER".into())),
            ("Version".to_string(), Attr::N("1".into())),
            ("Value".to_string(), Attr::N(value.into())),
        ])
    };
    let ra = dynamodb::put_if_version(&a, TABLE, &item("1"), 0).await;
    let rb = dynamodb::put_if_version(&b, TABLE, &item("1"), 0).await;
    let outcomes = [ra.is_ok(), rb.is_ok()];
    assert_eq!(
        outcomes.iter().filter(|ok| **ok).count(),
        1,
        "{ra:?} {rb:?}"
    );
    assert!(matches!(
        ra.err().or(rb.err()),
        Some(Error::ConditionalUpdateFailed)
    ));
}

#[tokio::test]
async fn concurrent_next_value_on_two_clients_yields_consecutive_values() {
    let Some(a) = ready().await else { return };
    let b = client().await.unwrap();
    let id = unique("c");
    let mut handles = Vec::new();
    for i in 0..40 {
        let client = if i % 2 == 0 { a.clone() } else { b.clone() };
        let c = Counter::new(client, TABLE, &id).with_retry_policy(fast());
        handles.push(tokio::spawn(async move { c.next_value().await.unwrap() }));
    }
    let mut values = BTreeSet::new();
    for h in handles {
        assert!(values.insert(h.await.unwrap()), "duplicate value");
    }
    assert_eq!(values, (1..=40).collect());
}

// ---- equivalence and compatibility ----

async fn script<C, Q>(counter: &C, queue: &Q) -> Vec<String>
where
    C: MonotonicCounter,
    C::Error: std::fmt::Debug,
    Q: MonotonicQueue,
    Q::Error: std::fmt::Debug,
{
    vec![
        format!(
            "{:?}",
            queue.join_queue("foo", tags(&[("k", "v")])).await.unwrap()
        ),
        format!("{:?}", queue.join_queue("bar", None).await.unwrap()),
        format!("{:?}", queue.join_queue("foo", None).await.unwrap()),
        format!("{:?}", queue.leave_queue("foo").await.unwrap()),
        format!("{:?}", queue.get_ticket("bar").await.unwrap()),
        format!("{:?}", queue.get_tickets().await.unwrap()),
        format!("{:?}", queue.leave_queue("foo").await.is_err()),
        format!("{:?}", counter.get_value().await.unwrap()),
        format!("{:?}", counter.next_value().await.unwrap()),
        format!("{:?}", counter.next_value().await.unwrap()),
        format!("{:?}", counter.next_value().await.unwrap()),
    ]
}

#[tokio::test]
async fn memory_and_dynamodb_backends_agree() {
    let Some(client) = ready().await else { return };
    let store = memory::Store::new();
    let in_memory = script(&store.counter("c"), &store.queue("q")).await;
    let on_dynamo = script(
        &Counter::new(client.clone(), TABLE, unique("eq-c")),
        &Queue::new(client, TABLE, unique("eq-q")),
    )
    .await;
    assert_eq!(in_memory, on_dynamo);
}

#[tokio::test]
async fn reads_and_extends_a_0_4_queue_item() {
    let Some(client) = ready().await else { return };
    let id = unique("legacy");
    put_raw(
        &client,
        HashMap::from([
            ("ID".into(), s(&id)),
            ("Type".into(), s("QUEUE")),
            ("Version".into(), n("2")),
            ("Value".into(), n("2")),
            (
                "Items".into(),
                AttributeValue::Ss(vec![
                    r#"{"process_id":"old-null","counter":1,"tags":null}"#.into(),
                    r#"{"process_id":"old-absent","counter":2}"#.into(),
                ]),
            ),
        ]),
    )
    .await;

    let q = Queue::new(client, TABLE, &id);
    let (token, tickets) = q.get_tickets().await.unwrap();
    assert_eq!(token, 2);
    assert_eq!(
        tickets
            .iter()
            .map(|t| (t.process_id.as_str(), t.counter, t.tags.is_empty()))
            .collect::<Vec<_>>(),
        vec![("old-null", 1, true), ("old-absent", 2, true)]
    );

    let (token, ticket) = q.join_queue("new", None).await.unwrap();
    assert_eq!((token, ticket.counter, ticket.position), (3, 3, 2));
    assert_eq!(q.get_tickets().await.unwrap().1.len(), 3);
}

// ---- error mapping ----

#[tokio::test]
async fn missing_table_and_unreachable_endpoint_are_wrapped_sdk_errors() {
    let Some(client) = client().await else { return };
    let err = Counter::new(client, unique("no-such-table"), "x")
        .get_value()
        .await
        .unwrap_err();
    match &err {
        Error::Sdk {
            operation, message, ..
        } => {
            assert_eq!(*operation, "GetItem");
            assert!(message.contains("ResourceNotFound"), "{message}");
        }
        other => panic!("expected Sdk error, got {other:?}"),
    }
    assert!(
        std::error::Error::source(&err).is_some(),
        "original error preserved"
    );

    let config = aws_config::defaults(BehaviorVersion::latest())
        .test_credentials()
        .region(aws_config::Region::new("eu-west-1"))
        .endpoint_url("http://127.0.0.1:9")
        .retry_config(aws_config::retry::RetryConfig::disabled())
        .load()
        .await;
    let err = Counter::new(Client::new(&config), TABLE, "x")
        .get_value()
        .await
        .unwrap_err();
    assert!(
        matches!(
            &err,
            Error::Sdk {
                operation: "GetItem",
                ..
            }
        ),
        "{err:?}"
    );
    assert!(err.to_string().contains("dispatch failure"), "{err}");
}
