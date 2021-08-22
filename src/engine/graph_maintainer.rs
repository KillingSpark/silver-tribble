use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::usize;

use crate::engine::graph::AcceptWork;
use crate::engine::graph::Graph;
use crate::engine::graph::PacketQueue;

pub struct GraphMaintainer<PacketData: Send, NodeData: AcceptWork<PacketData> + Send> {
    graph: Arc<Graph<PacketData, NodeData>>,

    worker_id_counter: AtomicUsize,

    stop_threads: Arc<AtomicBool>,
}

impl<PacketData: Send + 'static, NodeData: AcceptWork<PacketData> + Send + 'static>
    GraphMaintainer<PacketData, NodeData>
{
    pub fn new() -> Self {
        Self {
            graph: Arc::new(Graph::new()),
            worker_id_counter: AtomicUsize::new(0),
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
        let x = (0..num_threads)
            .into_iter()
            .map(|_| {
                let g = Arc::clone(&self.graph);
                let stop = Arc::clone(&self.stop_threads);
                let id = self.worker_id_counter.fetch_add(1, Ordering::Relaxed);
                std::thread::spawn(move || run_worker_thread(g, id, stop))
            })
            .collect();

        self.graph
            .use_queues(self.worker_id_counter.load(Ordering::Relaxed));

        x
    }
}

fn run_worker_thread<PacketData: Send, NodeData: AcceptWork<PacketData> + Send>(
    g: Arc<Graph<PacketData, NodeData>>,
    worker_id: usize,
    stop: Arc<AtomicBool>,
) {
    let mut further_packets = PacketQueue::new();
    loop {
        g.tick(worker_id, &mut further_packets);

        if stop.load(Ordering::Relaxed) {
            break;
        }
    }
}
