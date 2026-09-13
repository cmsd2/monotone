## Purpose

Both DynamoDB-backed structures (counter and queue) share one storage layer: a single DynamoDB table keyed by a string `ID`, one item per counter or queue, a `Type` attribute that discriminates the two, and a `Version` attribute used for optimistic locking with conditional writes. This capability also covers on-demand table creation and the retry loop that turns a failed conditional write into a re-read and retry.

## Requirements

### Requirement: Table schema
The system SHALL store all counters and queues in one DynamoDB table with a single hash key attribute `ID` of type string and no sort key. The default table name SHALL be `Counters`.

#### Scenario: Table created by the library
- **WHEN** the library creates the table
- **THEN** the table has exactly one key schema element, `ID` of type `HASH`, with attribute type `S`

### Requirement: Item schema
Every item SHALL carry `ID` (string), `Type` (string, either `COUNTER` or `QUEUE`), `Version` (number, unsigned) and `Value` (number, unsigned). Queue items SHALL additionally carry `Items` as a DynamoDB string set when the queue is non-empty, where each member is a JSON object with fields `process_id` (string), `counter` (unsigned integer) and `tags` (object of string to string, or null). When a queue is empty the `Items` attribute SHALL be omitted.

#### Scenario: Counter item
- **WHEN** a counter with ID `c1` has been incremented three times
- **THEN** its item is `{ID: "c1", Type: "COUNTER", Version: 3, Value: 3}`

#### Scenario: Queue item
- **WHEN** `foo` has joined queue `q1` with no tags
- **THEN** its item is `{ID: "q1", Type: "QUEUE", Version: 1, Value: 1, Items: {"{\"process_id\":\"foo\",\"counter\":1,\"tags\":{}}"}}`

#### Scenario: Empty queue after leave
- **WHEN** the only process leaves a queue
- **THEN** the written item has no `Items` attribute

#### Scenario: Missing Items on read
- **WHEN** a queue item is read and it has no `Items` attribute
- **THEN** the queue is treated as empty

### Requirement: Type discrimination
Reading an item SHALL fail if its `Type` does not match the structure being read. A counter reading a `QUEUE` item SHALL return `UnrecognisedCounterType`. A queue reading a `COUNTER` item SHALL return `UnrecognisedQueueType`. This prevents counter operations being run against a queue's row and vice versa.

#### Scenario: Counter command on a queue row
- **WHEN** a counter is constructed with the ID of an existing queue and `get_value` is called
- **THEN** it returns `UnrecognisedCounterType`

### Requirement: Consistent reads
All reads of counter and queue items SHALL use strongly consistent reads.

#### Scenario: Read after write
- **WHEN** a process writes version N and immediately reads
- **THEN** the read observes version N or later, never an earlier version

### Requirement: Optimistic locking
Every write SHALL be a `PutItem` that replaces the whole item, sets `Version` to the read version plus one, and carries the condition `Version = :version OR attribute_not_exists(Version)` where `:version` is the version that was read. A write for a fresh structure SHALL use read version 0. A conditional check failure SHALL surface as `ConditionalUpdateFailed`.

#### Scenario: Write against an unchanged item
- **WHEN** the item was read at version 4 and is still at version 4
- **THEN** the put succeeds and the item is now at version 5

#### Scenario: Write against a changed item
- **WHEN** the item was read at version 4 but another writer has since moved it to version 5
- **THEN** the put is rejected by DynamoDB and the library reports `ConditionalUpdateFailed`

#### Scenario: Write to create a new item
- **WHEN** no item exists and the writer uses read version 0
- **THEN** the put succeeds and the item is created at version 1

#### Scenario: Two writers race to create
- **WHEN** two processes both observe no item and both put with read version 0
- **THEN** exactly one put succeeds and the other receives `ConditionalUpdateFailed`

### Requirement: Retry on conditional failure
Mutating operations (`next_value`, `join_queue`, `leave_queue`) SHALL retry indefinitely on `ConditionalUpdateFailed`. Before each retry the caller SHALL sleep for a configurable base retry time plus a random jitter drawn uniformly from `[0, jitter_millis)` milliseconds. The default jitter SHALL be 100 ms. Any other error SHALL abort the operation immediately.

#### Scenario: Retry re-reads state
- **WHEN** a conditional failure occurs
- **THEN** the operation sleeps, re-reads the item, recomputes its change from the fresh state, and puts again

#### Scenario: Non-conditional error
- **WHEN** the put fails for a reason other than the condition (for example a throttling or credentials error)
- **THEN** the operation returns that error without retrying

### Requirement: Table provisioning
The system SHALL provide `create_table_if_needed(client, name, read_capacity, write_capacity)` which describes the table, creates it with the given provisioned throughput if DynamoDB reports it does not exist, and returns the table description. A "table already exists" response during creation SHALL be treated as success and the table re-described. The system SHALL also provide `wait_for_table(client, name)` which polls `DescribeTable` once per second until the table status is `ACTIVE`.

#### Scenario: Table absent
- **WHEN** `create_table_if_needed` is called for a table that does not exist
- **THEN** it issues `CreateTable` with hash key `ID` and the given throughput and returns the new table's description

#### Scenario: Table present
- **WHEN** `create_table_if_needed` is called for a table that exists
- **THEN** it returns the existing description without calling `CreateTable`

#### Scenario: Concurrent creation
- **WHEN** another process creates the table between this process's describe and create calls
- **THEN** the "already exists" error is swallowed and the existing table is described and returned

#### Scenario: Waiting for activation
- **WHEN** `wait_for_table` is called while the table is `CREATING`
- **THEN** it polls every second and returns once the status is `ACTIVE`

### Requirement: Error classification
DynamoDB errors that arrive as unstructured JSON SHALL be classified by message prefix into typed errors: "Requested resource not found: Table:" SHALL become `TableNotFound`, "Table already exists:" SHALL become `TableAlreadyExists`, and "The conditional request failed" SHALL become `ConditionalUpdateFailed`. Any other unstructured error SHALL be passed through as the underlying service error.

#### Scenario: Unknown error passes through
- **WHEN** DynamoDB returns an error whose message matches none of the known prefixes
- **THEN** the caller receives the original service error

### Requirement: Item deletion
`remove` on a counter or queue SHALL issue an unconditional `DeleteItem` for the structure's `ID`.

#### Scenario: Delete
- **WHEN** `remove` is called
- **THEN** the item with that ID no longer exists, and no error is raised if it was already absent
