//! Executes core effects against DynamoDB.

use std::collections::HashMap;
use std::time::Duration;

use aws_sdk_dynamodb::Client;
use aws_sdk_dynamodb::types::AttributeValue;
use rand::rngs::SmallRng;
use tracing::debug;

use super::error::Error;
use crate::core::encode::{Attr, Codec, ID, Item};
use crate::core::machine::{Effect, Input, Machine, Step};
use crate::core::row::Row;

/// The condition every write carries.
pub(crate) const CONDITION: &str = "Version = :version OR attribute_not_exists(Version)";

pub(crate) fn rng() -> SmallRng {
    rand::make_rng()
}

/// Converts a core attribute to the SDK form.
pub fn attr_to_sdk(attr: &Attr) -> AttributeValue {
    match attr {
        Attr::S(s) => AttributeValue::S(s.clone()),
        Attr::N(n) => AttributeValue::N(n.clone()),
        Attr::SS(set) => AttributeValue::Ss(set.iter().cloned().collect()),
        // Core encoding never produces `Other`; round-trip it as a string so
        // decoding reports it rather than panicking here.
        Attr::Other(kind) => AttributeValue::S(format!("<unsupported {kind}>")),
    }
}

/// Converts an SDK attribute to the core form. Kinds the schema never uses
/// become [`Attr::Other`] so decoding can name them.
pub fn attr_from_sdk(value: &AttributeValue) -> Attr {
    match value {
        AttributeValue::S(s) => Attr::S(s.clone()),
        AttributeValue::N(n) => Attr::N(n.clone()),
        AttributeValue::Ss(set) => Attr::SS(set.iter().cloned().collect()),
        AttributeValue::B(_) => Attr::Other("B".into()),
        AttributeValue::Bool(_) => Attr::Other("BOOL".into()),
        AttributeValue::Bs(_) => Attr::Other("BS".into()),
        AttributeValue::L(_) => Attr::Other("L".into()),
        AttributeValue::M(_) => Attr::Other("M".into()),
        AttributeValue::Ns(_) => Attr::Other("NS".into()),
        AttributeValue::Null(_) => Attr::Other("NULL".into()),
        _ => Attr::Other("unknown".into()),
    }
}

fn item_to_sdk(item: &Item) -> HashMap<String, AttributeValue> {
    item.iter()
        .map(|(k, v)| (k.clone(), attr_to_sdk(v)))
        .collect()
}

fn item_from_sdk(item: &HashMap<String, AttributeValue>) -> Item {
    item.iter()
        .map(|(k, v)| (k.clone(), attr_from_sdk(v)))
        .collect()
}

fn key(id: &str) -> HashMap<String, AttributeValue> {
    HashMap::from([(ID.to_owned(), AttributeValue::S(id.to_owned()))])
}

/// A fully specified conditional `PutItem`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PutRequest {
    pub item: HashMap<String, AttributeValue>,
    pub condition_expression: &'static str,
    pub expression_attribute_values: HashMap<String, AttributeValue>,
}

impl PutRequest {
    pub fn new(item: &Item, expected_version: u64) -> PutRequest {
        PutRequest {
            item: item_to_sdk(item),
            condition_expression: CONDITION,
            expression_attribute_values: HashMap::from([(
                ":version".to_owned(),
                AttributeValue::N(expected_version.to_string()),
            )]),
        }
    }
}

/// Performs the I/O for one operation. Split out so the driving loop can be
/// tested without DynamoDB.
pub(crate) trait Executor {
    fn id(&self) -> &str;
    async fn get(&self) -> Result<Option<Item>, Error>;
    /// `Ok(true)` on success, `Ok(false)` on a conditional check failure.
    async fn put(&self, request: PutRequest) -> Result<bool, Error>;
    async fn sleep(&self, duration: Duration);
}

pub(crate) struct DynamoExecutor<'a> {
    client: &'a Client,
    table: &'a str,
    id: &'a str,
}

impl<'a> DynamoExecutor<'a> {
    pub fn new(client: &'a Client, table: &'a str, id: &'a str) -> Self {
        DynamoExecutor { client, table, id }
    }
}

impl Executor for DynamoExecutor<'_> {
    fn id(&self) -> &str {
        self.id
    }

    async fn get(&self) -> Result<Option<Item>, Error> {
        read_item(self.client, self.table, self.id).await
    }

    async fn put(&self, request: PutRequest) -> Result<bool, Error> {
        match send_put(self.client, self.table, request).await {
            Ok(()) => Ok(true),
            Err(Error::ConditionalUpdateFailed) => Ok(false),
            Err(e) => Err(e),
        }
    }

    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

