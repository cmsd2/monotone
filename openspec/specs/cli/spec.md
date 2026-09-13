## Purpose

The `monotone` binary is a thin command-line front end over the DynamoDB counter and queue. Each invocation performs one operation against one counter or queue row and prints a JSON document to stdout, so it can be composed with tools like `jq` from boot scripts and CI jobs. There is no in-memory mode in the CLI.

## Requirements

### Requirement: Command structure
The CLI SHALL accept global options `--region/-r`, `--table/-t` and `--id/-i`, followed by a `counter` or `queue` subcommand, followed by an operation. Counter operations SHALL be `get`, `next` and `rm`. Queue operations SHALL be `get`, `list`, `join`, `leave` and `rm`. The `queue` subcommand SHALL accept `--process/-p` naming the process ID, and `join` SHALL accept one or more `--tag/-t KEY=VALUE` options.

#### Scenario: Increment a counter
- **WHEN** the user runs `monotone -i mycounter counter next`
- **THEN** the counter `mycounter` in table `Counters` in region `eu-west-1` is incremented

#### Scenario: Join a queue with tags
- **WHEN** the user runs `monotone -i q queue -p host1 join -t role=zk -t rack=a`
- **THEN** `host1` joins queue `q` with tags `{rack: a, role: zk}`

### Requirement: Defaults
The region SHALL default to `eu-west-1` and the table SHALL default to `Counters` when the corresponding option is omitted. Credentials SHALL be resolved through the AWS default credential chain (environment variables, profile, instance metadata).

#### Scenario: Explicit region and table
- **WHEN** the user passes `-r us-east-1 -t MyTable`
- **THEN** every DynamoDB call targets `MyTable` in `us-east-1` and the output echoes those values

### Requirement: Table auto-provisioning
Before every operation, the CLI SHALL call `create_table_if_needed` with read and write capacity of 1 and then `wait_for_table`, so a first run against a fresh account creates the table and blocks until it is active.

#### Scenario: First run in a fresh account
- **WHEN** the configured table does not exist
- **THEN** the CLI creates it with 1 read and 1 write capacity unit, waits until it is `ACTIVE`, and then performs the operation

### Requirement: Counter output
`counter get` and `counter next` SHALL print a pretty-printed JSON object with fields `id`, `value`, `region` and `table`. `counter rm` SHALL delete the row and print nothing.

#### Scenario: Get output
- **WHEN** `monotone -i mycounter counter get` runs against a fresh counter
- **THEN** stdout is `{"id": "mycounter", "value": 0, "region": "eu-west-1", "table": "Counters"}` pretty-printed

#### Scenario: Next output
- **WHEN** `monotone -i mycounter counter next` runs after the get above
- **THEN** stdout shows `"value": 1`

### Requirement: Queue output
`queue get` and `queue join` SHALL print a JSON object with `id`, `region`, `table`, `fencing_token` and a `ticket` object holding `process_id`, `counter`, `position` and `tags`. `queue list` SHALL print the same envelope with a `tickets` array in place of `ticket`. `queue leave` SHALL print the envelope with only `fencing_token` and no ticket. `queue rm` SHALL delete the row and print nothing.

#### Scenario: Join output
- **WHEN** `monotone -i myqueue queue -p foo join` runs against a fresh queue
- **THEN** stdout is a pretty-printed object with `fencing_token` 1 and `ticket` `{process_id: foo, counter: 1, position: 0, tags: {}}`

#### Scenario: Leave output
- **WHEN** `monotone -i myqueue queue -p foo leave` runs after the join above
- **THEN** stdout is a pretty-printed object with `id`, `region`, `table` and `fencing_token` 2 and no `ticket` field

#### Scenario: List output
- **WHEN** `monotone -i myqueue queue list` runs
- **THEN** stdout contains a `tickets` array ordered by position, each element with `process_id`, `counter`, `position` and `tags`

#### Scenario: Extract a member ID for scripting
- **WHEN** the user pipes `queue join` output through `jq .ticket.counter`
- **THEN** a single integer is printed, suitable for writing to a file such as a Zookeeper `myid`

### Requirement: Argument validation
The `--id` option SHALL be required for every operation. The `--process` option SHALL be required for `queue get`, `queue join` and `queue leave`. Each `--tag` value SHALL contain an `=`; the part before the first `=` is the key and the remainder is the value.

#### Scenario: Missing id
- **WHEN** the user runs `monotone counter get` without `-i`
- **THEN** the CLI logs "Missing required argument id", prints the help text, and exits with status 1

#### Scenario: Missing process on join
- **WHEN** the user runs `monotone -i q queue join` without `-p`
- **THEN** the CLI logs a missing-argument error for `process`, prints help, and exits with status 1

#### Scenario: Malformed tag
- **WHEN** the user passes `-t novalue` to `queue join`
- **THEN** the CLI fails with an `invalid tag: novalue` error before contacting DynamoDB

#### Scenario: Tag with an equals sign in the value
- **WHEN** the user passes `-t url=http://a=b`
- **THEN** the tag key is `url` and the value is `http://a=b`

### Requirement: Subcommand errors
When no subcommand or an unrecognised subcommand is given at either level, the CLI SHALL log an error, print the help text, and exit with status 1.

#### Scenario: No subcommand
- **WHEN** the user runs `monotone -i x`
- **THEN** the CLI logs "No subcommand provided", prints help, and exits with status 1

### Requirement: Failure exit status
Any library, credential, region-parse or serialisation error SHALL cause the CLI to terminate with a non-zero exit status and the error printed to stderr.

#### Scenario: Not-found on get
- **WHEN** `queue -p nobody get` is run for a process that is not queued
- **THEN** the CLI exits non-zero with a ticket-not-found error on stderr

#### Scenario: Counter command on a queue row
- **WHEN** `counter get` is run with the `-i` of an existing queue
- **THEN** the CLI exits non-zero with an unrecognised-counter-type error

### Requirement: Logging
The CLI SHALL initialise `env_logger`, so diagnostic output (table state polling, retry notices) is controlled by the `RUST_LOG` environment variable and written to stderr, never mixed into the JSON on stdout.

#### Scenario: Quiet by default
- **WHEN** `RUST_LOG` is unset
- **THEN** stdout contains only the JSON result
