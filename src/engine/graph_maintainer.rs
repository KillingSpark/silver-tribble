use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::usize;

use crate::engine::graph::AcceptWork;
use crate::engine::graph::Graph;
use crate::engine::graph::PacketQueue;

pub struct GraphMaintainer<PacketData: Send, NodeData: AcceptWork<PacketData> + Send> {
    graph: Arc<Graph<PacketData, NodeData>>,

    stop_threads: Arc<AtomicBool>,
}

impl<
        PacketData: Send + 'static,
        NodeData: AcceptWork<PacketData> + Send + 'static,
    > GraphMaintainer<PacketData, NodeData>
{
    pub fn new() -> Self {
        Self {
            graph: Arc::new(Graph::new()),
            stop_threads: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn get(&self) -> &Graph<PacketData, NodeData> {
        self.graph.as_ref()
    }

    pub fn stop_threads(&self) {
        self.stop_threads.store(true, Ordering::Relaxed)
    }

    pub fn run_threads(&self, num_threads: usize) -> Vec<JoinHandle<()>> {
        (0..num_threads)
            .into_iter()
            .map(|_| {
                let g = Arc::clone(&self.graph);
                let stop = Arc::clone(&self.stop_threads);
                std::thread::spawn(move || run_worker_thread(g, stop))
            })
            .collect()
    }
}

fn run_worker_thread<PacketData: Send, NodeData: AcceptWork<PacketData> + Send>(
    g: Arc<Graph<PacketData, NodeData>>,
    stop: Arc<AtomicBool>,
) {
    let mut further_packets = PacketQueue::new();
    loop {
        g.tick(&mut further_packets);

        if stop.load(Ordering::Relaxed) {
            break;
        }
    }
}
