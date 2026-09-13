## Purpose

A monotonic queue is a named, ordered list of process IDs. Each process that joins receives a ticket holding a counter value that never repeats within the queue, its current position, and an optional set of string tags. Every read and write also returns a fencing token that increases with each write to the queue, so callers can detect a stale view before acting on it. The library exposes one trait, `MonotonicQueue`, with an in-memory implementation and a DynamoDB implementation. Typical uses are assigning stable small integer IDs to cluster members and naive leader election by taking the process at position 0.

## Requirements

### Requirement: Queue contract
The system SHALL expose a `MonotonicQueue` trait with four operations: `join_queue(process_id, tags)`, `leave_queue(process_id)`, `get_ticket(process_id)` and `get_tickets()`. `join_queue`, `get_ticket` and `get_tickets` SHALL return a fencing token alongside their result. `leave_queue` SHALL return the new fencing token. Tags SHALL be an optional ordered map of string to string; passing no tags SHALL be equivalent to passing an empty map.

#### Scenario: Ticket shape
- **WHEN** any operation returns a ticket
- **THEN** the ticket carries the process ID, a counter value, a zero-based position, and a tag map

### Requirement: Fencing token
The fencing token SHALL start at 0 for a queue that has never been written. Every successful write (a join that adds a new process, or a leave that removes one) SHALL increase the token by exactly one. Reads SHALL return the token of the state they observed without changing it.

#### Scenario: Empty queue token
- **WHEN** `get_tickets` is called on a queue that has never been written
- **THEN** it returns token 0 and an empty list

#### Scenario: Token increments per write
- **WHEN** `foo` joins, then `bar` joins, then `foo` leaves
- **THEN** the returned tokens are 1, 2 and 3 in that order

#### Scenario: Read returns the current token
- **WHEN** `foo` has joined with token 1 and `get_ticket("foo")` is called
- **THEN** it returns token 1 and a ticket equal to the one returned by the join

### Requirement: Joining a queue
`join_queue` SHALL append the process to the back of the queue, assign it the next counter value, record its tags, and return its ticket. The counter value SHALL be unique for the lifetime of the queue and SHALL never be reused even after a process leaves.

#### Scenario: First join
- **WHEN** `foo` joins an empty queue
- **THEN** its ticket has position 0

#### Scenario: Second join
- **WHEN** `bar` joins after `foo`
- **THEN** `bar`'s ticket has position 1 and a counter one greater than `foo`'s

#### Scenario: Join with tags
- **WHEN** `foo` joins with tags `{role: leader}`
- **THEN** the returned ticket carries those tags
- **AND** `get_ticket("foo")` and `get_tickets` return the same tags

### Requirement: Joining is idempotent
`join_queue` for a process ID already in the queue SHALL NOT modify the queue. It SHALL return the process's existing ticket and the current fencing token.

#### Scenario: Rejoin
- **WHEN** `foo` joins (token 1) and then joins again
- **THEN** the second call returns token 1 and a ticket equal to the first
- **AND** `get_tickets` lists `foo` once

### Requirement: Leaving a queue
`leave_queue` SHALL remove the process from the queue and return the new fencing token. Processes behind the removed one SHALL move forward, so their positions decrease by one while their counter values are unchanged.

#### Scenario: Leave shifts later positions
- **WHEN** `foo` (position 0) and `bar` (position 1) are queued and `foo` leaves
- **THEN** `get_ticket("bar")` reports position 0 and the same counter `bar` was given on join

#### Scenario: Leave a process that is not queued
- **WHEN** `leave_queue` is called for a process ID not in the queue, or on a queue that does not exist
- **THEN** it returns a not-found error and the fencing token is unchanged

### Requirement: Looking up tickets
`get_ticket` SHALL return the ticket for one process. `get_tickets` SHALL return all tickets ordered by ascending counter value, with positions equal to their index in that list.

#### Scenario: Lookup of an unknown process
- **WHEN** `get_ticket` is called for a process ID not in the queue, or on a queue that does not exist
- **THEN** it returns a not-found error

#### Scenario: List after joins
- **WHEN** `foo` then `bar` have joined
- **THEN** `get_tickets` returns `[foo, bar]` with positions 0 and 1

### Requirement: In-memory queue
The system SHALL provide an in-memory queue (`local::queue::Queue`) guarded by a mutex. Its counter SHALL start at 0, so the first process to join receives counter 0.

#### Scenario: First counter value in memory
- **WHEN** `foo` joins a fresh in-memory queue
- **THEN** its ticket has counter 0 and position 0
- **AND** the next process to join receives counter 1

### Requirement: DynamoDB queue
The system SHALL provide a DynamoDB-backed queue (`aws::queue::Queue`) addressed by a table name and an item ID. The queue SHALL be persisted as one item whose `Type` attribute is `QUEUE`, with the fencing token stored as the item `Version`, the last issued counter as `Value`, and the entries as a string set of JSON documents in `Items`. Its counter SHALL start at 1, so the first process to join receives counter 1. Writes SHALL use the optimistic-concurrency protocol defined in the dynamodb-storage capability.

#### Scenario: First counter value on DynamoDB
- **WHEN** `foo` joins a queue with no existing item
- **THEN** its ticket has counter 1 and position 0
- **AND** the next process to join receives counter 2

#### Scenario: Read when no item exists
- **WHEN** `get_tickets` is called and no item exists for the queue ID
- **THEN** it returns token 0 and an empty list without creating an item

#### Scenario: Join retries on a concurrent write
- **WHEN** `join_queue` reads version N and another process writes version N+1 first
- **THEN** the conditional put fails and the join re-reads and retries after the retry time plus jitter
- **AND** if the retried read now shows the process already queued, its existing ticket is returned without a further write

#### Scenario: Item is not a queue
- **WHEN** any queue operation reads an item whose `Type` attribute is not `QUEUE`
- **THEN** it returns an `UnrecognisedQueueType` error

#### Scenario: Entries are ordered by counter on read
- **WHEN** the `Items` string set is read back from DynamoDB in arbitrary order
- **THEN** the entries are sorted by ascending counter before positions are assigned

### Requirement: Queue removal
The DynamoDB queue SHALL provide a `remove` operation that deletes the queue's item, discarding all entries and resetting the fencing token and counter. Removal SHALL succeed whether or not the item exists.

#### Scenario: Remove then list
- **WHEN** `remove` is called on a queue with entries and `get_tickets` is then called
- **THEN** it returns token 0 and an empty list
