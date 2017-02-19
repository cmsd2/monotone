# monotone
Counters and queues for configuration management in distributed systems

Monotone is a library and cli for maintaining atomic counters and queues.
The implementations are designed with configuration management in mind.
Note the counters are not performance counters for use in event tracking.

Two implementations are include:
1. a single-process implementation, synchronised by `Arc<Mutex<...>>`
2. a DynamoDb implementation which uses conditional updates for optimistic locking

## Building / Installing

### Laptop / Development env

Install rust. Stable rust is fine, but it should be at least 1.13.

Consider using rustup: https://www.rustup.rs

Then run `cargo install monotone` to install the version from https://crates.io
or `cargo install .` to build from checked out source.

### CI / CD

Either install the rust toolchain on your jenkins or use a docker container like this one: https://hub.docker.com/r/jimmycuadra/rust/

Then build as you would in your dev env and copy the built artifact somewhere safe.
 
## Cli commands

Each counter or queue is stored in its own row in the table in DynamoDb.
The `-i` parameter selects which row.
The cli will prevent you running counter commands on a queue and visa versa.

### Counter

Counter is a simple atomic counter. Run like so:

```
monotone_cli -i mycounter counter get
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
monotone_cli -i mycounter counter next
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

Add a process ID to the queue like so:

```
monotone_cli -i myqueue queue -p foo join
```

which will output something like this:

```
{
  "id": "myqueue",
  "process_id": "foo",
  "counter": 1,
  "position": 0,
  "region": "eu-west-1",
  "table": "Counters"
}
```

You can also remove a node (for tidyness) like so:

```
monotone_cli -i myqueue queue -p foo leave
```

To list the nodes use:

```
monotone_cli -i myqueue queue list
```

which will output something like this:

```
[{
  "id": "myqueue",
  "process_id": "foo",
  "counter": 1,
  "position": 0,
  "region": "eu-west-1",
  "table": "Counters"
}]
```

## Example Usecases

### Assigning server IDs to nodes in a Zookeeper cluster

Zookeeper is a good place to store atomic counters like the ones implemented in this crate.
But what if you don't have a zookeeper cluster yet and you're trying to build one?
You have to build on something you do have, like DynamoDb.

On first boot, run the cli's queue command like so (make very sure your hostnames are unique e.g. EC2 instance IDs!):

```
monotone_cli -i myzkcluster queue -p $(hostname -f) join
```

which will output something like this:

```
{
  "id": "myzkcluster",
  "process_id": "foo",
  "counter": 1,
  "position": 0,
  "region": "eu-west-1",
  "table": "Counters"
}
```

Write the value of the counter field to `/etc/zookeeper/conf/myid` as appropriate.

Note the zookeeper docs say the server ID must be between 0 and 255.
Monotone uses the full range of u64 integers.
