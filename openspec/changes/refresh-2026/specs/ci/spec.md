## Purpose

Continuous integration on GitHub Actions builds and tests both crates on every push and pull request, runs integration tests against a local DynamoDB without any cloud credentials, keeps the dependency tree audited, and publishes releases from a tag.

## ADDED Requirements

### Requirement: Checks on every push and pull request
A workflow SHALL run on every push to any branch and on every pull request. It SHALL run, and fail the check on any failure or warning from: `cargo fmt --check`, `cargo clippy --all-targets --all-features` with warnings denied, `cargo build` for the library with default features and with all features with warnings denied, and `cargo test` for both crates with all features.

#### Scenario: Formatting violation
- **WHEN** a pull request contains a file that `rustfmt` would change
- **THEN** the check fails and the pull request shows a failed status

#### Scenario: Compiler warning
- **WHEN** a change introduces an unused import
- **THEN** the build step fails

### Requirement: Advisory checks report without gating
The workflow SHALL run `cargo audit` and `cargo deny check` on every push and pull request in a job that reports its findings but SHALL NOT be a required check, because an advisory can exist with no available fix. An advisory the maintainers accept SHALL be listed in `deny.toml` under `advisories.ignore` with a reason.

#### Scenario: Unfixable advisory
- **WHEN** a transitive dependency gains a RustSec advisory with no patched release
- **THEN** the advisory job reports it and the required checks still pass

#### Scenario: Accepted advisory
- **WHEN** an advisory ID is listed in `deny.toml` with a reason
- **THEN** `cargo deny check advisories` passes

### Requirement: Toolchain matrix
Tests SHALL run on stable Rust using the committed lock file, and the workspace SHALL build on the minimum supported Rust version declared as `rust-version`, using dependency versions re-resolved for that toolchain. A change that no longer compiles on the declared MSRV SHALL fail the check.

#### Scenario: MSRV regression
- **WHEN** a change uses a language feature newer than the declared MSRV
- **THEN** the MSRV job fails while the stable job passes

#### Scenario: Newer dependencies in the lock file
- **WHEN** the committed lock file holds a dependency release that needs a newer rustc than the MSRV
- **THEN** the MSRV job resolves an older compatible release and still passes

### Requirement: Integration tests need no cloud credentials
Integration tests for the DynamoDB backend and the CLI SHALL run against a DynamoDB Local service container started by the workflow. The workflow SHALL pass the container's endpoint to the tests through `AWS_ENDPOINT_URL` and supply placeholder static credentials and region. No workflow file, secret, or variable SHALL hold long-lived AWS credentials.

#### Scenario: Fresh fork
- **WHEN** someone forks the repository and opens a pull request with no secrets configured
- **THEN** the full check including integration tests passes

#### Scenario: Endpoint not reachable
- **WHEN** the DynamoDB Local container fails to start
- **THEN** the integration test job fails rather than silently skipping the integration tests

### Requirement: Scheduled dependency audit
A workflow SHALL run `cargo audit` on a weekly schedule against the default branch so new advisories surface without a code change.

#### Scenario: Weekly run
- **WHEN** the scheduled time passes
- **THEN** the audit runs and reports failure if any advisory applies

### Requirement: Tag-driven release
Pushing a tag of the form `v<semver>` SHALL trigger a release workflow that verifies both crate manifests declare a version equal to the tag, runs the full check, publishes the library and then the CLI to crates.io using a repository secret holding a crates.io token, and creates a GitHub release with CLI binaries for Linux x86_64, macOS aarch64 and macOS x86_64 attached.

#### Scenario: Version mismatch
- **WHEN** tag `v0.5.1` is pushed but the manifests declare 0.5.0
- **THEN** the release workflow fails before publishing anything

#### Scenario: Successful release
- **WHEN** tag `v0.5.0` is pushed and manifests declare 0.5.0
- **THEN** `monotone 0.5.0` and `monotone-cli 0.5.0` appear on crates.io and a GitHub release `v0.5.0` exists with three binaries

### Requirement: Repository secret hygiene
The working tree SHALL contain no cloud access key IDs or secrets. GitHub secret scanning and push protection SHALL be enabled on the repository.

#### Scenario: Key pushed by mistake
- **WHEN** a commit containing an AWS access key ID is pushed
- **THEN** push protection rejects the push
