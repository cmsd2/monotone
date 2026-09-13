## Purpose

The sans-IO core holds every rule about counters and queues (row arithmetic, ticket ordering, fencing tokens, conflict retry) as pure code that performs no I/O. Backends drive it by executing the effects it emits and feeding back results, so every backend shares one set of semantics and the rules can be tested exhaustively without a network, a clock or a database.

## ADDED Requirements

### Requirement: Effect-driven operations
Every counter and queue operation SHALL be expressed as a state machine that emits effects and consumes inputs. The effects SHALL be exactly: `Read` (fetch the row for the operation's ID), `Write { row, expected_version }` (store the row only if the stored version equals `expected_version`, or no row exists and `expected_version` is 0), and `Sleep(duration)`. The inputs SHALL be exactly: `Row(Option<row>)` in reply to `Read`, and `WriteOk` or `WriteConflict` in reply to `Write`. `Sleep` takes no reply beyond resuming. After the machine reports completion it SHALL emit no further effects.

#### Scenario: Read-only operation
- **WHEN** a `get_value` machine is started and fed `Row(Some(counter at value 7))`
- **THEN** it emitted exactly one `Read` and then completes with 7

#### Scenario: Read-only operation on a missing row
- **WHEN** a `get_tickets` machine is fed `Row(None)`
- **THEN** it completes with fencing token 0 and an empty list without emitting `Write`

#### Scenario: Mutating operation without contention
- **WHEN** a `next_value` machine is fed `Row(Some(value 3, version 3))` then `WriteOk`
- **THEN** the effects were `Read`, then `Write { value 4, version 4, expected_version 3 }`, then completion with 4

#### Scenario: Input does not match the pending effect
- **WHEN** a machine that just emitted `Read` is fed `WriteOk`
- **THEN** the step returns a protocol error and the machine emits nothing further

### Requirement: Conflict retry sequencing
When a `Write` is answered with `WriteConflict`, the machine SHALL emit `Sleep(retry_time + jitter)` where jitter is drawn uniformly from `[0, jitter_millis)` milliseconds, then emit `Read`, and recompute its change from the freshly read row. It SHALL repeat this without limit until a `Write` is answered with `WriteOk`. The retry time and jitter bound SHALL be configurable per machine, with a default jitter bound of 100 ms.

#### Scenario: Three conflicts then success
- **WHEN** a `next_value` machine is fed `Row(Some(v=1))`, `WriteConflict`, `Row(Some(v=2))`, `WriteConflict`, `Row(Some(v=3))`, `WriteConflict`, `Row(Some(v=4))`, `WriteOk`
- **THEN** it emitted three `Sleep` effects and four `Read` effects
- **AND** every `Write` carried `expected_version` equal to the version it had just read
- **AND** it completes with 5

#### Scenario: Retry observes a rejoin
- **WHEN** a `join_queue("foo")` machine is fed `Row(None)`, `WriteConflict`, then `Row(Some(queue where foo is already at position 0))`
- **THEN** it completes with foo's existing ticket and the read token without emitting another `Write`

#### Scenario: Retry observes a removal
- **WHEN** a `leave_queue("foo")` machine is fed `Row(Some(queue with foo))`, `WriteConflict`, then `Row(Some(queue without foo))`
- **THEN** it completes with a not-found error without emitting another `Write`

### Requirement: Deterministic jitter
The machine SHALL draw jitter from a random source supplied at construction. Two machines constructed with the same seed and fed the same inputs SHALL emit identical `Sleep` durations.

#### Scenario: Same seed, same sleeps
- **WHEN** two `next_value` machines are built with seed 42, retry 100 ms, jitter 50 ms and each fed two conflicts
- **THEN** both emit the same two `Sleep` durations, each in the range 100 ms to 149 ms inclusive

### Requirement: Pure row operations
The core SHALL expose the row transformations as pure functions independent of the effect machines: increment a counter row, join a queue row, leave a queue row, look up one ticket, list all tickets. Each SHALL be deterministic in its inputs and SHALL never perform I/O. The invariants in the counter and queue capabilities (counter never reused after leave, join is idempotent and does not bump the version, leave shifts later positions down by one, token increments once per write) SHALL hold for every reachable sequence of operations.

#### Scenario: Counter value is never reused
- **WHEN** any sequence of joins and leaves is applied to an empty queue row
- **THEN** no two tickets ever issued share a counter value
- **AND** every counter value issued is greater than all values issued before it

#### Scenario: Positions are dense after leave
- **WHEN** `a`, `b`, `c` join and then `b` leaves
- **THEN** the tickets list is `[a at 0, c at 1]` and `c`'s counter is unchanged

#### Scenario: Version bumps exactly once per write
- **WHEN** `a` joins, `a` joins again, `a` leaves, `a` leaves again (error)
- **THEN** the row version went 0, 1, 1, 2, 2

### Requirement: Row encoding
The core SHALL provide pure encode and decode between rows and the DynamoDB item shape defined in the dynamodb-storage capability. Decoding SHALL reject an item whose `Type` does not match the requested structure, whose required attributes are missing, or whose numeric attributes are not unsigned integers. Encoding then decoding any row SHALL yield an equal row.

#### Scenario: Round trip
- **WHEN** any counter row or queue row is encoded and decoded
- **THEN** the result equals the original

#### Scenario: Items are sorted on decode
- **WHEN** a queue item's `Items` set is presented in an order other than ascending counter
- **THEN** the decoded row lists entries in ascending counter order

### Requirement: Backend equivalence
Any backend that executes effects faithfully (a `Read` returns the current row, a `Write` succeeds only when the version matches, a `Sleep` merely delays) SHALL observe identical results for identical operation sequences. The in-memory backend and the DynamoDB backend SHALL both be such adapters.

#### Scenario: Same script, same results
- **WHEN** the sequence `join foo`, `join bar`, `leave foo`, `get_ticket bar`, `next_value` × 3 is run against the in-memory backend and against DynamoDB
- **THEN** the returned tokens, counters, positions and values are identical

### Requirement: Core has no I/O dependencies
Building the library with default features SHALL bring in no HTTP client, TLS stack, AWS SDK or async runtime. Those SHALL be pulled in only by the `dynamodb` feature.

#### Scenario: Default build
- **WHEN** the library is built with default features
- **THEN** the dependency graph contains no AWS SDK, hyper, or tokio crates
