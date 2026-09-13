## Context

See proposal.md for motivation. The current code is about 1,600 lines across two crates: a library with an in-memory backend and a rusoto-based DynamoDB backend, and a clap 2 CLI. Both backends reimplement the same read-modify-write loop independently, which is how they drifted (first counter 0 versus 1, and the DynamoDB `leave_queue` hang). The official AWS SDK for Rust is async-only, so a straight port would force the async question onto the library API. The baseline specs under `openspec/specs/` record current behaviour and are the compatibility target, except where the delta specs deliberately change it.

Constraints:
- The library must still build with default features without any HTTP or async runtime, because the in-memory backend is used by callers with no AWS dependency.
- Integration tests must run in CI with no AWS account, using DynamoDB Local.
- The public crate names `monotone` and `monotone-cli` and the binary name `monotone` stay as they are.

## Goals / Non-Goals

**Goals:**
- One implementation of the counter and queue rules, exercised by every backend.
- The conflict retry loop testable in a unit test by scripting conflicts, with no clock and no database.
- Default-feature build has no I/O dependencies. The `dynamodb` feature pulls in the SDK and tokio.
- The CLI is a thin consumer of the async DynamoDB backend and is tested end to end.
- CI green on a fresh fork with no secrets.

**Non-Goals:**
- A synchronous public API. The core is I/O-free, so a blocking adapter is a later, separate change if anyone asks.
- Changing the DynamoDB item schema. Existing tables written by 0.4 must remain readable and writable by 0.5.
- Liveness checking, TTLs or leader election. The README's caveats stand.
- Rewriting git history to remove the retired key ID.
- Terraform-managed CI infrastructure. The module stays as an optional convenience.

## Decisions

### D1: Sans-IO core with an explicit effect enum
The core exposes each operation as a small state machine: `start()` returns the first `Effect`, `step(Input)` returns either the next `Effect` or `Done(T)`. Effects are `Read`, `Write { row, expected_version }`, `Sleep(Duration)`. All three mutating operations are one generic read-modify-write machine parameterised by a pure `modify: Fn(Option<Row>) -> Modify<Row, T>` where `Modify` is `Write(row, result)`, `NoWrite(result)` or `Fail(error)`. Read-only operations are `Read` then a pure projection.

Alternatives considered:
- *Port trait (`Store` with async `read`/`put_if_version`)*: simpler, but the trait itself must be async or sync, which puts the runtime question back into the core and makes the retry loop untestable without an async test harness.
- *Generators / coroutines*: not stable in Rust 2024 edition. The hand-rolled enum machine is small enough (one generic machine, two effect kinds) that the ergonomics loss is minor.
- *No sans-IO, just port to aws-sdk*: fastest, but leaves the duplicate loops and the divergence bugs in place and makes "exhaustive tests" mean "exhaustive integration tests", which are slow and flaky.

### D2: Jitter from an injected RNG
The machine takes `rand::rngs::SmallRng` seeded at construction (from entropy in production, from a fixed seed in tests). The `Sleep` effect carries the computed duration so adapters never need randomness. Alternative: emit `Backoff { attempt }` and let the adapter choose. Rejected because the retry policy is spec behaviour (dynamodb-storage: Retry on conditional failure) and must be identical across backends.

### D3: Row types and encoding live in the core
`CounterRow`, `QueueRow`, `QueueEntry` and their encode/decode to a backend-neutral `Item` (a `BTreeMap<String, Attr>` where `Attr` is `S(String) | N(u64) | SS(BTreeSet<String>)`) sit in the core. The DynamoDB adapter converts `Attr` to and from the SDK's `AttributeValue` in a dozen lines. This keeps the item schema and its fixture tests I/O-free and keeps SDK types out of the core. `QueueEntry` keeps `tags: Option<BTreeMap>` on the wire for compatibility with 0.4 rows that wrote `null`, and always decodes to an empty map.

### D4: Async traits with `async fn` in traits
`MonotonicCounter` and `MonotonicQueue` use native `async fn` (stable since 1.75, edition 2024). Error type stays an associated type. No `async-trait` crate, no boxing. Trait objects are not a goal; callers are expected to name the concrete backend or be generic.

### D5: Module layout
```
monotone/src/
  lib.rs          traits, Ticket, FencingToken
  core/           machine.rs (effects, Rmw driver), row.rs, encode.rs, error.rs
  memory/         counter.rs, queue.rs  (Arc<Mutex<HashMap<String, Item>>> store)
  dynamodb/       client.rs (table ops), adapter.rs (drive machine), counter.rs, queue.rs, error.rs   [feature = "dynamodb"]
```
Feature `aws` is kept as an alias of `dynamodb` for one release so `features = ["aws"]` in downstream manifests keeps working.

### D6: In-memory backend is an adapter, not a separate implementation
The memory backend holds `Arc<Mutex<HashMap<String, (u64, Item)>>>` and drives the same machine. `Write` compares versions under the lock, so it really does exercise the conflict path when tests interleave two drivers by hand. `Sleep` is a no-op in memory. This is what makes the backend-equivalence requirement hold by construction.

### D7: DynamoDB adapter error mapping
Use the SDK's typed error enums: `PutItemError::ConditionalCheckFailedException`, `DescribeTableError::ResourceNotFoundException`, `CreateTableError::ResourceInUseException`. Everything else is wrapped in `Error::Sdk(Box<dyn Error + Send + Sync>)` with `source()` preserved. The 0.4 message-prefix matching is gone.

