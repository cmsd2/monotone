## ADDED Requirements

### Requirement: Endpoint configuration
The DynamoDB backend SHALL accept a caller-supplied client configuration so that it can target any DynamoDB-compatible endpoint. When configuration is loaded from the environment, the `AWS_ENDPOINT_URL` variable SHALL override the endpoint, allowing DynamoDB Local to be used for tests and local development with placeholder credentials.

#### Scenario: DynamoDB Local
- **WHEN** `AWS_ENDPOINT_URL=http://localhost:8000` and placeholder credentials are set and the environment configuration is loaded
- **THEN** all table and item calls go to the local endpoint and succeed without contacting AWS

#### Scenario: No override
- **WHEN** `AWS_ENDPOINT_URL` is unset
- **THEN** calls go to the regional DynamoDB endpoint for the configured region

## MODIFIED Requirements

### Requirement: Retry on conditional failure
Mutating operations (`next_value`, `join_queue`, `leave_queue`) SHALL retry indefinitely on a conditional check failure, following the conflict retry sequencing defined in the sans-io-core capability: before each retry the backend SHALL sleep for a configurable base retry time plus a random jitter drawn uniformly from `[0, jitter_millis)` milliseconds, with a default jitter of 100 ms, then re-read and recompute. Any error other than a conditional check failure SHALL abort the operation immediately and SHALL be returned to the caller.

#### Scenario: Retry re-reads state
- **WHEN** a conditional failure occurs
- **THEN** the operation sleeps, re-reads the item, recomputes its change from the fresh state, and puts again

#### Scenario: Non-conditional error
- **WHEN** the put fails for a reason other than the condition (for example a throttling or credentials error)
- **THEN** the operation returns that error without retrying

### Requirement: Error classification
The backend SHALL classify DynamoDB service errors by their typed error kind, not by parsing message text. A conditional check failure on `PutItem` SHALL become `ConditionalUpdateFailed`. A resource-not-found error on `DescribeTable` SHALL become `TableNotFound`. A resource-in-use error on `CreateTable` SHALL become `TableAlreadyExists`. Every other service, transport or credential error SHALL be returned wrapped, preserving the original error for inspection.

#### Scenario: Conditional check failure
- **WHEN** `PutItem` fails with a conditional check failed error
- **THEN** the backend reports `ConditionalUpdateFailed`

#### Scenario: Table missing
- **WHEN** `DescribeTable` fails with a resource not found error
- **THEN** the backend reports `TableNotFound` naming the table

#### Scenario: Unknown error passes through
- **WHEN** DynamoDB returns any other error, or the request fails at the transport layer
- **THEN** the caller receives a wrapped error from which the original can be recovered