/// Reads the item for `id` with a strongly consistent `GetItem`.
pub async fn read_item(client: &Client, table: &str, id: &str) -> Result<Option<Item>, Error> {
    let out = client
        .get_item()
        .table_name(table)
        .set_key(Some(key(id)))
        .consistent_read(true)
        .send()
        .await
        .map_err(|e| Error::sdk("GetItem", e))?;
    Ok(out.item.as_ref().map(item_from_sdk))
}

async fn send_put(client: &Client, table: &str, request: PutRequest) -> Result<(), Error> {
    let result = client
        .put_item()
        .table_name(table)
        .set_item(Some(request.item))
        .condition_expression(request.condition_expression)
        .set_expression_attribute_values(Some(request.expression_attribute_values))
        .send()
        .await;
    match result {
        Ok(_) => Ok(()),
        Err(e)
            if e.as_service_error()
                .is_some_and(|s| s.is_conditional_check_failed_exception()) =>
        {
            Err(Error::ConditionalUpdateFailed)
        }
        Err(e) => Err(Error::sdk("PutItem", e)),
    }
}

/// Stores `item` only if the stored `Version` equals `expected_version`, or no
/// item exists and `expected_version` is 0. A rejected condition is
/// [`Error::ConditionalUpdateFailed`].
pub async fn put_if_version(
    client: &Client,
    table: &str,
    item: &Item,
    expected_version: u64,
) -> Result<(), Error> {
    send_put(client, table, PutRequest::new(item, expected_version)).await
}

pub(crate) async fn delete_item(client: &Client, table: &str, id: &str) -> Result<(), Error> {
    client
        .delete_item()
        .table_name(table)
        .set_key(Some(key(id)))
        .send()
        .await
        .map_err(|e| Error::sdk("DeleteItem", e))?;
    Ok(())
}

