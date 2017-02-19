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

pub const QUEUE_TYPE: &'static str = "QUEUE";

pub struct Queue<P,D> where P: ProvideAwsCredentials, D: DispatchSignedRequest {
    pub client: DynamoDbClient<P,D>,
    pub table_name: String,
    pub id: String,
    pub retry_time: Duration,
    pub jitter_millis: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct QueuePosition {
    process_id: String,
    counter: u64,
}

impl QueuePosition {
    pub fn new(process_id: String, counter: u64) -> QueuePosition {
        QueuePosition {
            process_id: process_id,
            counter: counter,
        }
    }

    pub fn from_vec(strs: &[String]) -> Result<Vec<QueuePosition>> {
        let mut result = vec![];

        for s in strs {
            result.push(Self::from_str(s)?)
        }

        result.sort_by(|a,b| a.counter.cmp(&b.counter));

        Ok(result)
    }

    pub fn to_string(&self) -> Result<String> {
        serde_json::to_string(self).map_err(Error::from)
    }

    pub fn to_string_vec(positions: &[QueuePosition]) -> Result<Vec<String>> {
        let mut result = vec![];

        for p in positions {
            let s = p.to_string()?;
            result.push(s);
        }

        Ok(result)
    }

    pub fn from_str(s: &str) -> Result<QueuePosition> {
        serde_json::from_str(s).map_err(Error::from)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueueRow {
    pub id: String,
    pub version: u64,
    pub value: u64,
    pub items: Vec<QueuePosition>,
}

impl QueueRow {
    pub fn new(id: String) -> QueueRow {
        QueueRow {
            id: id,
            version: 0,
            value: 0,
            items: vec![],
        }
    }
}

impl <P,D> Queue<P,D> where P: ProvideAwsCredentials, D: DispatchSignedRequest {
    pub fn new<S1, S2>(client: DynamoDbClient<P,D>, table_name: S1, id: S2, retry_time: Duration) -> Queue<P,D> where S1: Into<String>, S2: Into<String> {
        Queue {
            client: client,
            table_name: table_name.into(),
            id: id.into(),
            retry_time: retry_time,
            jitter_millis: 100,
        }
    }

    pub fn read(&self) -> Result<Option<QueueRow>> {
        let mut key = HashMap::new();
        key.insert(s("ID"), AttributeValue { s: Some(s(self.id.clone())), ..Default::default() });

        let get_item_input = GetItemInput {
            consistent_read: Some(false),
            key: key,
            table_name: self.table_name.clone(),
            ..Default::default()
        };

        let item = self.client.get_item(&get_item_input)?;

        if let Some(item) = item.item {
            debug!("counter table={} id={} : {:?}", self.table_name, self.id, item);

            let maybe_typ: &AttributeValue = item.get("Type").ok_or(ErrorKind::MissingAttribute)?;
            let typ = maybe_typ.s.as_ref().ok_or(ErrorKind::MissingAttribute)?;

            if typ != QUEUE_TYPE {
                bail!(ErrorKind::UnrecognisedQueueType);
            }

            let id = item.get("ID").ok_or(ErrorKind::MissingAttribute)?;
            let version = item.get("Version").ok_or(ErrorKind::MissingAttribute)?;
            let value = item.get("Value").ok_or(ErrorKind::MissingAttribute)?;
            let items = if let Some(items) = item.get("Items") {
                QueuePosition::from_vec(items.ss.as_ref().ok_or(ErrorKind::MissingAttribute)?)?
            } else {
                vec![]
            };

            Ok(Some(QueueRow {
                id: id.s.as_ref().ok_or(ErrorKind::MissingAttribute)?.to_owned(),
                version: version.n.as_ref().ok_or(ErrorKind::MissingAttribute)?.parse()?,
                value: value.n.as_ref().ok_or(ErrorKind::MissingAttribute)?.parse()?,
                items: items,
            }))
        } else {
            debug!("empty counter table={} id={}", self.table_name, self.id);

            Ok(None)
        }
    }

    pub fn write(&self, row: QueueRow) -> Result<u64> {
        let mut item = HashMap::new();
        item.insert(s("ID"), AttributeValue { s: Some(s(self.id.clone())), ..Default::default() });
        item.insert(s("Version"), AttributeValue { n: Some(format!("{}", row.version + 1)), ..Default::default() });
        item.insert(s("Type"), AttributeValue { s: Some(s(QUEUE_TYPE)), ..Default::default() });
        item.insert(s("Value"), AttributeValue { n: Some(format!("{}", row.value)), ..Default::default() });
        
        if row.items.len() != 0 {
            item.insert(s("Items"), AttributeValue { ss: Some(QueuePosition::to_string_vec(&row.items)?), ..Default::default() });
        }

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
                Ok(row.version + 1)
            }
        }
    }
}

impl <P,D> MonotonicQueue for Queue<P,D> where P: ProvideAwsCredentials, D: DispatchSignedRequest {
    type Error = Error;

