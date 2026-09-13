# monotone

[![CI](https://github.com/cmsd2/monotone/actions/workflows/ci.yml/badge.svg)](https://github.com/cmsd2/monotone/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/monotone.svg)](https://crates.io/crates/monotone)
[![docs.rs](https://docs.rs/monotone/badge.svg)](https://docs.rs/monotone)

Monotonic counters and fenced queues for coordination in distributed systems.

A **counter** is a named `u64` that only goes up. A **queue** is an ordered list
of process IDs. Each process that joins gets a counter value that is never
reused, and every write bumps a **fencing token** so callers can detect a stale
view. They are built for configuration management: assigning member IDs,
picking a leader, versioning. They are not performance counters.

The repository holds two crates:

- [`monotone`](monotone) is the library, with an in-memory backend and a
  DynamoDB backend that uses conditional writes for optimistic locking.
- [`monotone-cli`](cli) builds the `monotone` binary, a JSON-printing front end
  over the DynamoDB backend.

All counter and queue rules live in a sans-IO core that performs no I/O. Each
backend executes the core's read, conditional-write and sleep effects, so both
backends behave identically and the retry logic is tested without a database.

## Library

```toml
[dependencies]
monotone = "0.5"

# With the DynamoDB backend:
monotone = { version = "0.5", features = ["dynamodb"] }
```

The minimum supported Rust version is 1.88. The default build has no network,
TLS or async runtime dependencies.

### In memory

```rust
use monotone::memory::Store;
use monotone::{MonotonicCounter, MonotonicQueue};

#[tokio::main]
async fn main() -> Result<(), monotone::Error> {
    let store = Store::new();

    let builds = store.counter("builds");
    assert_eq!(builds.next_value().await?, 1);
    assert_eq!(builds.next_value().await?, 2);

    let cluster = store.queue("zookeeper");
    let (token, ticket) = cluster.join_queue("host-a", None).await?;
    assert_eq!((token, ticket.counter, ticket.position), (1, 1, 0));
    Ok(())
}
```

### On DynamoDB

```rust,no_run
use monotone::dynamodb::{self, table, Queue};
use monotone::MonotonicQueue;

#[tokio::main]
async fn main() -> Result<(), dynamodb::Error> {
    let config = dynamodb::load_config(Some("eu-west-1".into())).await;
    let client = aws_sdk_dynamodb::Client::new(&config);
    table::create_table_if_needed(&client, "Counters", 1, 1).await?;
    table::wait_for_table(&client, "Counters").await?;

    let queue = Queue::new(client, "Counters", "myzkcluster");
    let (fencing_token, ticket) = queue.join_queue("host-a", None).await?;
    println!("server id {} (token {fencing_token})", ticket.counter);
    Ok(())
}
```

Credentials, region and endpoint come from the standard AWS configuration
chain. `load_config` overrides the region when you pass one.

## CLI

Install from crates.io, or download a binary from the
[releases page](https://github.com/cmsd2/monotone/releases):

```sh
cargo install monotone-cli
```

Every invocation acts on one counter or queue row and prints JSON to stdout.

| Option | Default | Meaning |
|---|---|---|
| `-i, --id <ID>` | required | Counter or queue ID |
| `-t, --table <TABLE>` | `Counters` | DynamoDB table; created on first use |
| `-r, --region <REGION>` | `eu-west-1` | AWS region |
| `-p, --process <PROCESS_ID>` | | Process ID, for `queue get`, `join` and `leave` |

Credentials come from the standard AWS chain. Set `AWS_ENDPOINT_URL` to target
DynamoDB Local. Set `RUST_LOG=debug` for diagnostics on stderr. Failures print
`error: ...` to stderr and exit with status 1.

A counter command on a queue's ID, or the reverse, fails rather than corrupting
the row.

### Counter

```sh
monotone -i mycounter counter get
```

```json
{
  "id": "mycounter",
  "value": 0,
  "region": "eu-west-1",
  "table": "Counters"
}
```

`counter next` increments and prints the new value. `counter rm` deletes the row
and prints nothing.

### Queue

```sh
monotone -i myqueue queue -p foo join
monotone -i myqueue queue -p bar join --tag role=zk --tag rack=a
```

```json
{
  "id": "myqueue",
  "region": "eu-west-1",
  "table": "Counters",
  "fencing_token": 2,
  "ticket": {
    "process_id": "bar",
    "counter": 2,
    "position": 1,
    "tags": {
      "rack": "a",
      "role": "zk"
    }
  }
}
```

Joining again with the same process ID returns the existing ticket and does not
bump the token. `queue list` prints every ticket in position order:

```json
{
  "id": "myqueue",
  "region": "eu-west-1",
  "table": "Counters",
  "fencing_token": 2,
  "tickets": [
    {
      "process_id": "foo",
      "counter": 1,
      "position": 0,
      "tags": {}
    },
    {
      "process_id": "bar",
      "counter": 2,
      "position": 1,
      "tags": {
        "rack": "a",
        "role": "zk"
      }
    }
  ]
}
```

`queue -p foo leave` removes the process. Later processes move forward one
position and keep their counters:

```json
{
  "id": "myqueue",
  "region": "eu-west-1",
  "table": "Counters",
  "fencing_token": 3
}
```

`queue -p foo get` prints one ticket, or exits 1 with
`error: ticket not found for process_id foo`. `queue rm` deletes the queue.

## Example uses

### Assigning server IDs to a Zookeeper cluster

Zookeeper stores atomic counters well, but you cannot use it to bootstrap
itself. On each node's first boot, join a queue with a unique host name (an EC2
instance ID works) and write the counter to `myid`:

```sh
monotone -i myzkcluster queue -p "$(hostname -f)" join | jq .ticket.counter > /etc/zookeeper/conf/myid
```

Zookeeper server IDs must fall between 1 and 255. Monotone issues the full
`u64` range, so recycle the queue before it runs out.

### Simple leader election or lock

Treat the process at position 0 as the leader. This has limits:

1. Nothing checks liveness, so a failed process stays in the queue until
   something removes it.
2. Pass the fencing token to downstream conditional writes so a leader acting on
   a stale view is rejected.

## Storage format

Counters and queues share one table with a string hash key `ID`. Each row has a
`Type` (`COUNTER` or `QUEUE`), a numeric `Version` used for conditional writes,
and a numeric `Value`. Queue rows add `Items`, a string set of JSON entries.
Version 0.5 reads and writes rows created by 0.4.

## Development

```sh
docker run -d -p 8000:8000 amazon/dynamodb-local -jar DynamoDBLocal.jar -inMemory -sharedDb

export AWS_ENDPOINT_URL=http://localhost:8000
export AWS_ACCESS_KEY_ID=local AWS_SECRET_ACCESS_KEY=local AWS_REGION=eu-west-1

cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Without `AWS_ENDPOINT_URL`, the DynamoDB and CLI integration tests print a
notice and pass. Set `MONOTONE_REQUIRE_INTEGRATION=1` to make them fail instead,
as CI does. The [`terraform`](terraform) module provisions optional credentials
for running the tests against real AWS.

`cargo audit` and `cargo deny check` run in CI as advisory reports. Record any
advisory you accept in `deny.toml` with a reason.

## Migrating from 0.4

- `MonotonicCounter` and `MonotonicQueue` are async. Await every call inside a
  tokio runtime.
- `join_queue` takes `&str` for the process ID.
- `monotone::local` is now `monotone::memory`. Create a `memory::Store` and take
  counters and queues from it.
- `monotone::aws` is now `monotone::dynamodb`, built on `aws-sdk-dynamodb`.
  Construct backends with an SDK `Client`, a table name and an ID. The `aws`
  feature still works as an alias for `dynamodb`.
- Errors are plain enums: `monotone::Error` for the core and memory backend,
  `monotone::dynamodb::Error` for DynamoDB.
- The in-memory queue now issues counter 1 to the first joiner, matching
  DynamoDB.
- The CLI's `join` takes `--tag KEY=VALUE`. The `-t` short flag always means
  `--table`.

## Releasing

See [RELEASING.md](RELEASING.md).

## License

Apache-2.0. See [LICENSE](LICENSE).
