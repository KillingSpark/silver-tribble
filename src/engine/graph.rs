use std::{
    collections::{HashMap, VecDeque},
    marker::PhantomData,
    sync::{atomic::AtomicUsize, Arc},
};

use arc_swap::{ArcSwap, AsRaw};

use crate::engine::spin::SpinLock;
use crate::engine::spin_rw::SpinRwLock;

pub trait AcceptWork<PacketData: Send> {
    fn do_work(&mut self, packet: PacketData, further_packets: &mut PacketQueue<PacketData>);
}

impl<PacketData: Send> AcceptWork<PacketData>
    for Box<dyn AcceptWork<PacketData> + Send>
{
    fn do_work(&mut self, packet: PacketData, further_packets: &mut PacketQueue<PacketData>) {
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

pub struct PacketQueue<PacketData: Send>(VecDeque<WorkPacket<PacketData>>);

impl<PacketData: Send> PacketQueue<PacketData> {
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

pub struct Graph<PacketData: Send, NodeData: AcceptWork<PacketData> + Send> {
    nodes: SpinRwLock<HashMap<NodeId, ArcSwap<Node<PacketData, NodeData>>>>,
    work_packets: Arc<SpinLock<PacketQueue<PacketData>>>,

    id_counter: AtomicUsize,
}

pub struct GraphPacketQueue<PacketData: Send>(Arc<SpinLock<PacketQueue<PacketData>>>);

impl<PacketData: Send> GraphPacketQueue<PacketData> {
    pub fn push(&self, node_id: NodeId, packet: PacketData) {
        self.0.as_ref().lock().get().push(node_id, packet);
    }
}

impl<PacketData: Send, NodeData: AcceptWork<PacketData> + Send> Graph<PacketData, NodeData> {
    pub fn new() -> Self {
        Self {
            nodes: SpinRwLock::new(HashMap::new()),
            work_packets: Arc::new(SpinLock::new(PacketQueue::new())),
            id_counter: AtomicUsize::new(0),
        }
    }

    pub fn packet_queue(&self) -> GraphPacketQueue<PacketData> {
        GraphPacketQueue(Arc::clone(&self.work_packets))
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

    pub fn tick(&self, further_packets: &mut PacketQueue<PacketData>) {
        if let Some(packet) = self.work_packets.lock().get().pop() {
            self.process_packet(packet, further_packets);
        }
        while let Some(packet) = further_packets.pop() {
            self.process_packet(packet, further_packets);
        }
    }

    pub fn process_packet(
        &self,
        packet: WorkPacket<PacketData>,
        further_packets: &mut PacketQueue<PacketData>,
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
        self.work_packets.lock().get().push_packet(packet)
    }

    pub fn insert_work(&self, for_node: NodeId, packet: PacketData) {
        self.insert_packet(WorkPacket {
            for_node,
            payload: packet,
        });
    }
}
