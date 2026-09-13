//! DynamoDB backend (feature `dynamodb`).
//!
//! [`Counter`] and [`Queue`] drive the shared [`core`](crate::core) machines
//! against one DynamoDB table keyed by a string `ID`. Reads are strongly
//! consistent; writes are conditional on the version read, and conflicts are
//! retried after the configured delay plus jitter.
//!
//! [`table`] creates and waits for the table. [`load_config`] builds an SDK
//! configuration from the environment, including `AWS_ENDPOINT_URL` for
//! DynamoDB Local.
//!
//! ```no_run
//! use monotone::MonotonicCounter;
//! use monotone::dynamodb::{self, Counter};
//!
//! # async fn demo() -> Result<(), monotone::dynamodb::Error> {
//! let config = dynamodb::load_config(Some("eu-west-1".into())).await;
//! let client = aws_sdk_dynamodb::Client::new(&config);
//! dynamodb::table::create_table_if_needed(&client, "Counters", 1, 1).await?;
//!
//! let counter = Counter::new(client, "Counters", "builds");
//! let next = counter.next_value().await?;
//! # Ok(())
//! # }
//! ```

mod adapter;
mod error;
pub mod table;

use aws_config::{BehaviorVersion, Region, SdkConfig};
use aws_sdk_dynamodb::Client;

pub use adapter::{attr_from_sdk, attr_to_sdk, put_if_version, read_item};
pub use error::Error;

use crate::core::machine::RetryPolicy;
use crate::core::ops;
use crate::{FencingToken, MonotonicCounter, MonotonicQueue, Tags, Ticket};
use adapter::{DynamoExecutor, drive, rng};

/// Loads SDK configuration from the standard AWS environment and profile chain.
///
/// `region` overrides the environment's region when given. `AWS_ENDPOINT_URL`
/// redirects every call, which is how tests reach DynamoDB Local.
pub async fn load_config(region: Option<String>) -> SdkConfig {
    let mut loader = aws_config::defaults(BehaviorVersion::latest());
    if let Some(region) = region {
        loader = loader.region(Region::new(region));
    }
    loader.load().await
}

/// A monotonic counter stored as one DynamoDB item.
#[derive(Debug, Clone)]
pub struct Counter {
    client: Client,
    table: String,
    id: String,
    policy: RetryPolicy,
}

impl Counter {
    /// Returns a handle to the counter `id` in `table`.
    pub fn new(client: Client, table: impl Into<String>, id: impl Into<String>) -> Counter {
        Counter {
            client,
            table: table.into(),
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

    /// The table holding the counter.
    pub fn table(&self) -> &str {
        &self.table
    }

    /// Deletes the counter's item. Succeeds whether or not it exists.
    pub async fn remove(&self) -> Result<(), Error> {
        adapter::delete_item(&self.client, &self.table, &self.id).await
    }

    fn executor(&self) -> DynamoExecutor<'_> {
        DynamoExecutor::new(&self.client, &self.table, &self.id)
    }
}

impl MonotonicCounter for Counter {
    type Error = Error;

    async fn get_value(&self) -> Result<u64, Error> {
        drive(&self.executor(), ops::get_value()).await
    }

    async fn next_value(&self) -> Result<u64, Error> {
        drive(&self.executor(), ops::next_value(self.policy, rng())).await
    }
}

/// A monotonic queue stored as one DynamoDB item.
#[derive(Debug, Clone)]
pub struct Queue {
    client: Client,
    table: String,
    id: String,
    policy: RetryPolicy,
}

impl Queue {
    /// Returns a handle to the queue `id` in `table`.
    pub fn new(client: Client, table: impl Into<String>, id: impl Into<String>) -> Queue {
        Queue {
            client,
            table: table.into(),
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

    /// The table holding the queue.
    pub fn table(&self) -> &str {
        &self.table
    }

    /// Deletes the queue's item and all its entries. Succeeds whether or not it exists.
    pub async fn remove(&self) -> Result<(), Error> {
        adapter::delete_item(&self.client, &self.table, &self.id).await
    }

    fn executor(&self) -> DynamoExecutor<'_> {
        DynamoExecutor::new(&self.client, &self.table, &self.id)
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
            &self.executor(),
            ops::join_queue(process_id, tags, self.policy, rng()),
        )
        .await
    }

    async fn leave_queue(&self, process_id: &str) -> Result<FencingToken, Error> {
        drive(
            &self.executor(),
            ops::leave_queue(process_id, self.policy, rng()),
        )
        .await
    }

    async fn get_ticket(&self, process_id: &str) -> Result<(FencingToken, Ticket), Error> {
        drive(&self.executor(), ops::get_ticket(process_id)).await
    }

    async fn get_tickets(&self) -> Result<(FencingToken, Vec<Ticket>), Error> {
        drive(&self.executor(), ops::get_tickets()).await
    }
}
