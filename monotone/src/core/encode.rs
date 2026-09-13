//! Encoding rows to and from a backend-neutral item.
//!
//! The item shape matches the DynamoDB schema used since 0.4:
//!
//! | Attribute | Kind | Present on |
//! |-----------|------|------------|
//! | `ID`      | S    | all items |
//! | `Type`    | S    | all items, `COUNTER` or `QUEUE` |
//! | `Version` | N    | all items |
//! | `Value`   | N    | all items |
//! | `Items`   | SS   | non-empty queues; each member is a JSON entry |
//!
//! Numbers are kept as their decimal string form, as DynamoDB transmits them,
//! so decoding can reject values that are not unsigned integers.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::error::Error;
use super::row::{CounterRow, QueueEntry, QueueRow, Row};
use crate::Tags;

/// Attribute name of the partition key.
pub const ID: &str = "ID";
/// Attribute name of the structure discriminator.
pub const TYPE: &str = "Type";
/// Attribute name of the optimistic-locking version.
pub const VERSION: &str = "Version";
/// Attribute name of the counter value.
pub const VALUE: &str = "Value";
/// Attribute name of the queue entries.
pub const ITEMS: &str = "Items";

/// A single attribute value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attr {
    /// A string.
    S(String),
    /// A number in decimal string form.
    N(String),
    /// A set of strings.
    SS(BTreeSet<String>),
    /// Any other attribute kind, named for error reporting.
    Other(String),
}

/// A stored item: attribute name to value.
pub type Item = BTreeMap<String, Attr>;

/// Rows that can be converted to and from an [`Item`].
pub trait Codec: Row + Sized {
    /// Encodes the row as the item stored under `id`.
    fn encode(&self, id: &str) -> Item;

    /// Decodes a row, checking its `Type` and required attributes.
    fn decode(item: &Item) -> Result<Self, Error>;
}

/// The JSON form of one queue entry inside the `Items` string set.
///
/// `tags` is optional on the wire: 0.4 items can hold `null`, and entries
/// written before tags existed omit it.
#[derive(Serialize, Deserialize)]
struct WireEntry {
    process_id: String,
    counter: u64,
    #[serde(default)]
    tags: Option<Tags>,
}

fn kind(attr: &Attr) -> &'static str {
    match attr {
        Attr::S(_) => "S",
        Attr::N(_) => "N",
        Attr::SS(_) => "SS",
        Attr::Other(_) => "other",
    }
}

fn get<'a>(item: &'a Item, name: &'static str) -> Result<&'a Attr, Error> {
    item.get(name).ok_or(Error::MissingAttribute(name))
}

fn get_s<'a>(item: &'a Item, name: &'static str) -> Result<&'a str, Error> {
    match get(item, name)? {
        Attr::S(s) => Ok(s),
        other => Err(Error::InvalidAttribute {
            name,
            reason: format!("expected S, found {}", kind(other)),
        }),
    }
}

fn get_n(item: &Item, name: &'static str) -> Result<u64, Error> {
    match get(item, name)? {
        Attr::N(n) => n.parse().map_err(|_| Error::InvalidAttribute {
            name,
            reason: format!("not an unsigned integer: {n}"),
        }),
        other => Err(Error::InvalidAttribute {
            name,
            reason: format!("expected N, found {}", kind(other)),
        }),
    }
}

fn check_type(item: &Item, expected: &'static str) -> Result<(), Error> {
    get_s(item, ID)?;
    let found = get_s(item, TYPE)?;
    if found != expected {
        return Err(Error::WrongType {
            expected,
            found: found.to_owned(),
        });
    }
    Ok(())
}

fn header(id: &str, typ: &str, version: u64, value: u64) -> Item {
    BTreeMap::from([
        (ID.to_owned(), Attr::S(id.to_owned())),
        (TYPE.to_owned(), Attr::S(typ.to_owned())),
        (VERSION.to_owned(), Attr::N(version.to_string())),
        (VALUE.to_owned(), Attr::N(value.to_string())),
    ])
}

