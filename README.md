# Silver-tribble
This is a proof of concept for a system that forwards or multicasts and mutates packets.

An example where this might be useful is an RTP mixer/translator/middlebox

## How does this work?
The idea is to have a Set of nodes that take Packets, mutate them and pass them on to none/some/multiple further nodes

```
                                                                            |--> (egress node)
(socket) ---> (GraphPacketQueue) ---> (ingress node) --> (multicast node) --|--> (egress node)
                                                                            |--> (egress node)
```

## Multithreading, locking and bottlenecks

## Graph processing
The GraphPacketQueue is processed by multiple threads that take work packets out of it and then traverse the graph
depending on how the nodes decide to forward the packets.

```
0. Take packet out of graph queue and put into local queue
1. While have more packets in local queue
    1. Take packet
    2. Lock node
    3. Call node.do_work() -> 0..x new Packets
    4. Insert new packets into local queue
    5. repeat 
```

On the surface there is a lot of locking going on there. But the usecase lets us get away with cheap spin locks because 
the only really congested nodes are the egress nodes and those do very little work, by offloading the network IO onto a separate thread pool.

## Sockets -> Graph -> Sockets
All egress nodes push to the same channel to send packets to the network IO thread pool.

Better would be to have multiple queues that get used more or less evenly, similar to how the communication towards the graph is solved.

### TODO use ringbuffer queues that overwrite old messages in favor of new messages
* ring-channel doesn't work well for this usecase because it requires &mut for send and recv
* Implementing this on top of crossbeam_queue::ArrayQueue should be relatively straight forward
* Doing my own impl of this might be fun