### D8: Errors with thiserror
One `Error` enum per layer (`core::Error`, `dynamodb::Error`, CLI error) with `#[from]` conversions. `NotFound(process_id)` keeps the display text `ticket not found for process_id {0}` because the CLI spec asserts it.

### D9: Table provisioning stays in the adapter
`create_table_if_needed` and `wait_for_table` are plain async functions on the DynamoDB client. Provisioned throughput stays at 1/1 from the CLI to match current behaviour and because DynamoDB Local ignores billing mode anyway.

### D10: CLI on clap 4 derive and tokio
`#[tokio::main(flavor = "current_thread")]`. clap derive with `#[command(version)]` so `--version` comes from the manifest. `run()` returns `Result<(), Error>`; `main` prints `error: {e}` to stderr and exits 1. `--tag` loses its `-t` short form; clap 4 rejects the conflicting short that clap 2 tolerated. `tracing-subscriber` with `EnvFilter::from_default_env()` writing to stderr.

### D11: Test layers
- `core` unit tests: every scenario in sans-io-core, counter, queue by scripting inputs into machines. `proptest` strategies over operation sequences check the invariants in "Pure row operations".
- Encoding fixture tests: the item examples in dynamodb-storage as literal `Item` values, plus a 0.4-shaped item with `"tags": null`.
- Memory backend tests: the same scenarios through the async trait, plus a concurrency test with `tokio::spawn`.
- DynamoDB integration tests (`monotone/tests/dynamodb.rs`, `#[ignore]` unless `AWS_ENDPOINT_URL` is set): table provisioning, one happy path per operation, a real two-client conflict, error mapping, and the equivalence script run on both backends.
- CLI tests (`cli/tests/cli.rs`, `assert_cmd`): every output shape and exit code in the cli spec against DynamoDB Local, with a unique table name per test run.

### D12: CI shape
One `ci.yml` with jobs: `lint` (fmt, clippy), `test` (matrix stable + MSRV, services: `amazon/dynamodb-local:latest` on 8000, env `AWS_ENDPOINT_URL`, `AWS_ACCESS_KEY_ID=local`, `AWS_SECRET_ACCESS_KEY=local`, `AWS_REGION=eu-west-1`), `audit` (cargo-audit, cargo-deny), which reports but is not a required check. `audit.yml` on `schedule: cron weekly`. `release.yml` on `push: tags: v*` with a version-check step, then `cargo publish -p monotone`, wait for index, `cargo publish -p monotone-cli`, then build and upload binaries. Both crates move into a Cargo workspace at the root so one lock file and one `cargo test --workspace` cover everything.

### D13: MSRV and dependency bounds
`rust-version = "1.88"`. The original plan used 1.85 (the edition 2024 minimum), but no `aws-sdk-dynamodb` 1.x release resolves with auth crates that build on 1.85; 1.88 is the oldest toolchain the SDK line supports.

Manifests use loose major-version requirements (`aws-sdk-dynamodb = "1"`, `tokio = "1"`, and so on) rather than tracking minors. The committed `Cargo.lock` holds the newest releases, even where they need a newer rustc than the MSRV; `.cargo/config.toml` sets `resolver.incompatible-rust-versions = "allow"` so routine `cargo update` keeps doing that. The MSRV CI job re-resolves with `CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback` before building, so it proves the declared bound without holding the lock file back. `rust-toolchain.toml` pins stable for contributors.

The AWS crates are used with `default-features = false` and `default-https-client` plus `rt-tokio`. Their default `rustls` feature pulls a legacy hyper 0.14 / rustls 0.21 connector carrying open advisories.

## Risks / Trade-offs

- [Lock-step version bump breaks 0.4 users] → Semver minor on 0.x signals breakage. README gets a migration section (sync to async, feature rename, module paths).
- [aws-sdk crates are heavy and change often] → Loose manifest bounds with a fresh lock file; `cargo deny` warns on duplicate versions; the weekly audit workflow surfaces advisories. Advisories with no fix are accepted in `deny.toml` with a reason rather than blocking work.
- [DynamoDB Local behaviour differs from the service] → It implements conditional puts and the error types used here. The terraform module remains for anyone who wants to run the same tests against a real table; document the env vars.
- [Infinite retry with no cap] → Preserved from 0.4 and stated in the spec. The machine exposes `attempts` so a future change can add a cap without touching adapters. Noted as an open question.
- [Hand-rolled state machine is unfamiliar] → It is one generic type of roughly 100 lines with exhaustive unit tests; the design doc and module docs describe the protocol.
- [Workspace conversion changes the crates.io source layout] → `cargo publish -p` handles workspace members; `RELEASING.md` is rewritten for it.
- [Retired key ID remains in history] → Out of scope by decision; the IAM key must be confirmed deleted out of band. Push protection prevents recurrence.

## Migration Plan

1. Land the change on a branch; CI must be green on the branch before merge.
2. Merge to master, tag `v0.5.0`, let the release workflow publish.
3. Yank nothing: 0.4.0 stays installable for anyone pinned to it.
4. Rollback is `git revert` of the merge; crates.io publishes are immutable, so a broken 0.5.0 is followed by 0.5.1.

## Open Questions

- Should retries be capped (with a `RetryExhausted` error) in a later release? The machine will expose the attempt count so this can be added without changing adapters.
- Should the release workflow also build a Linux aarch64 binary? Cheap to add later; omitted from the spec for now.
