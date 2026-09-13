## 1. Workspace and toolchain

- [x] 1.1 Convert the repo to a Cargo workspace with members `monotone` and `cli`, one root `Cargo.lock`, and delete the two per-crate lock files; verify `cargo metadata --format-version 1 | jq .workspace_members` lists both crates
- [x] 1.2 Set `edition = "2024"`, `rust-version = "1.88"` and version `0.5.0` in both manifests, add `rust-toolchain.toml` pinning stable, and verify `cargo +1.88 check --workspace` (with fallback resolution) and `cargo check --workspace` both pass on an empty-bodied skeleton
- [x] 1.3 Replace dependencies: remove rusoto, error-chain, hyper, serde 0.9, log, rand 0.3, clap 2, env_logger; add serde 1, serde_json 1, thiserror, rand 0.10 (SmallRng), and under feature `dynamodb` (with `aws` as an alias) aws-sdk-dynamodb, aws-config, tokio; add clap 4, tokio, tracing, tracing-subscriber to the CLI; verify `cargo tree -e no-dev` for the library with default features shows no tokio, hyper or aws crates
- [x] 1.4 Add `deny.toml` (advisories with a documented ignore list, licenses allow Apache-2.0/MIT/BSD/ISC/Unicode/Zlib, bans multiple-versions warn) and drop the AWS crates' legacy `rustls` default feature; verify `cargo deny check licenses bans sources` passes and record the `cargo audit` result, adding any unfixable advisory to the ignore list with a reason

## 2. Sans-IO core

- [x] 2.1 Implement `core::row` with `CounterRow`, `QueueRow`, `QueueEntry` and pure `increment`, `join`, `leave`, `ticket`, `tickets`; verify unit tests cover every scenario under "Pure row operations" and "Counter numbering" (first joiner gets 1)
- [x] 2.2 Implement `core::encode` with backend-neutral `Item`/`Attr` and encode/decode for both row types, including `Type` checking, missing-attribute and non-integer errors, sorting of `Items` on decode, and `tags: null` compatibility; verify fixture tests for the counter item, queue item, empty queue (no `Items`), missing `Items`, and a proptest round trip pass
- [x] 2.3 Implement `core::machine` with `Effect`, `Input`, `Step`, the generic read-modify-write driver, the read-only driver, injected `SmallRng`, configurable retry time and jitter bound (default 100 ms), and a protocol error for mismatched input; verify unit tests script every scenario under "Effect-driven operations", "Conflict retry sequencing" and "Deterministic jitter"
- [x] 2.4 Add proptest over random sequences of join/leave/next applied through the machines with randomly injected conflicts; verify the invariants (no counter reuse, dense positions, version increments once per write, result matches last read) hold for 1,000 cases
- [x] 2.5 Define `core::Error` with thiserror, keeping the `ticket not found for process_id {0}` display text; verify a test asserts the exact string

## 3. Async traits and in-memory backend

- [x] 3.1 Rewrite `MonotonicCounter` and `MonotonicQueue` in `lib.rs` as native `async fn` traits with associated `Error`; verify the crate compiles with default features and `cargo doc --no-deps` has no broken intra-doc links
- [x] 3.2 Implement `memory::Store` (`Arc<Mutex<HashMap<String, (u64, Item)>>>`) that executes `Read`, version-checked `Write` and no-op `Sleep`, and `memory::Counter` / `memory::Queue` driving the machines; verify every scenario in the counter and queue specs passes through the async traits using `tokio::test`
- [x] 3.3 Add a memory concurrency test spawning 100 tasks calling `next_value` and 50 tasks calling `join_queue` with distinct IDs; verify all values and counters are distinct and the final token equals the write count
- [x] 3.4 Add a hand-interleaved conflict test that drives two machines against one memory store step by step so the second `Write` conflicts; verify the loser re-reads and its final result reflects the winner's write

## 4. DynamoDB backend

- [x] 4.1 Implement `dynamodb::client` with `create_table_if_needed`, `wait_for_table`, `describe_table`, `create_table`, `list_tables` on `aws_sdk_dynamodb::Client`, mapping `ResourceNotFoundException` and `ResourceInUseException` to typed errors; verify an integration test against DynamoDB Local creates a fresh table, returns the existing one on a second call, and reports `TableNotFound` for a missing name
- [x] 4.2 Implement `Attr` to and from `AttributeValue` conversion and the `dynamodb::adapter` that executes `Read` as a consistent `GetItem`, `Write` as a conditional `PutItem` with `Version = :version OR attribute_not_exists(Version)`, and `Sleep` as `tokio::time::sleep`, mapping `ConditionalCheckFailedException` to `ConditionalUpdateFailed` and wrapping everything else; verify a unit test with a stubbed executor checks the exact condition expression and expression values
- [x] 4.3 Implement `dynamodb::Counter` and `dynamodb::Queue` including `remove`; verify integration tests against DynamoDB Local pass for every scenario in the counter and queue specs, including "Leave on a missing queue returns promptly" with a 5 second timeout
- [x] 4.4 Add a real conflict integration test: two clients read the same row, both put, and verify exactly one gets `ConditionalUpdateFailed` and that the high-level `next_value` on both clients yields distinct consecutive values
- [x] 4.5 Add the backend equivalence test running the script from the sans-io-core spec against memory and DynamoDB Local; verify the two result vectors are equal
- [x] 4.6 Add a compatibility test that writes a 0.4-shaped queue item (with `"tags":null` entries) directly via the SDK; verify `get_tickets` decodes it with empty tag maps and a subsequent join writes a row 0.5 can read back

