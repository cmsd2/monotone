//! Counter and queue rows and their pure operations.
//!
//! A row is the full stored state of one counter or queue. Every mutating
//! operation returns a new row whose `version` is exactly one greater than the
//! row it started from. A missing row behaves as [`Default`], which has
//! version 0.

use crate::{FencingToken, Tags, Ticket};

use super::error::Error;

/// Common behaviour of stored rows.
pub trait Row: Clone + Default {
    /// Value of the `Type` attribute identifying this structure.
    const TYPE: &'static str;

    /// The version this row state represents. 0 for a row never written.
    fn version(&self) -> u64;
}

/// Stored state of a monotonic counter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CounterRow {
    /// Write version, used for optimistic locking.
    pub version: u64,
    /// Current counter value.
    pub value: u64,
}

impl Row for CounterRow {
    const TYPE: &'static str = "COUNTER";

    fn version(&self) -> u64 {
        self.version
    }
}

impl CounterRow {
    /// Returns the row with the value and version each increased by one.
    pub fn increment(&self) -> CounterRow {
        CounterRow {
            version: self.version + 1,
            value: self.value + 1,
        }
    }
}

/// One process's entry in a queue row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueEntry {
    /// The process ID supplied on join.
    pub process_id: String,
    /// The counter issued on join.
    pub counter: u64,
    /// Tags supplied on join.
    pub tags: Tags,
}

/// Stored state of a monotonic queue.
///
/// `items` is always ordered by ascending `counter`, so an entry's index is its
/// position. `value` is the highest counter issued so far, and the fencing
/// token is `version`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueueRow {
    /// Write version and fencing token.
    pub version: u64,
    /// Highest counter issued so far. The next joiner receives `value + 1`.
    pub value: u64,
    /// Entries ordered by ascending counter.
    pub items: Vec<QueueEntry>,
}

impl Row for QueueRow {
    const TYPE: &'static str = "QUEUE";

    fn version(&self) -> u64 {
        self.version
    }
}

/// Outcome of [`QueueRow::join`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Join {
    /// The process was already queued. Nothing changes.
    Existing(FencingToken, Ticket),
    /// The process was appended. `row` must be written.
    Added {
        /// The new row to store.
        row: QueueRow,
        /// The ticket issued to the process.
        ticket: Ticket,
    },
}

impl QueueRow {
    fn ticket_at(&self, position: usize) -> Ticket {
        let entry = &self.items[position];
        Ticket {
            process_id: entry.process_id.clone(),
            counter: entry.counter,
            position,
            tags: entry.tags.clone(),
        }
    }

    fn position_of(&self, process_id: &str) -> Option<usize> {
        self.items.iter().position(|e| e.process_id == process_id)
    }

    /// Appends the process with the next counter, or reports its existing ticket.
    pub fn join(&self, process_id: &str, tags: Tags) -> Join {
        if let Some(position) = self.position_of(process_id) {
            return Join::Existing(self.version, self.ticket_at(position));
        }

        let mut row = self.clone();
        row.version += 1;
        row.value += 1;
        row.items.push(QueueEntry {
            process_id: process_id.to_owned(),
            counter: row.value,
            tags,
        });
        let ticket = row.ticket_at(row.items.len() - 1);
        Join::Added { row, ticket }
    }

    /// Removes the process. Later entries move forward by one position.
    pub fn leave(&self, process_id: &str) -> Result<QueueRow, Error> {
        let position = self
            .position_of(process_id)
            .ok_or_else(|| Error::NotFound(process_id.to_owned()))?;

        let mut row = self.clone();
        row.version += 1;
        row.items.remove(position);
        Ok(row)
    }

    /// Returns the current fencing token and the process's ticket.
    pub fn ticket(&self, process_id: &str) -> Result<(FencingToken, Ticket), Error> {
        self.position_of(process_id)
            .map(|position| (self.version, self.ticket_at(position)))
            .ok_or_else(|| Error::NotFound(process_id.to_owned()))
    }