    fn join_queue(&self, process_id: String) -> Result<(u64, Ticket)> {
        loop {
            let maybe_queue = self.read()?;

            let mut queue = maybe_queue.unwrap_or_else(|| {
                debug!("no queue read. creating new..");
                QueueRow::new(self.id.clone())
            });


            if let Some(ticket) = queue.items
                    .iter()
                    .enumerate()
                    .find(|&(_pos, t)| t.process_id == process_id)
                    .map(|(position,t)| Ticket::new(t.process_id.clone(), t.counter, position)) {

                return Ok((queue.version, ticket))
            }

            queue.value += 1;
            let position = queue.items.len();
            let counter = queue.value;
            let ticket = QueuePosition::new(process_id.clone(), counter);
            
            queue.items.push(ticket);

            match self.write(queue) {
                Err(Error(ErrorKind::ConditionalUpdateFailed, _)) => {
                    // try again
                    info!("transient error updating queue");
                    thread::sleep(self.retry_time.jitter(self.jitter_millis));
                },
                Err(e) => {
                    bail!(e);
                },
                Ok(version) => {
                    return Ok((version, Ticket::new(process_id, counter, position)));
                }
            }
        }
    }

    fn leave_queue(&self, process_id: &str) -> Result<u64> {
        loop {
            if let Some(mut queue) = self.read()? {              
                if let Some(pos) = queue.items.iter().position(|t| t.process_id == process_id) {
                    queue.items.remove(pos);
                } else {
                    bail!(ErrorKind::TicketNotFound(s(process_id)));
                }

                match self.write(queue) {
                    Err(Error(ErrorKind::ConditionalUpdateFailed, _)) => {
                        // try again
                        info!("transient error updating queue");
                        thread::sleep(self.retry_time.jitter(self.jitter_millis));
                    },
                    Err(e) => {
                        bail!(e);
                    },
                    Ok(version) => {
                        return Ok(version);
                    }
                }
            }
        }
    }

    fn get_ticket(&self, process_id: &str) -> Result<(u64, Ticket)> {
        if let Some(queue) = self.read()? {
            queue.items
                .iter()
                .enumerate()
                .find(|&(_pos, t)| t.process_id == process_id)
                .map(|(position,t)| (queue.version, Ticket::new(t.process_id.clone(), t.counter, position)))
                .ok_or_else(|| ErrorKind::TicketNotFound(process_id.to_owned()).into())

        } else {
            bail!(ErrorKind::TicketNotFound(process_id.to_owned()));
        }
    }

    fn get_tickets(&self) -> Result<(u64, Vec<Ticket>)> {
        if let Some(queue) = self.read()? {
            Ok((queue.version, queue.items
                .iter()
                .enumerate()
                .map(|(position,t)| Ticket::new(t.process_id.clone(), t.counter, position))
                .collect()))
        } else {
            Ok((0, vec![]))
        }
    }
}
