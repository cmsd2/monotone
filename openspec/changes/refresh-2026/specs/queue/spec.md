## ADDED Requirements

### Requirement: Counter numbering
Every backend SHALL issue counter 1 to the first process to join a fresh queue and SHALL increment by one for each subsequent distinct joiner. The counter value stored on the queue row SHALL equal the highest counter issued so far.

#### Scenario: First and second joiner
- **WHEN** `foo` then `bar` join a fresh queue on any backend
- **THEN** `foo` has counter 1 and `bar` has counter 2

## MODIFIED Requirements

### Requirement: Queue contract
The system SHALL expose a `MonotonicQueue` trait with four asynchronous operations: `join_queue(process_id, tags)`, `leave_queue(process_id)`, `get_ticket(process_id)` and `get_tickets()`. `join_queue`, `get_ticket` and `get_tickets` SHALL return a fencing token alongside their result. `leave_queue` SHALL return the new fencing token. Tags SHALL be an optional ordered map of string to string; passing no tags SHALL be equivalent to passing an empty map.

#### Scenario: Ticket shape
- **WHEN** any operation returns a ticket
- **THEN** the ticket carries the process ID, a counter value, a zero-based position, and a tag map

### Requirement: In-memory queue
The system SHALL provide an in-memory queue that drives the shared sans-IO core against process-local storage and serialises access so concurrent tasks observe the same semantics as separate processes on DynamoDB. Clones of the queue SHALL share state.

#### Scenario: First counter value in memory
- **WHEN** `foo` joins a fresh in-memory queue
- **THEN** its ticket has counter 1 and position 0
- **AND** the next process to join receives counter 2

#### Scenario: Concurrent joins in one process
- **WHEN** several tasks await `join_queue` with distinct process IDs on clones of the same queue
- **THEN** every ticket has a distinct counter and a distinct position
- **AND** the final fencing token equals the number of joins

### Requirement: DynamoDB queue
The system SHALL provide a DynamoDB-backed queue addressed by a table name and an item ID. The queue SHALL be persisted as one item whose `Type` attribute is `QUEUE`, with the fencing token stored as the item `Version`, the last issued counter as `Value`, and the entries as a string set of JSON documents in `Items`. Writes SHALL drive the shared sans-IO core using the optimistic-concurrency protocol defined in the dynamodb-storage capability.

#### Scenario: First counter value on DynamoDB
- **WHEN** `foo` joins a queue with no existing item
- **THEN** its ticket has counter 1 and position 0
- **AND** the next process to join receives counter 2

#### Scenario: Read when no item exists
- **WHEN** `get_tickets` is awaited and no item exists for the queue ID
- **THEN** it returns token 0 and an empty list without creating an item

#### Scenario: Join retries on a concurrent write
- **WHEN** `join_queue` reads version N and another process writes version N+1 first
- **THEN** the conditional put fails and the join re-reads and retries after the retry time plus jitter
- **AND** if the retried read now shows the process already queued, its existing ticket is returned without a further write

#### Scenario: Leave on a missing queue returns promptly
- **WHEN** `leave_queue` is awaited and no item exists for the queue ID
- **THEN** it returns a not-found error without retrying

#### Scenario: Item is not a queue
- **WHEN** any queue operation reads an item whose `Type` attribute is not `QUEUE`
- **THEN** it returns an error identifying the wrong structure type

#### Scenario: Entries are ordered by counter on read
- **WHEN** the `Items` string set is read back from DynamoDB in arbitrary order
- **THEN** the entries are sorted by ascending counter before positions are assigned