impl Codec for CounterRow {
    fn encode(&self, id: &str) -> Item {
        header(id, Self::TYPE, self.version, self.value)
    }

    fn decode(item: &Item) -> Result<Self, Error> {
        check_type(item, Self::TYPE)?;
        Ok(CounterRow {
            version: get_n(item, VERSION)?,
            value: get_n(item, VALUE)?,
        })
    }
}

impl Codec for QueueRow {
    fn encode(&self, id: &str) -> Item {
        let mut item = header(id, Self::TYPE, self.version, self.value);
        if !self.items.is_empty() {
            let entries = self
                .items
                .iter()
                .map(|e| {
                    let wire = WireEntry {
                        process_id: e.process_id.clone(),
                        counter: e.counter,
                        tags: Some(e.tags.clone()),
                    };
                    serde_json::to_string(&wire).expect("queue entry serialises to JSON")
                })
                .collect();
            item.insert(ITEMS.to_owned(), Attr::SS(entries));
        }
        item
    }

    fn decode(item: &Item) -> Result<Self, Error> {
        check_type(item, Self::TYPE)?;
        let version = get_n(item, VERSION)?;
        let value = get_n(item, VALUE)?;

        let mut items = match item.get(ITEMS) {
            None => Vec::new(),
            Some(Attr::SS(set)) => set
                .iter()
                .map(|s| {
                    serde_json::from_str::<WireEntry>(s)
                        .map(|w| QueueEntry {
                            process_id: w.process_id,
                            counter: w.counter,
                            tags: w.tags.unwrap_or_default(),
                        })
                        .map_err(|e| Error::InvalidAttribute {
                            name: ITEMS,
                            reason: format!("bad entry {s}: {e}"),
                        })
                })
                .collect::<Result<Vec<_>, _>>()?,
            Some(other) => {
                return Err(Error::InvalidAttribute {
                    name: ITEMS,
                    reason: format!("expected SS, found {}", kind(other)),
                });
            }
        };
        items.sort_by_key(|e| e.counter);

        Ok(QueueRow {
            version,
            value,
            items,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Attr {
        Attr::S(v.to_owned())
    }

    fn n(v: &str) -> Attr {
        Attr::N(v.to_owned())
    }

    fn item(pairs: Vec<(&str, Attr)>) -> Item {
        pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect()
    }

    fn ss(members: &[&str]) -> Attr {
        Attr::SS(members.iter().map(|m| m.to_string()).collect())
    }

    #[test]
    fn counter_item_fixture() {
        let row = CounterRow {
            version: 3,
            value: 3,
        };
        let expected = item(vec![
            ("ID", s("c1")),
            ("Type", s("COUNTER")),
            ("Version", n("3")),
            ("Value", n("3")),
        ]);
        assert_eq!(row.encode("c1"), expected);
        assert_eq!(CounterRow::decode(&expected), Ok(row));
    }

    #[test]
    fn queue_item_fixture() {
        let row = QueueRow {
            version: 1,
            value: 1,
            items: vec![QueueEntry {
                process_id: "foo".into(),
                counter: 1,
                tags: Tags::new(),
            }],
        };
        let expected = item(vec![
            ("ID", s("q1")),
            ("Type", s("QUEUE")),
            ("Version", n("1")),
            ("Value", n("1")),
            (
                "Items",
                ss(&[r#"{"process_id":"foo","counter":1,"tags":{}}"#]),
            ),
        ]);
        assert_eq!(row.encode("q1"), expected);
        assert_eq!(QueueRow::decode(&expected), Ok(row));
    }

    #[test]
    fn empty_queue_omits_items() {
        let row = QueueRow {
            version: 2,
            value: 1,
            items: vec![],
        };
        assert!(!row.encode("q1").contains_key("Items"));
    }

    #[test]
    fn missing_items_decodes_as_empty() {
        let it = item(vec![
            ("ID", s("q1")),
            ("Type", s("QUEUE")),
            ("Version", n("2")),
            ("Value", n("1")),
        ]);
        assert_eq!(QueueRow::decode(&it).unwrap().items, vec![]);
    }

    #[test]
    fn legacy_null_and_absent_tags_decode_as_empty() {
        let it = item(vec![
            ("ID", s("q1")),
            ("Type", s("QUEUE")),
            ("Version", n("2")),
            ("Value", n("2")),
            (
                "Items",
                ss(&[
                    r#"{"process_id":"a","counter":1,"tags":null}"#,
                    r#"{"process_id":"b","counter":2}"#,
                ]),
            ),
        ]);
        let row = QueueRow::decode(&it).unwrap();
        assert!(row.items.iter().all(|e| e.tags.is_empty()));
    }

    #[test]
    fn items_are_sorted_by_counter_on_decode() {
        // BTreeSet orders members lexically, so counter 10 sorts before 9.
        let it = item(vec![
            ("ID", s("q1")),
            ("Type", s("QUEUE")),
            ("Version", n("3")),
            ("Value", n("10")),
            (
                "Items",
                ss(&[
                    r#"{"process_id":"late","counter":10,"tags":{}}"#,
                    r#"{"process_id":"early","counter":9,"tags":{}}"#,
                ]),
            ),
        ]);
        let order: Vec<_> = QueueRow::decode(&it)
            .unwrap()
            .items
            .into_iter()
            .map(|e| e.counter)
            .collect();
        assert_eq!(order, vec![9, 10]);
    }

    #[test]
    fn wrong_type_is_rejected_both_ways() {
        let queue = QueueRow::default().encode("x");
        assert_eq!(
            CounterRow::decode(&queue),
            Err(Error::WrongType {
                expected: "COUNTER",
                found: "QUEUE".into()
            })
        );
        let counter = CounterRow::default().encode("x");
        assert_eq!(
            QueueRow::decode(&counter),
            Err(Error::WrongType {
                expected: "QUEUE",
                found: "COUNTER".into()
            })
        );
    }

    #[test]
    fn each_missing_required_attribute_is_named() {
        for name in [ID, TYPE, VERSION, VALUE] {
            let mut it = CounterRow::default().encode("x");
            it.remove(name);
            assert_eq!(
                CounterRow::decode(&it),
                Err(Error::MissingAttribute(name)),
                "{name}"
            );
        }
    }

    #[test]
    fn non_integer_numbers_are_rejected() {
        for bad in ["-1", "1.5", "abc", ""] {
            let mut it = CounterRow::default().encode("x");
            it.insert(VALUE.into(), n(bad));
            match CounterRow::decode(&it) {
                Err(Error::InvalidAttribute { name: "Value", .. }) => {}
                other => panic!("{bad:?}: {other:?}"),
            }
        }
    }

    #[test]
    fn wrong_attribute_kinds_are_rejected() {
        let mut it = CounterRow::default().encode("x");
        it.insert(VERSION.into(), s("1"));
        assert!(matches!(
            CounterRow::decode(&it),
            Err(Error::InvalidAttribute {
                name: "Version",
                ..
            })
        ));

        let mut it = QueueRow::default().encode("x");
        it.insert(ITEMS.into(), s("[]"));
        assert!(matches!(
            QueueRow::decode(&it),
            Err(Error::InvalidAttribute { name: "Items", .. })
        ));

        let mut it = QueueRow::default().encode("x");
        it.insert(ITEMS.into(), ss(&["not json"]));
        assert!(matches!(
            QueueRow::decode(&it),
            Err(Error::InvalidAttribute { name: "Items", .. })
        ));

        let mut it = CounterRow::default().encode("x");
        it.insert(TYPE.into(), Attr::Other("BOOL".into()));
        assert!(matches!(
            CounterRow::decode(&it),
            Err(Error::InvalidAttribute { name: "Type", .. })
        ));
    }
}
