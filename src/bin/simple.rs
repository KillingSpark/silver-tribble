use std::sync::{atomic::AtomicUsize, Arc};

use graph_processing::engine::graph::{AcceptWork, LocalPacketQueue};
use graph_processing::engine::graph_maintainer;

fn main() {
    #[derive(Debug)]
    struct Receiver(usize, Arc<AtomicUsize>);
    struct Packet(usize);

    impl AcceptWork<Packet> for Receiver {
        fn do_work(&mut self, _packet: Packet, _further_packets: &mut LocalPacketQueue<Packet>) {
            //println!("Receiver {} got packet: {}", self.0, packet.0);
            self.1.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    let call_counter = Arc::new(AtomicUsize::new(0));

    let maintainer = graph_maintainer::GraphMaintainer::<Packet, Receiver>::new();

    println!("Made maintainer");

    let id = maintainer
        .get()
        .insert_node(Receiver(0, Arc::new(AtomicUsize::new(0))));

    println!("Inserted node");

    maintainer
        .get()
        .replace_node(id, Receiver(10, call_counter.clone()))
        .unwrap();

    println!("Updated node");

    println!("Starting worker threads");

    let handles = maintainer.run_threads(10);

    println!("Started worker threads");

    let calls = 1_000_000;
    for _ in 0..calls {
        maintainer.get().insert_work(id, Packet(10));
    }

    println!("Waiting for all calls to be made");
    loop {
        let current = call_counter.load(std::sync::atomic::Ordering::Relaxed);
        if current == calls {
            break;
        } else {
            println!("Calls: {}", current);
        }
    }
    println!("All calls went through");

    maintainer.stop_threads();

    println!("Stopping threads");

    for handle in handles.into_iter() {
        handle.join().ok();
    }

    println!("Stopped threads");
}