/// Runs a machine to completion through an executor.
pub(crate) async fn drive<E, M>(executor: &E, mut machine: M) -> Result<M::Output, Error>
where
    E: Executor,
    M: Machine,
    M::Row: Codec,
{
    let id = executor.id().to_owned();
    let mut step = machine.start()?;
    loop {
        step = match step {
            Step::Done(output) => return Ok(output),
            Step::Effect(Effect::Read) => {
                let row = match executor.get().await? {
                    Some(item) => Some(M::Row::decode(&item)?),
                    None => None,
                };
                machine.step(Input::Row(row))?
            }
            Step::Effect(Effect::Write {
                row,
                expected_version,
            }) => {
                let request = PutRequest::new(&row.encode(&id), expected_version);
                if executor.put(request).await? {
                    machine.step(Input::WriteOk)?
                } else {
                    debug!(id = %id, version = row.version(), "write conflict, retrying");
                    machine.step(Input::WriteConflict)?
                }
            }
            Step::Effect(Effect::Sleep(duration)) => {
                executor.sleep(duration).await;
                machine.resume()?
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use rand::SeedableRng;

    use super::*;
    use crate::core::machine::RetryPolicy;
    use crate::core::ops;
    use crate::core::row::CounterRow;

    /// Records requests and simulates a single versioned item.
    #[derive(Default)]
    struct Stub {
        stored: Mutex<Option<(u64, Item)>>,
        puts: Mutex<Vec<PutRequest>>,
        sleeps: Mutex<Vec<Duration>>,
        /// Versions to force a conflict on, as if another writer won.
        conflict_on: Mutex<Vec<u64>>,
        get_error: bool,
    }

    impl Executor for Stub {
        fn id(&self) -> &str {
            "stub"
        }

        async fn get(&self) -> Result<Option<Item>, Error> {
            if self.get_error {
                return Err(Error::TableNotFound("t".into()));
            }
            Ok(self.stored.lock().unwrap().as_ref().map(|(_, i)| i.clone()))
        }

        async fn put(&self, request: PutRequest) -> Result<bool, Error> {
            self.puts.lock().unwrap().push(request.clone());
            let expected: u64 = match &request.expression_attribute_values[":version"] {
                AttributeValue::N(n) => n.parse().unwrap(),
                other => panic!("{other:?}"),
            };
            let mut forced = self.conflict_on.lock().unwrap();
            if let Some(pos) = forced.iter().position(|v| *v == expected) {
                forced.remove(pos);
                // Another writer bumps the stored row first.
                let mut stored = self.stored.lock().unwrap();
                let row = stored
                    .as_ref()
                    .map(|(_, i)| CounterRow::decode(i).unwrap())
                    .unwrap_or_default()
                    .increment();
                *stored = Some((row.version, row.encode("stub")));
                return Ok(false);
            }
            let mut stored = self.stored.lock().unwrap();
            let current = stored.as_ref().map_or(0, |(v, _)| *v);
            if current != expected {
                return Ok(false);
            }
            let item: Item = item_from_sdk(&request.item);
            let version: u64 = match &item["Version"] {
                Attr::N(n) => n.parse().unwrap(),
                other => panic!("{other:?}"),
            };
            *stored = Some((version, item));
            Ok(true)
        }

        async fn sleep(&self, duration: Duration) {
            self.sleeps.lock().unwrap().push(duration);
        }
    }

    #[tokio::test]
    async fn put_request_carries_exact_condition_and_version() {
        let stub = Stub::default();
        let out = drive(
            &stub,
            ops::next_value(RetryPolicy::default(), SmallRng::seed_from_u64(1)),
        )
        .await;
        assert_eq!(out.unwrap(), 1);

        let puts = stub.puts.lock().unwrap();
        assert_eq!(puts.len(), 1);
        assert_eq!(
            puts[0].condition_expression,
            "Version = :version OR attribute_not_exists(Version)"
        );
        assert_eq!(
            puts[0].expression_attribute_values,
            HashMap::from([(":version".to_string(), AttributeValue::N("0".into()))])
        );
        assert_eq!(
            puts[0].item,
            HashMap::from([
                ("ID".to_string(), AttributeValue::S("stub".into())),
                ("Type".to_string(), AttributeValue::S("COUNTER".into())),
                ("Version".to_string(), AttributeValue::N("1".into())),
                ("Value".to_string(), AttributeValue::N("1".into())),
            ])
        );
    }

    #[tokio::test]
    async fn conflict_sleeps_then_rereads_and_writes_on_top() {
        let stub = Stub {
            conflict_on: Mutex::new(vec![0]),
            ..Stub::default()
        };
        let policy = RetryPolicy {
            retry_time: Duration::from_millis(10),
            jitter_millis: 5,
        };
        let out = drive(&stub, ops::next_value(policy, SmallRng::seed_from_u64(1))).await;
        assert_eq!(out.unwrap(), 2);

        let versions: Vec<_> = stub
            .puts
            .lock()
            .unwrap()
            .iter()
            .map(|p| p.expression_attribute_values[":version"].clone())
            .collect();
        assert_eq!(
            versions,
            vec![AttributeValue::N("0".into()), AttributeValue::N("1".into())]
        );
        let sleeps = stub.sleeps.lock().unwrap();
        assert_eq!(sleeps.len(), 1);
        assert!(sleeps[0] >= Duration::from_millis(10) && sleeps[0] < Duration::from_millis(15));
    }

    #[tokio::test]
    async fn executor_errors_abort_without_retry() {
        let stub = Stub {
            get_error: true,
            ..Stub::default()
        };
        let out = drive(
            &stub,
            ops::next_value(RetryPolicy::default(), SmallRng::seed_from_u64(1)),
        )
        .await;
        assert!(matches!(out, Err(Error::TableNotFound(_))));
        assert!(stub.puts.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn decode_errors_surface_as_core_errors() {
        let stub = Stub::default();
        let mut item = CounterRow::default().encode("stub");
        item.insert("Type".into(), Attr::S("QUEUE".into()));
        *stub.stored.lock().unwrap() = Some((1, item));
        let out = drive(&stub, ops::get_value()).await;
        assert!(matches!(
            out,
            Err(Error::Core(crate::core::error::Error::WrongType {
                expected: "COUNTER",
                ..
            }))
        ));
    }

    #[test]
    fn attribute_conversion_round_trips_and_names_unsupported_kinds() {
        for attr in [
            Attr::S("x".into()),
            Attr::N("42".into()),
            Attr::SS(["a".to_string(), "b".to_string()].into()),
        ] {
            assert_eq!(attr_from_sdk(&attr_to_sdk(&attr)), attr);
        }
        assert_eq!(
            attr_from_sdk(&AttributeValue::Bool(true)),
            Attr::Other("BOOL".into())
        );
        assert_eq!(
            attr_from_sdk(&AttributeValue::Null(true)),
            Attr::Other("NULL".into())
        );
    }
}
