use std::{collections::{HashMap, VecDeque}, marker::PhantomData, ops::Add, sync::{atomic::AtomicUsize, Arc}};

use arc_swap::{ArcSwap, AsRaw};

use crate::engine::spin::SpinLock;
use crate::engine::spin_rw::SpinRwLock;

pub trait AcceptWork<PacketData: Send> {
    fn do_work(&mut self, packet: PacketData, further_packets: &mut LocalPacketQueue<PacketData>);
}

impl<PacketData: Send> AcceptWork<PacketData> for Box<dyn AcceptWork<PacketData> + Send> {
    fn do_work(&mut self, packet: PacketData, further_packets: &mut LocalPacketQueue<PacketData>) {
        let this: &mut dyn AcceptWork<_> = self.as_mut();
        this.do_work(packet, further_packets)
    }
}

pub struct Node<PacketData: Send, T: AcceptWork<PacketData> + Send> {
    inner: SpinLock<T>,
    _marker: PhantomData<PacketData>,
}

impl<PacketData: Send, NodeData: AcceptWork<PacketData> + Send> Node<PacketData, NodeData> {
    pub fn new(data: NodeData) -> Self {
        Self {
            inner: SpinLock::new(data),
            _marker: PhantomData {},
        }
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct NodeId(usize);

impl NodeId {
    pub fn new(value: usize) -> Self {
        Self(value)
    }
}

pub struct WorkPacket<T: Send> {
    for_node: NodeId,
    payload: T,
}

/// To be used by one thread to ba able to hold packets that are produced by a node without going through the more expensive
/// threadsafe queues.
pub struct LocalPacketQueue<PacketData: Send>(VecDeque<WorkPacket<PacketData>>);

impl<PacketData: Send> LocalPacketQueue<PacketData> {
    pub fn new() -> Self {
        Self(VecDeque::new())
    }

    pub fn push(&mut self, to_node: NodeId, packet: PacketData) {
        self.push_packet(WorkPacket {
            for_node: to_node,
            payload: packet,
        })
    }

    fn push_packet(&mut self, packet: WorkPacket<PacketData>) {
        self.0.push_back(packet)
    }

    pub fn pop(&mut self) -> Option<WorkPacket<PacketData>> {
        self.0.pop_front()
    }
}

pub struct GraphPacketQueue<PacketData: Send> {
    queues: SpinRwLock<
        Vec<(
            Arc<crossbeam_channel::Sender<WorkPacket<PacketData>>>,
            Arc<crossbeam_channel::Receiver<WorkPacket<PacketData>>>,
        )>,
    >,
}

impl<PacketData: Send> GraphPacketQueue<PacketData> {
    /// Put packets for the same node in the same queue so a node is likely to be processed by the same thread
    pub fn push(&self, node_id: NodeId, packet: PacketData) {
        self.push_packet(WorkPacket {
            for_node: node_id,
            payload: packet,
        })
    }

    fn push_packet(&self, packet: WorkPacket<PacketData>) {
        let queues = self.queues.read();
        let idx = packet.for_node.0 % queues.get().len();
        queues.get()[idx].0.send(packet).unwrap();
    }

    pub fn pop(&self, worker_id: usize) -> Option<WorkPacket<PacketData>> {
        let queue = {
            let queues = self.queues.read();
            // start looking in "own" queue but take packets from other queues if nothing is left in "own" queue
            let start_idx = worker_id % queues.get().len();
            let queue = Arc::clone(&queues.get()[start_idx].1);
            std::mem::drop(queues);
            queue
        };
        queue.recv_deadline(std::time::Instant::now().add(std::time::Duration::from_millis(10))).ok()
    }

    pub fn try_pop(&self, worker_id: usize) -> Option<WorkPacket<PacketData>> {
        let queues = self.queues.read();

        // start looking in "own" queue but take packets from other queues if nothing is left in "own" queue
        let start_idx = worker_id % queues.get().len();

        for idx in 0..queues.get().len() {
            let idx = idx.wrapping_add(start_idx) % queues.get().len();
            if let Some(packet) = queues.get()[idx].1.try_recv().ok() {
                return Some(packet);
            }
        }
        None
    }

    pub fn new() -> Self {
        let (s, r) = crossbeam_channel::bounded(100);
        Self {
            queues: SpinRwLock::new(vec![(Arc::new(s), Arc::new(r))]),
        }
    }
}

pub struct Graph<PacketData: Send, NodeData: AcceptWork<PacketData> + Send> {
    nodes: SpinRwLock<HashMap<NodeId, ArcSwap<Node<PacketData, NodeData>>>>,

    work_packets: Arc<GraphPacketQueue<PacketData>>,

    id_counter: AtomicUsize,
}

impl<PacketData: Send, NodeData: AcceptWork<PacketData> + Send> Graph<PacketData, NodeData> {
    pub fn new() -> Self {
        Self {
            nodes: SpinRwLock::new(HashMap::new()),
            work_packets: Arc::new(GraphPacketQueue::new()),
            id_counter: AtomicUsize::new(0),
        }
    }

    pub fn packet_queue(&self) -> Arc<GraphPacketQueue<PacketData>> {
        Arc::clone(&self.work_packets)
    }

    fn next_id(&self) -> NodeId {
        let id = self
            .id_counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        NodeId::new(id)
    }

    pub fn insert_node(&self, data: NodeData) -> NodeId {
        let id = self.next_id();
        self.nodes
            .write()
            .get()
            .insert(id, ArcSwap::new(Arc::new(Node::new(data))));
        id
    }

    pub fn update_node(&self, node_id: NodeId, data: NodeData) {
        if let Some(node) = self.nodes.read().get().get(&node_id) {
            let new = Arc::new(Node::new(data));
            loop {
                let current = node.load().as_raw();
                let maybe_current = node.compare_and_swap(current, new.clone());
                if current == maybe_current.as_raw() {
                    break;
                }
            }
        }
    }

    pub fn delete_node(&self, node_id: NodeId) {
        self.nodes.write().get().remove(&node_id);
    }

    pub fn use_queues(&self, want_queues: usize) {
        let queues = self.work_packets.queues.write();
        if queues.get().len() < want_queues {
            let new_queues = want_queues - queues.get().len();
            (0..new_queues).into_iter().for_each(|_| {
                let (s, r) = crossbeam_channel::bounded(100);
                queues.get().push((Arc::new(s), Arc::new(r)));
            })
        }
    }

    pub fn tick_blocking(&self, worker_id: usize, further_packets: &mut LocalPacketQueue<PacketData>) -> bool {
        if let Some(packet) = self.work_packets.try_pop(worker_id) {
            self.process_packet(packet, further_packets);
            while let Some(packet) = further_packets.pop() {
                self.process_packet(packet, further_packets);
            }
            true
        } else {
            false
        }
    }

    pub fn try_tick(
        &self,
        worker_id: usize,
        further_packets: &mut LocalPacketQueue<PacketData>,
    ) -> bool {
        if let Some(packet) = self.work_packets.pop(worker_id) {
            self.process_packet(packet, further_packets);
            while let Some(packet) = further_packets.pop() {
                self.process_packet(packet, further_packets);
            }
            true
        } else {
            false
        }
    }

    pub fn process_packet(
        &self,
        packet: WorkPacket<PacketData>,
        further_packets: &mut LocalPacketQueue<PacketData>,
    ) {
        let nodes_guard = self.nodes.read();
        let node = if let Some(node) = nodes_guard.get().get(&packet.for_node) {
            node.load()
        } else {
            // Drop packets for nodes that do not exist on the floor
            return;
        };

        // Explicitly drop before we do anything other.
        // We should to keep the time nodes is locked to a mininum, even if the readers don't hinder each other
        std::mem::drop(nodes_guard);

        if let Some(node) = node.as_ref().inner.try_lock() {
            node.get().do_work(packet.payload, further_packets);
        } else {
            // if we failed to lock this node, we will try again later
            self.insert_packet(packet);
        };
    }

    fn insert_packet(&self, packet: WorkPacket<PacketData>) {
        // TODO signaling
        self.work_packets.as_ref().push_packet(packet)
    }

    pub fn insert_work(&self, for_node: NodeId, packet: PacketData) {
        self.insert_packet(WorkPacket {
            for_node,
            payload: packet,
        });
    }
}