    /// Returns the current fencing token and every ticket in position order.
    pub fn tickets(&self) -> (FencingToken, Vec<Ticket>) {
        let tickets = (0..self.items.len()).map(|p| self.ticket_at(p)).collect();
        (self.version, tickets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(pairs: &[(&str, &str)]) -> Tags {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn added(join: Join) -> (QueueRow, Ticket) {
        match join {
            Join::Added { row, ticket } => (row, ticket),
            other => panic!("expected Added, got {other:?}"),
        }
    }

    #[test]
    fn counter_increment_bumps_value_and_version() {
        let row = CounterRow::default().increment();
        assert_eq!(
            row,
            CounterRow {
                version: 1,
                value: 1
            }
        );
        assert_eq!(
            row.increment(),
            CounterRow {
                version: 2,
                value: 2
            }
        );
    }

    #[test]
    fn first_and_second_joiner_get_counters_one_and_two() {
        let (row, foo) = added(QueueRow::default().join("foo", Tags::new()));
        assert_eq!((foo.counter, foo.position, row.version), (1, 0, 1));

        let (row, bar) = added(row.join("bar", Tags::new()));
        assert_eq!((bar.counter, bar.position, row.version), (2, 1, 2));
    }

    #[test]
    fn join_records_tags() {
        let t = tags(&[("role", "leader")]);
        let (row, ticket) = added(QueueRow::default().join("foo", t.clone()));
        assert_eq!(ticket.tags, t);
        assert_eq!(row.ticket("foo").unwrap().1.tags, t);
        assert_eq!(row.tickets().1[0].tags, t);
    }

    #[test]
    fn rejoin_returns_existing_ticket_without_a_write() {
        let (row, first) = added(QueueRow::default().join("foo", Tags::new()));
        match row.join("foo", tags(&[("ignored", "yes")])) {
            Join::Existing(token, ticket) => {
                assert_eq!(token, 1);
                assert_eq!(ticket, first);
            }
            other => panic!("expected Existing, got {other:?}"),
        }
    }

    #[test]
    fn positions_are_dense_after_leave_and_counters_unchanged() {
        let (row, _) = added(QueueRow::default().join("a", Tags::new()));
        let (row, _) = added(row.join("b", Tags::new()));
        let (row, c) = added(row.join("c", Tags::new()));
        let row = row.leave("b").unwrap();

        let (token, tickets) = row.tickets();
        assert_eq!(token, 4);
        let summary: Vec<_> = tickets
            .iter()
            .map(|t| (t.process_id.as_str(), t.position, t.counter))
            .collect();
        assert_eq!(summary, vec![("a", 0, 1), ("c", 1, c.counter)]);
    }

    #[test]
    fn counter_is_not_reused_after_leave() {
        let (row, _) = added(QueueRow::default().join("a", Tags::new()));
        let row = row.leave("a").unwrap();
        let (_, again) = added(row.join("a", Tags::new()));
        assert_eq!(again.counter, 2);
    }

    #[test]
    fn version_bumps_exactly_once_per_write() {
        let mut versions = vec![0];
        let (row, _) = added(QueueRow::default().join("a", Tags::new()));
        versions.push(row.version);
        let row = match row.join("a", Tags::new()) {
            Join::Existing(..) => row,
            Join::Added { .. } => panic!("rejoin must not add"),
        };
        versions.push(row.version);
        let row = row.leave("a").unwrap();
        versions.push(row.version);
        assert_eq!(row.leave("a"), Err(Error::NotFound("a".into())));
        versions.push(row.version);
        assert_eq!(versions, vec![0, 1, 1, 2, 2]);
    }

    #[test]
    fn lookups_on_unknown_process_are_not_found() {
        let row = QueueRow::default();
        assert_eq!(row.ticket("foo"), Err(Error::NotFound("foo".into())));
        assert_eq!(row.leave("foo"), Err(Error::NotFound("foo".into())));
    }

    #[test]
    fn empty_queue_lists_token_zero_and_no_tickets() {
        assert_eq!(QueueRow::default().tickets(), (0, vec![]));
    }
}