## 5. CLI

- [x] 5.1 Rewrite the CLI on clap 4 derive with `#[command(name = "monotone", version)]`, the option tree from the cli spec, `--tag` with no short form, and `#[tokio::main(flavor = "current_thread")]`; verify `monotone --version` prints `monotone 0.5.0` and `monotone --help` names both subcommands
- [x] 5.2 Load AWS config via `aws_config::defaults(BehaviorVersion::latest())` with the `--region` override applied and `AWS_ENDPOINT_URL` honoured by the SDK; verify a test with `AWS_ENDPOINT_URL` set to a closed port exits 1 with a connection error on stderr and nothing on stdout
- [x] 5.3 Implement the counter and queue subcommands with the JSON output structs from the cli spec, table auto-provisioning at 1/1 before each operation, and `rm` printing nothing; verify `assert_cmd` tests against DynamoDB Local match the exact pretty-printed JSON for get, next, join, leave, list including `tags`
- [x] 5.4 Implement error handling: missing `--id` or `--process` prints a one-line error plus help and exits 1; library errors print `error: <message>` to stderr and exit 1 with no panic; verify tests for missing id, missing process, malformed tag, not-found ticket, and counter-on-queue-row each assert exit status 1 and the stderr substring from the spec
- [x] 5.5 Wire `tracing-subscriber` with `EnvFilter::from_default_env()` to stderr; verify a test with `RUST_LOG` unset has empty stderr on success and one with `RUST_LOG=debug` has non-empty stderr while stdout is unchanged

## 6. CI and repository hygiene

- [x] 6.1 Delete `.travis.yml` and confirm no other file in the tree contains an `AKIA` access key ID; verify `git grep -nE 'AKIA[0-9A-Z]{16}'` returns nothing
- [ ] 6.2 Add `.github/workflows/ci.yml` with `lint`, `test` (stable with the committed lock plus an MSRV 1.88 build using fallback resolution, `amazon/dynamodb-local` service on 8000, env `AWS_ENDPOINT_URL`, placeholder credentials and region) and a non-required `audit` job running cargo-audit and cargo-deny; verify the required jobs pass on a push to a branch
- [ ] 6.3 Make integration and CLI tests self-skip only when `AWS_ENDPOINT_URL` is unset locally but hard-fail in CI by setting `MONOTONE_REQUIRE_INTEGRATION=1` in the workflow; verify a CI run with the service removed fails
- [ ] 6.4 Add `.github/workflows/audit.yml` on a weekly cron running `cargo audit`; verify it appears under Actions and a manual `workflow_dispatch` run succeeds
- [ ] 6.5 Add `.github/workflows/release.yml` on `v*` tags: check both manifests equal the tag, run the full check, `cargo publish -p monotone`, wait for the index, `cargo publish -p monotone-cli`, build binaries for linux x86_64, macOS aarch64 and macOS x86_64, and create a GitHub release with them attached; pre-release tags such as `v0.5.0-rc.1` run a publish dry run and create no GitHub release; verify a `v0.5.0-rc.1` tag run passes every job and publishes nothing
- [ ] 6.6 Enable secret scanning and push protection on the GitHub repository and add Dependabot config for cargo and github-actions ecosystems; verify `gh api repos/cmsd2/monotone --jq .security_and_analysis` shows both enabled

## 7. Terraform and documentation

- [x] 7.1 Update `terraform/` to current provider syntax (`required_providers`, `templatefile` instead of `template_file`), extend the IAM policy to include `DescribeTable`, `CreateTable` and `ListTables`, rename the Travis-specific user, and rewrite `terraform/README.md` to describe it as optional real-AWS test infrastructure; verify `terraform validate` passes
- [x] 7.2 Rewrite `README.md`: GitHub Actions badge, async usage example, `dynamodb` feature, MSRV, DynamoDB Local instructions with `AWS_ENDPOINT_URL`, CLI examples showing `tags` in output and `--tag`, and a 0.4 to 0.5 migration section; verify every code block compiles or runs as written
- [x] 7.3 Rewrite `RELEASING.md` for the workspace and tag-driven release (bump both versions, update README, commit, tag `vX.Y.Z`, push tag, watch the release workflow); verify the steps match `release.yml`
- [ ] 7.4 Close GitHub issue #1 (rusoto migration) with a comment referencing the change once merged; verify the issue is closed

## 8. Final verification

- [ ] 8.1 Run the full local suite with DynamoDB Local in Docker: `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace --all-features`; verify all pass with zero warnings, and record the `cargo audit` and `cargo deny check` results
- [ ] 8.2 Check the Dependabot alert list after the lock file lands on master; verify every alert from the rusoto-era lock files is closed and any remaining alert is listed in `deny.toml` with a reason
- [ ] 8.3 Walk every scenario in the baseline and delta specs and map it to a test name; verify the mapping table in the pull request description has no gaps
