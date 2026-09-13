## Why

The repository has not changed since April 2017. Neither crate compiles on a current Rust toolchain, every one of the 37 open Dependabot alerts is transitive through the abandoned `rusoto 0.24` mega-crate, the Travis CI configuration points at a service that no longer exists, and a retired AWS access key ID is committed in the tree. The library's core idea (a monotonic counter and a fenced queue on DynamoDB conditional writes) is still useful, so the crate is worth reviving rather than archiving.

## What Changes

- **BREAKING** Restructure the library around a sans-IO core. Row logic and the read-modify-write retry loop become pure code that emits effects (`Read`, `Write`, `Sleep`) and consumes results. All backends drive that one core, so the in-memory and DynamoDB implementations share identical semantics.
- **BREAKING** Replace `rusoto` with the official `aws-sdk-dynamodb` crate through an async adapter on tokio. `MonotonicCounter` and `MonotonicQueue` become async traits. Library version moves to 0.5.0.
- **BREAKING** The in-memory queue issues counter 1 to the first joiner, matching the DynamoDB backend and the README, instead of 0.
- Fix DynamoDB `leave_queue` on a missing queue, which currently loops forever instead of returning not-found.
- Migrate to Rust edition 2024, `serde 1`, `clap 4`, `thiserror`, `rand 0.10`, `tracing`. Remove `error-chain` and the unused direct `hyper` dependency. Regenerate both lock files.
- Add exhaustive tests: unit and property tests over the pure core with simulated conflicts, fixture tests for the DynamoDB item encoding, integration tests for the adapter against DynamoDB Local, and end-to-end CLI tests that assert JSON output and exit codes.
- Replace Travis with GitHub Actions: build, test, clippy, fmt, `cargo audit` and `cargo deny` on every push and pull request, DynamoDB Local as a service container, a matrix over stable and the declared MSRV, and a tag-triggered release job that publishes to crates.io.
- Remove `.travis.yml` and the committed AWS access key ID from the working tree. Enable GitHub secret scanning and push protection. Git history is not rewritten.
- Refresh the terraform test infrastructure so it is optional: document that CI uses DynamoDB Local, keep the module for people who want a real table, and update its provider syntax.
- Update README (async examples, tags in output, endpoint override for DynamoDB Local, MSRV, badge) and RELEASING (tag-driven release through Actions).
- Make the CLI report its real crate name and version, exit with status 1 and a one-line error on stderr on library failures instead of panicking, and honour `AWS_ENDPOINT_URL` for local testing.

## Capabilities

### New Capabilities
- `sans-io-core`: the effect-driven read-modify-write driver and the pure row operations that every backend shares, including retry sequencing on write conflicts and the invariant that any adapter driving the core observes the same results.
- `ci`: what the GitHub Actions workflows run, what gates a merge, how integration tests obtain a DynamoDB endpoint, and how a release is cut.

### Modified Capabilities
- `counter`: trait becomes async; the in-memory counter is an adapter over the shared core.
- `queue`: trait becomes async; in-memory first counter becomes 1; in-memory and DynamoDB requirements merge into one backend-independent set.
- `dynamodb-storage`: error classification moves from JSON message-prefix matching to typed SDK errors; the adapter accepts a configured endpoint so DynamoDB Local can be targeted; retry policy is unchanged but now lives in the core.
- `cli`: runs on an async runtime; `--version` reports the crate version; failures exit with status 1 and a message rather than a panic; `AWS_ENDPOINT_URL` is honoured; the `-t` short flag on `join` becomes `--tag` only, freeing `-t` for `--table`.

## Impact

- **Public API**: async traits, new error types via `thiserror`, new module layout (`core`, `memory`, `dynamodb`). Any 0.4 user must update. Semver bump to 0.5.0 for both crates.
- **Dependencies removed**: rusoto, error-chain, hyper, serde 0.9, log 0.3, rand 0.3, clap 2, env_logger 0.4. This clears all 37 Dependabot alerts.
- **Dependencies added**: aws-sdk-dynamodb, aws-config, tokio, thiserror, serde 1, serde_json 1, clap 4, tracing, tracing-subscriber, rand 0.10; dev: proptest, assert_cmd, predicates, tokio-test.
- **Infrastructure**: `.travis.yml` deleted; `.github/workflows/` added; `deny.toml` and `rust-toolchain.toml` added; terraform updated but no longer required for CI.
- **Security**: committed key ID removed from the tree. The key must be confirmed deleted in the AWS account as an out-of-band step.
- **Docs**: README, RELEASING, terraform/README rewritten for the new toolchain and workflow.
