//! Monotonic counters and fenced queues for coordination in distributed systems.
//!
//! The crate is built around a sans-IO [`core`]: every rule about counters and
//! queues (row arithmetic, ticket ordering, fencing tokens, conflict retry) is
//! pure code that emits [`core::machine::Effect`]s and consumes
//! [`core::machine::Input`]s. Backends execute those effects:
//!
//! - [`memory`] runs the core against process-local storage.
//! - `dynamodb` (feature `dynamodb`) runs it against an AWS DynamoDB table using
//!   conditional writes for optimistic locking.
//!
//! Both backends implement [`MonotonicCounter`] and [`MonotonicQueue`], and both
//! observe identical results for identical operation sequences.

use std::collections::BTreeMap;

pub mod core;
#[cfg(feature = "dynamodb")]
pub mod dynamodb;
pub mod memory;

pub use crate::core::error::Error;

/// Compiles and runs the Rust examples in README.md.
#[cfg(all(doctest, feature = "dynamodb"))]
#[doc = include_str!("../../README.md")]
pub struct ReadmeDoctests;

/// A token that increases by exactly one with every write to a queue.
///
/// Compare it in downstream conditional updates to avoid acting on a stale
/// view of the queue.
pub type FencingToken = u64;

/// Free-form string tags attached to a queue entry.
pub type Tags = BTreeMap<String, String>;

/// A process's place in a queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    /// The process ID supplied on join.
    pub process_id: String,
    /// The counter value issued on join. Never reused within the queue.
    pub counter: u64,
    /// Zero-based position in the queue, ordered by ascending counter.
    pub position: usize,
    /// Tags supplied on join.
    pub tags: Tags,
}

/// A counter that only ever increases.
///
/// `async fn` in this public trait cannot promise `Send` futures to generic
/// callers. Both backends in this crate return `Send` futures; callers that
/// need the bound should name the concrete backend type.
#[allow(async_fn_in_trait)]
pub trait MonotonicCounter {
    /// Backend-specific error type.
    type Error;

    /// Returns the current value, 0 for a counter that has never been incremented.
    async fn get_value(&self) -> Result<u64, Self::Error>;

    /// Atomically increments the counter by one and returns the new value.
    async fn next_value(&self) -> Result<u64, Self::Error>;
}

/// An ordered list of process IDs with monotonic counters and a fencing token.
///
/// See [`MonotonicCounter`] for a note on `async fn` in public traits.
#[allow(async_fn_in_trait)]
pub trait MonotonicQueue {
    /// Backend-specific error type.
    type Error;

    /// Appends the process to the back of the queue, or returns its existing
    /// ticket unchanged if it is already queued.
    async fn join_queue<T>(
        &self,
        process_id: &str,
        tags: T,
    ) -> Result<(FencingToken, Ticket), Self::Error>
    where
        T: Into<Option<Tags>>;

    /// Removes the process from the queue and returns the new fencing token.
    async fn leave_queue(&self, process_id: &str) -> Result<FencingToken, Self::Error>;

    /// Returns the ticket for one process.
    async fn get_ticket(&self, process_id: &str) -> Result<(FencingToken, Ticket), Self::Error>;

    /// Returns every ticket, ordered by ascending counter.
    async fn get_tickets(&self) -> Result<(FencingToken, Vec<Ticket>), Self::Error>;
}
