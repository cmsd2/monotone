# monotone
[![Build Status](https://travis-ci.org/cmsd2/monotone.svg?branch=master)](https://travis-ci.org/cmsd2/monotone)

Counters and queues for configuration management in distributed systems

Monotone is a library and cli for maintaining atomic counters and queues.
The implementations are designed with configuration management in mind.
Note the counters are not performance counters for use in event tracking.

Two implementations are include:

1. a single-process implementation, synchronised by `Arc<Mutex<...>>`
2. a DynamoDb implementation which uses conditional updates for optimistic locking

## Documentation

[https://docs.rs/monotone](https://docs.rs/monotone)

## Building / Installing

The source repository contains two projects. The library is called monotone, and the
cli lives in a folder called cli, although it builds an executable file called monotone.

### Library

You can link to the library by adding it as a dependency to your Cargo.toml as usual.
Select the `aws` feature to bring in rusoto and use the DynamoDb backend.

The default is just the in-memory backend.

```
[dependencies]
monotone = { version = "0.4", features = ["aws"] }
```

### CLI on Laptop / Development env

Install rust. Stable rust is fine, but it should be at least 1.15.

Consider using rustup: https://www.rustup.rs

Then run `cargo install monotone` to install the version from https://crates.io
or `cargo install --path=.` to install directly from checked out source.

### CLI on CI / CD

Either install the rust toolchain on your jenkins or use a docker container like this one: https://hub.docker.com/r/jimmycuadra/rust/

Then build as you would in your dev env and copy the built artifact somewhere safe.

## Testing

The `monotone/tests` folder contains integration tests.
The `terraform` folder contains infrastructure definitions for running the integration tests. See the readme file there.

## Cli commands

Each counter or queue is stored in its own row in the table in DynamoDb.
The `-i` parameter selects which row.
The cli will prevent you running counter commands on a queue and visa versa.

### Counter

Counter is a simple atomic counter. Run like so:

```
monotone -i mycounter counter get
```

will return

```
{
  "id": "mycounter",
  "value": 0,
  "region": "eu-west-1",
  "table": "Counters"
}
```

Increment the counter like so:

```
monotone -i mycounter counter next
```

will return

```
{
  "id": "mycounter",
  "value": 1,
  "region": "eu-west-1",
  "table": "Counters"
}
```

### Queue

The queue is a list of string process IDs. Each entry in the queue is given the monotonic counter value when it joins the list.
The list is sorted in ascending order of counter value.

The queue contains a fencing token which is returned by all operations.
This will monotonically increase with every write to storage.
Use this for conditional updates in other systems to prevent acting on a stale view of the queue.

Add a process ID to the queue like so:

```
monotone -i myqueue queue -p foo join
```

which will output something like this:

```
{
  "id": "myqueue",
  "region": "eu-west-1",
  "table": "Counters",
  "fencing_token": 1,
  "ticket": {
    "process_id": "foo",
    "counter": 1,
    "position": 0
  }
}
```

You can also remove a node (for tidyness) like so:

```
monotone -i myqueue queue -p foo leave
```

which prints output:

```
{
  "id": "myqueue",
  "region": "eu-west-1",
  "table": "Counters",
  "fencing_token": 2
}
```

To list the nodes use:

```
monotone -i myqueue queue list
```

which will output something like this:

```
{
  "id": "myqueue",
  "region": "eu-west-1",
  "table": "Counters",
  "fencing_token": 2,
  "tickets": [
    {
      "process_id": "foo",
      "counter": 1,
      "position": 0
    }
  ]
}
```

## Example Usecases

### Assigning server IDs to nodes in a Zookeeper cluster

Zookeeper is a good place to store atomic counters like the ones implemented in this crate.
But what if you don't have a zookeeper cluster yet and you're trying to build one?
You have to build on something you do have, like DynamoDb.

On first boot, run the cli's queue command like so (make very sure your hostnames are unique e.g. EC2 instance IDs!):

```
monotone -i myzkcluster queue -p $(hostname -f) join | jq .ticket.counter
```

Write the resulting value to `/etc/zookeeper/conf/myid` as appropriate.

Note the zookeeper docs say the server ID must be between 0 and 255.
Monotone uses the full range of u64 integers.

### Simple leader election or lock

The list of nodes in the queue is enough to nominate a distinguished process or leader.

Just use the first process in the queue.

There are several limitations:

1. There's no liveness checking to remove failed processes from the queue
2. You must use the fencing token to ensure the queue hasn't changed while acting as leader / holding the lock.
