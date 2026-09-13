## ADDED Requirements

### Requirement: Version and program name
The CLI SHALL identify itself as `monotone` in help output and SHALL print `monotone <version>` for `--version`, where the version equals the CLI crate's manifest version.

#### Scenario: Version flag
- **WHEN** the user runs `monotone --version`
- **THEN** stdout is `monotone 0.5.0` for the 0.5.0 release and the exit status is 0

#### Scenario: Help flag
- **WHEN** the user runs `monotone --help`
- **THEN** the output names the program `monotone`, lists the `counter` and `queue` subcommands, and exits 0

## MODIFIED Requirements

### Requirement: Command structure
The CLI SHALL accept global options `--region/-r`, `--table/-t` and `--id/-i`, followed by a `counter` or `queue` subcommand, followed by an operation. Counter operations SHALL be `get`, `next` and `rm`. Queue operations SHALL be `get`, `list`, `join`, `leave` and `rm`. The `queue` subcommand SHALL accept `--process/-p` naming the process ID, and `join` SHALL accept one or more `--tag KEY=VALUE` options. `--tag` SHALL have no short form, so `-t` always means `--table`.

#### Scenario: Increment a counter
- **WHEN** the user runs `monotone -i mycounter counter next`
- **THEN** the counter `mycounter` in table `Counters` in region `eu-west-1` is incremented

#### Scenario: Join a queue with tags
- **WHEN** the user runs `monotone -i q queue -p host1 join --tag role=zk --tag rack=a`
- **THEN** `host1` joins queue `q` with tags `{rack: a, role: zk}`

### Requirement: Defaults
The region SHALL default to `eu-west-1` and the table SHALL default to `Counters` when the corresponding option is omitted. Credentials, region overrides and endpoint overrides SHALL be resolved through the AWS default configuration chain, so `AWS_ENDPOINT_URL` redirects the CLI to a DynamoDB Local instance.

#### Scenario: Explicit region and table
- **WHEN** the user passes `-r us-east-1 -t MyTable`
- **THEN** every DynamoDB call targets `MyTable` in `us-east-1` and the output echoes those values

#### Scenario: Local endpoint
- **WHEN** `AWS_ENDPOINT_URL=http://localhost:8000` is set and the user runs any command
- **THEN** all calls go to the local endpoint and the output still echoes the configured region and table

### Requirement: Failure exit status
Any library, credential, configuration or serialisation error SHALL cause the CLI to print a single-line error message to stderr and exit with status 1. The CLI SHALL NOT panic or print a backtrace for expected failures.

#### Scenario: Not-found on get
- **WHEN** `queue -p nobody get` is run for a process that is not queued
- **THEN** the CLI exits with status 1 and stderr contains `ticket not found for process_id nobody`

#### Scenario: Counter command on a queue row
- **WHEN** `counter get` is run with the `-i` of an existing queue
- **THEN** the CLI exits with status 1 and stderr names the unrecognised structure type

#### Scenario: Unreachable endpoint
- **WHEN** `AWS_ENDPOINT_URL` points at a closed port
- **THEN** the CLI exits with status 1 with a connection error on stderr and nothing on stdout

### Requirement: Logging
The CLI SHALL emit diagnostic output (table state polling, retry notices) through a subscriber controlled by the `RUST_LOG` environment variable and written to stderr, never mixed into the JSON on stdout.

#### Scenario: Quiet by default
- **WHEN** `RUST_LOG` is unset
- **THEN** stdout contains only the JSON result and stderr is empty on success

#### Scenario: Verbose
- **WHEN** `RUST_LOG=debug` is set
- **THEN** diagnostic lines appear on stderr and stdout still contains only the JSON result
