use std::thread;
use std::time::Duration;
use std::default::Default;
use std::collections::HashMap;
use rusoto::{ProvideAwsCredentials, DispatchSignedRequest};
use rusoto::dynamodb::*;
use ::*;
use string::*;
use time::*;
use super::*;
use super::error::*;

pub const COUNTER_TYPE: &'static str = "COUNTER";

pub struct Counter<P,D> where P: ProvideAwsCredentials, D: DispatchSignedRequest {
    pub client: DynamoDbClient<P,D>,
    pub table_name: String,
    pub id: String,
    pub retry_time: Duration,
    pub jitter_millis: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CounterRow {
    pub id: String,
    pub version: u64,
    pub value: u64,
}

impl CounterRow {
    pub fn new(id: String) -> CounterRow {
        CounterRow {
            id: id,
            version: 0,
            value: 0,
        }
    }
}

impl <P,D> Counter<P,D> where P: ProvideAwsCredentials, D: DispatchSignedRequest {
    pub fn new<S1, S2>(client: DynamoDbClient<P,D>, table_name: S1, id: S2, retry_time: Duration) -> Counter<P,D> where S1: Into<String>, S2: Into<String> {
        Counter {
            client: client,
            table_name: table_name.into(),
            id: id.into(),
            retry_time: retry_time,
            jitter_millis: 100,
        }
    }

    pub fn remove(&self) -> Result<()> {
        let mut key = HashMap::new();
        key.insert(s("ID"), AttributeValue { s: Some(s(self.id.clone())), ..Default::default() });

        let delete_item_input = DeleteItemInput {
            key: key,
            table_name: self.table_name.clone(),
            ..Default::default()
        };

        self.client.delete_item(&delete_item_input)?;
        
        Ok(())
    }

    pub fn read(&self) -> Result<Option<CounterRow>> {
        let mut key = HashMap::new();
        key.insert(s("ID"), AttributeValue { s: Some(s(self.id.clone())), ..Default::default() });

        let get_item_input = GetItemInput {
            consistent_read: Some(true),
            key: key,
            table_name: self.table_name.clone(),
            ..Default::default()
        };

        let item = self.client.get_item(&get_item_input)?;

        if let Some(item) = item.item {
            debug!("counter table={} id={} : {:?}", self.table_name, self.id, item);

            let maybe_typ: &AttributeValue = item.get("Type").ok_or(ErrorKind::MissingAttribute)?;
            let typ = maybe_typ.s.as_ref().ok_or(ErrorKind::MissingAttribute)?;

            if typ != COUNTER_TYPE {
                bail!(ErrorKind::UnrecognisedCounterType);
            }

            let id = item.get("ID").ok_or(ErrorKind::MissingAttribute)?;
            let version = item.get("Version").ok_or(ErrorKind::MissingAttribute)?;
            let value = item.get("Value").ok_or(ErrorKind::MissingAttribute)?;

            Ok(Some(CounterRow {
                id: id.s.as_ref().ok_or(ErrorKind::MissingAttribute)?.to_owned(),
                version: version.n.as_ref().ok_or(ErrorKind::MissingAttribute)?.parse()?,
                value: value.n.as_ref().ok_or(ErrorKind::MissingAttribute)?.parse()?,
            }))
        } else {
            debug!("empty counter table={} id={}", self.table_name, self.id);

            Ok(None)
        }
    }

    pub fn write(&self, row: CounterRow) -> Result<()> {
        let mut item = HashMap::new();
        item.insert(s("ID"), AttributeValue { s: Some(s(self.id.clone())), ..Default::default() });
        item.insert(s("Version"), AttributeValue { n: Some(format!("{}", row.version + 1)), ..Default::default() });
        item.insert(s("Type"), AttributeValue { s: Some(s(COUNTER_TYPE)), ..Default::default() });
        item.insert(s("Value"), AttributeValue { n: Some(format!("{}", row.value)), ..Default::default() });

        let mut expression_values = HashMap::new();
        expression_values.insert(s(":version"), AttributeValue { n: Some(format!("{}", row.version)), ..Default::default() });

        let get_item_input = PutItemInput {
            item: item,
            condition_expression: Some(s("Version = :version OR attribute_not_exists(Version)")),
            expression_attribute_values: Some(expression_values),
            table_name: self.table_name.clone(),
            ..Default::default()
        };

        match self.client.put_item(&get_item_input) {
            Err(PutItemError::Unknown(json)) => {
                let maybe_value = serde_json::from_str::<AWSError>(&json);
            
                if let Ok(value) = maybe_value {
                    if value.message.starts_with("The conditional request failed") {
                        bail!(ErrorKind::ConditionalUpdateFailed);
                    }
                }

                bail!(ErrorKind::PutItem(PutItemError::Unknown(json)));
            },
            Err(e) => {
                bail!(ErrorKind::PutItem(e));
            },
            Ok(_) => {
                Ok(())
            }
        }
    }
}

impl <P,D> MonotonicCounter for Counter<P,D> where P: ProvideAwsCredentials, D: DispatchSignedRequest {
    type Error = Error;

    fn get_value(&self) -> Result<u64> {
        let counter = self.read()?;
        
        Ok(counter.map(|c| c.value).unwrap_or(0))
    }

    fn next_value(&self) -> Result<u64> {
        loop {
            let maybe_counter = self.read()?;
            
            let mut counter = maybe_counter.unwrap_or_else(|| {
                debug!("no counter read. creating new..");
                CounterRow::new(self.id.clone())
            });

            counter.value += 1;
            let value = counter.value;

            match self.write(counter) {
                Err(Error(ErrorKind::ConditionalUpdateFailed, _)) => {
                    // try again
                    info!("transient error updating counter");
                    thread::sleep(self.retry_time.jitter(self.jitter_millis));
                },
                Err(e) => {
                    bail!(e);
                },
                Ok(()) => {
                    return Ok(value);
                }
            }
        }
    }
}
