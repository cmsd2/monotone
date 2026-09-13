//! In-memory backend.
//!
//! A [`Store`] holds items for any number of counters and queues, keyed by ID,
//! exactly as a DynamoDB table would. [`Counter`] and [`Queue`] drive the shared
//! [`core`](crate::core) machines against it: reads and conditional writes each
//! take the store's lock briefly, and `Sleep` effects are skipped. Concurrent
//! tasks therefore see the same conflict-and-retry semantics as separate
//! processes on DynamoDB.
//!
//! ```
//! use monotone::memory::Store;
//! use monotone::{MonotonicCounter, MonotonicQueue};
//!
//! # async fn demo() -> Result<(), monotone::Error> {
//! let store = Store::new();
//! let counter = store.counter("builds");
//! assert_eq!(counter.next_value().await?, 1);
//!
//! let queue = store.queue("zookeeper");
//! let (token, ticket) = queue.join_queue("host-a", None).await?;
//! assert_eq!((token, ticket.counter, ticket.position), (1, 1, 0));
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use rand::rngs::SmallRng;

use crate::core::encode::{Codec, Item};
use crate::core::error::Error;
use crate::core::machine::{Effect, Input, Machine, RetryPolicy, Step};
use crate::core::ops;
use crate::{FencingToken, MonotonicCounter, MonotonicQueue, Tags, Ticket};

/// Shared process-local storage for counters and queues.
///
/// Clones share the same underlying map.
#[derive(Debug, Clone, Default)]
pub struct Store {
    items: Arc<Mutex<HashMap<String, (u64, Item)>>>,
}

impl Store {
    /// Creates an empty store.
    pub fn new() -> Store {
        Store::default()
    }

    /// Returns a handle to the counter stored under `id`.
    pub fn counter(&self, id: impl Into<String>) -> Counter {
        Counter::new(self, id)
    }

    /// Returns a handle to the queue stored under `id`.
    pub fn queue(&self, id: impl Into<String>) -> Queue {
        Queue::new(self, id)
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, (u64, Item)>> {
        // A panic while holding the lock cannot leave a half-written item, since
        // every mutation is a single insert or remove.
        self.items
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn read(&self, id: &str) -> Option<Item> {
        self.lock().get(id).map(|(_, item)| item.clone())
    }

    /// Stores `item` at `version` if the current version is `expected_version`
    /// (or no item exists and `expected_version` is 0). Returns whether it did.
    pub(crate) fn put_if_version(
        &self,
        id: &str,
        item: Item,
        version: u64,
        expected_version: u64,
    ) -> bool {
        let mut items = self.lock();
        let current = items.get(id).map_or(0, |(v, _)| *v);
        if current != expected_version {
            return false;
        }
        items.insert(id.to_owned(), (version, item));
        true
    }

    pub(crate) fn remove(&self, id: &str) {
        self.lock().remove(id);
    }
}

/// Runs a machine to completion against the store.
pub(crate) fn drive<M>(store: &Store, id: &str, mut machine: M) -> Result<M::Output, Error>
where
    M: Machine,
    M::Row: Codec,
{
    let mut step = machine.start()?;
    loop {
        step = match step {
            Step::Done(output) => return Ok(output),
            Step::Effect(Effect::Read) => {
                let row = store
                    .read(id)
                    .map(|item| M::Row::decode(&item))
                    .transpose()?;
                machine.step(Input::Row(row))?
            }
            Step::Effect(Effect::Write {
                row,
                expected_version,
            }) => {
                let version = crate::core::row::Row::version(&row);
                if store.put_if_version(id, row.encode(id), version, expected_version) {
                    machine.step(Input::WriteOk)?
                } else {
                    machine.step(Input::WriteConflict)?
                }
            }
            // Nothing to back off from in memory.
            Step::Effect(Effect::Sleep(_)) => machine.resume()?,
        };
    }
}

fn rng() -> SmallRng {
    rand::make_rng()
}

/// An in-memory monotonic counter.
#[derive(Debug, Clone)]
pub struct Counter {
    store: Store,
    id: String,
    policy: RetryPolicy,
}

impl Counter {
    /// Returns a handle to the counter stored under `id` in `store`.
    pub fn new(store: &Store, id: impl Into<String>) -> Counter {
        Counter {
            store: store.clone(),
            id: id.into(),
            policy: RetryPolicy::default(),
        }
    }

    /// Replaces the retry policy used after write conflicts.
    pub fn with_retry_policy(mut self, policy: RetryPolicy) -> Counter {
        self.policy = policy;
        self
    }

    /// The counter's ID.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Deletes the counter. Succeeds whether or not it exists.
    pub async fn remove(&self) -> Result<(), Error> {
        self.store.remove(&self.id);
        Ok(())
    }
}

impl MonotonicCounter for Counter {
    type Error = Error;

    async fn get_value(&self) -> Result<u64, Error> {
        drive(&self.store, &self.id, ops::get_value())
    }

    async fn next_value(&self) -> Result<u64, Error> {
        drive(&self.store, &self.id, ops::next_value(self.policy, rng()))
    }
}

/// An in-memory monotonic queue.
#[derive(Debug, Clone)]
pub struct Queue {
    store: Store,
    id: String,
    policy: RetryPolicy,
}

impl Queue {
    /// Returns a handle to the queue stored under `id` in `store`.
    pub fn new(store: &Store, id: impl Into<String>) -> Queue {
        Queue {
            store: store.clone(),
            id: id.into(),
            policy: RetryPolicy::default(),
        }
    }

    /// Replaces the retry policy used after write conflicts.
    pub fn with_retry_policy(mut self, policy: RetryPolicy) -> Queue {
        self.policy = policy;
        self
    }

    /// The queue's ID.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Deletes the queue and all its entries. Succeeds whether or not it exists.
    pub async fn remove(&self) -> Result<(), Error> {
        self.store.remove(&self.id);
        Ok(())
    }
}

impl MonotonicQueue for Queue {
    type Error = Error;

    async fn join_queue<T>(
        &self,
        process_id: &str,
        tags: T,
    ) -> Result<(FencingToken, Ticket), Error>
    where
        T: Into<Option<Tags>>,
    {
        let tags = tags.into().unwrap_or_default();
        drive(
            &self.store,
            &self.id,
            ops::join_queue(process_id, tags, self.policy, rng()),
        )
    }

    async fn leave_queue(&self, process_id: &str) -> Result<FencingToken, Error> {
        drive(
            &self.store,
            &self.id,
            ops::leave_queue(process_id, self.policy, rng()),
        )
    }

    async fn get_ticket(&self, process_id: &str) -> Result<(FencingToken, Ticket), Error> {
        drive(&self.store, &self.id, ops::get_ticket(process_id))
    }

    async fn get_tickets(&self) -> Result<(FencingToken, Vec<Ticket>), Error> {
        drive(&self.store, &self.id, ops::get_tickets())
    }
}

#[cfg(test)]
mod tests;
