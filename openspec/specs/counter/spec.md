## Purpose

A monotonic counter is a named unsigned 64-bit integer that only ever increases. Callers read the current value or atomically increment it and receive the new value. The library exposes one trait, `MonotonicCounter`, with two implementations: an in-memory counter guarded by a mutex for a single process, and a DynamoDB-backed counter for coordination across processes. Counters are intended for configuration management (assigning IDs, versions, fencing tokens), not for high-rate event tracking.

## Requirements

### Requirement: Counter contract
The system SHALL expose a `MonotonicCounter` trait with `get_value` returning the current value and `next_value` atomically incrementing the counter by one and returning the new value. Values SHALL be unsigned 64-bit integers. Both operations SHALL return a backend-specific error type on failure.

#### Scenario: Read a fresh counter
- **WHEN** `get_value` is called on a counter that has never been incremented
- **THEN** it returns 0

#### Scenario: Increment returns the new value
- **WHEN** `next_value` is called on a counter whose value is 0
- **THEN** it returns 1
- **AND** a subsequent `get_value` returns 1

#### Scenario: Repeated increments
- **WHEN** `next_value` is called twice on a fresh counter
- **THEN** the calls return 1 then 2
- **AND** `get_value` then returns 2

### Requirement: In-memory counter
The system SHALL provide an in-memory counter (`local::counter::Counter`) that starts at 0 and serialises access through a shared mutex. Clones of the counter SHALL share the same underlying value.

#### Scenario: Concurrent increments in one process
- **WHEN** several threads call `next_value` on clones of the same counter
- **THEN** every returned value is distinct
- **AND** the final `get_value` equals the number of increments

### Requirement: DynamoDB counter
The system SHALL provide a DynamoDB-backed counter (`aws::counter::Counter`) addressed by a table name and an item ID. The counter SHALL be persisted as one item whose `Type` attribute is `COUNTER`. Increments SHALL use the optimistic-concurrency protocol defined in the dynamodb-storage capability, so that concurrent increments from separate processes never return the same value.

#### Scenario: Read when no item exists
- **WHEN** `get_value` is called and there is no item for the counter ID
- **THEN** it returns 0 without creating an item

#### Scenario: First increment creates the item
- **WHEN** `next_value` is called and there is no item for the counter ID
- **THEN** an item is written with `Value` 1 and `Version` 1
- **AND** the call returns 1

#### Scenario: Increment retries on a concurrent write
- **WHEN** `next_value` reads version N and another process writes version N+1 before this process's conditional put
- **THEN** the conditional put fails
- **AND** the counter sleeps for the retry time plus jitter, re-reads, and tries again until a put succeeds
- **AND** the returned value is one greater than the value it last read

#### Scenario: Item is not a counter
- **WHEN** `get_value` or `next_value` reads an item whose `Type` attribute is not `COUNTER`
- **THEN** it returns an `UnrecognisedCounterType` error

#### Scenario: Item is missing required attributes
- **WHEN** the item lacks any of `ID`, `Type`, `Version` or `Value`, or a numeric attribute does not parse as an unsigned integer
- **THEN** the operation returns a `MissingAttribute` or parse error

### Requirement: Counter removal
The DynamoDB counter SHALL provide a `remove` operation that deletes the counter's item. Removal SHALL succeed whether or not the item exists.

#### Scenario: Remove then read
- **WHEN** `remove` is called on a counter with value 5 and `get_value` is then called
- **THEN** `get_value` returns 0

#### Scenario: Remove a missing counter
- **WHEN** `remove` is called and no item exists
- **THEN** the call succeeds
