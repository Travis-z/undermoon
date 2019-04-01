![undermoon logo](docs/undermoon-logo.png)

# Undermoon [![Build Status](https://travis-ci.com/doyoubi/undermoon.svg?branch=master)](https://travis-ci.com/doyoubi/undermoon)
Aims to provide a server-side Redis proxy implementing Redis Cluster Protocol supporting multiple tenants and easy scaling.

This proxy is not limit to Redis. Any storage system implementing redis protocol can work with undermoon.

### Redis Cluster Client Protocol
[Redis Cluster](https://redis.io/topics/cluster-tutorial) is the official Redis distributed solution supporting sharding and failover.
Compared to using a single instance redis, clients connecting to `Redis Cluster` should implement the `Redis Cluster Client Protocol`.
What it basically does is:
- Do the redirection if we are requesting the wrong node.
- Caching the cluster meta data by using one of the two commands, `CLUSTER NODES` and `CLUSTER SLOTS`.

To be compatible with the existing Redis client,
there're some `Redis Cluster Proxies` to adapt the protocol,
like [corvus](https://github.com/eleme/corvus) and [aster](https://github.com/wayslog/aster).

### Server-side Proxy
##### A small bite of server-side proxy

```bash
# Run a redis-server first
$ redis-server
```

```bash
# Build and run the server_proxy
> cargo build
> target/debug/server_proxy  # runs on port 5299 and will forward commands to 127.0.0.1:6379
```

Note that how we use `AUTH [database]` command to do something like `USE [database]` in Mysql.
```bash
# Initialize Server-side Proxy
> redis-cli -p 5299
# Initialize the procy by `UMCTL` commands.
127.0.0.1:5299> UMCTL SETDB 1 NOFLAGS mydb 127.0.0.1:6379 0-8000
127.0.0.1:5299> UMCTL SETPEER 1 NOFLAGS mydb 127.0.0.1:7000 8001-16383

# Now we still didn't select our database `mydb` we just set.
# Then we are in the default `admin` database.
# But nothing was set for `admin` yet so we get a error.
127.0.0.1:5299> get a
(error) db not found: admin

# Now choose to use our `mydb`.
127.0.0.1:5299> AUTH mydb
OK

# Done! We can use it like a Redis Cluster!
127.0.0.1:5299> CLUSTER NODES
mydb________________127.0.0.1:5299______ 127.0.0.1:5299 master - 0 0 1 connected 0-8000
mydb________________127.0.0.1:7000______ 127.0.0.1:7000 master - 0 0 1 connected 8001-16383
127.0.0.1:5299> get a
(error) MOVED 15495 127.0.0.1:7000
127.0.0.1:5299> set b 1
OK
```

##### Why another "Redis Cluster"?
When I was in the last company, we maintained over 1000 Redis Clusters with different sizes from several nodes to hundreds of nodes.
We spent most of the time building a complex deployment system dedicated for Redis Cluster to support:
- Fast deployment
- Serving different clusters for different services
- Spreading the flood as evenly as possible to all the physical machines

The cost of maintaining this kind of deployment system is high
and we still can't scale the clusters fast.

I build this project to test whether a server-side approach
makes easier maintenance of both small and large, just several and a great amount of redis clusters.

##### Why server-side proxy?
A server-side proxy is able to migrate the data and scale in a super fast way
by using the `Redis Replication Protocol`
and make it possible to have a better control of full sync in replication.

## Quick Tour Examples
Requirements:

- docker-compose
- redis-cli

Before run any example, run this command to build the basic `undermoon` docker image:
```bash
$ make docker-build-image
```

### (1) Multi-process Redis
This example will run a redis proxy with multiple redis processes behind the proxy. You can use it as a "multi-threaded" redis.

```bash
$ make docker-multi-redis
```

Then you can connect to redis:

```bash
# Note that we need to add '-a' to select our database.
$ redis-cli -a mydb -h 127.0.0.1 -p 5299 SET key value
```

### (2) Redis Cluster
This example will run a sharding redis cluster with similar protocol as the [official Redis Cluster](https://redis.io/topics/cluster-tutorial).

```bash
$ make docker-multi-redis
```

You also need to add the following records to the `/etc/hosts` to support the redirection.

```
# /etc/hosts
127.0.0.1 server_proxy1
127.0.0.1 server_proxy2
127.0.0.1 server_proxy3
```

Now you have a Redis Cluster with 3 nodes!

- server_proxy1:6001
- server_proxy2:6002
- server_proxy3:6003

Connect to any node above and enable cluster mode by adding '-c':
```bash
$ redis-cli -h server_proxy1 -p 6001 -a mydb -c

Warning: Using a password with '-a' option on the command line interface may not be safe.
127.0.0.1:6001> set a 1
-> Redirected to slot [15495] located at server_proxy3:6003
OK
server_proxy3:6003> set b 2
-> Redirected to slot [3300] located at server_proxy1:6001
OK
server_proxy1:6001> set c 3
-> Redirected to slot [7365] located at server_proxy2:6002
OK
server_proxy2:6002> cluster nodes
mydb________________server_proxy2:6002__ server_proxy2:6002 master - 0 0 1 connected 5462-10922
mydb________________server_proxy1:6001__ server_proxy1:6001 master - 0 0 1 connected 0-5461
mydb________________server_proxy3:6003__ server_proxy3:6003 master - 0 0 1 connected 10923-16383
```

### (3) Failover
The server-side proxy itself does not support failure detection and failover.
But this can be easy done by using the two meta data management commands:

- UMCTL SETDB
- UMCTL SETPEER

See the api docs later for details.
Here we will use a simple Python script utilizing these two commands to do the failover for us.
This script locates in `examples/failover/checker.py`.
Now lets run it with the Redis Cluster in section (2):

```bash
$ make docker-failover
```

Connect to `server_proxy2`.

```bash
$ redis-cli -h server_proxy2 -p 6002 -a mydb
Warning: Using a password with '-a' option on the command line interface may not be safe.
server_proxy2:6002> cluster nodes
mydb________________server_proxy2:6002__ server_proxy2:6002 master - 0 0 1 connected 5462-10922
mydb________________server_proxy1:6001__ server_proxy1:6001 master - 0 0 1 connected 0-5461
mydb________________server_proxy3:6003__ server_proxy3:6003 master - 0 0 1 connected 10923-16383
server_proxy2:6002> get b
(error) MOVED 3300 server_proxy1:6001
```

`server_proxy1` is responsible for key `b`. Now kill the `server_proxy1`:

```bash
$ docker ps | grep server_proxy1 | awk '{print $1}' | xargs docker kill
```

5 seconds later, slots from `server_proxy1` was transferred to `server_proxy2`!

```bash
$ redis-cli -h server_proxy2 -p 6002 -a mydb
Warning: Using a password with '-a' option on the command line interface may not be safe.
server_proxy2:6002> cluster nodes
mydb________________server_proxy2:6002__ server_proxy2:6002 master - 0 0 1 connected 5462-10922 0-5461
mydb________________server_proxy3:6003__ server_proxy3:6003 master - 0 0 1 connected 10923-16383
server_proxy2:6002> get b
(nil)
```

But what if the `checker.py` fails?
What if we have multiple `checker.py` running, but they have different views about the liveness of nodes?

Well, this checker script is only for those who just want a simple solution.
For serious distributed systems, we should use some other robust solution.
This is where the `Coordinator` comes in.

### (4) Coordinator and HTTP Broker
Undermoon also provides a `Coordinator` to do something similar to the `checker.py` above in example (3).
It will keep checking the server-side proxies and do failover.
However, `Coordinator` itself does not store any data. It is a stateless service backed by a HTTP broker maintaining the meta data.
You can implement this HTTP broker yourself and there's a working in progress [Golang implementation](https://github.com/doyoubi/overmoon).

![architecture](docs/architecture.svg)

## API
### Server-side Proxy API
### HTTP Broker API

## Current Status
This project is now only for demonstration and has **NOT** been well tested in production environment yet.

There're two big features needed:
- Replication for better failover.
- The HTTP broker project - overmoon.

## TODO
- ~~Basic proxy implementation~~ (done)
- ~~Multiple backend connections~~ (done)
- ~~Slot map and cluster map~~ (done)
- ~~Implement AUTH command to select database~~ (done)
- ~~Implement meta data manipulation api~~ (done)
- Implement coordinator (in progress)
- Implement Redis Replication Protocol
- Support slot migration via replication
- ~~Optimize RESP parsing~~ (done)
- Implement CLUSTER SLOTS
- Implement commands to get proxy meta
- Track spawned futures
- Simple script to push configuration to proxy for demonstration
- Batch write operations with interval flushing
- ~~Add configuration~~ (done)
- Limit running commands, connections
- Slow log
- Statistics
- Support multi-key commands
