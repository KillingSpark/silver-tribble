use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};

use graph_processing::engine::graph::{AcceptWork, NodeId, PacketQueue};
use graph_processing::engine::graph_maintainer::{self, GraphMaintainer};

use graph_processing::sockets::read_sockets::{SocketId, SocketMaintainer};
use graph_processing::sockets::socket::{NetworkPacket, Socket};

struct LoggingNode {
    next_node: NodeId,
}

impl AcceptWork<NetworkPacket> for LoggingNode {
    fn do_work(&mut self, packet: NetworkPacket, further_packets: &mut PacketQueue<NetworkPacket>) {
        eprintln!("Received packet of size: {}", packet.data.len());
        further_packets.push(self.next_node, packet)
    }
}

struct DropNode {}

impl AcceptWork<NetworkPacket> for DropNode {
    fn do_work(
        &mut self,
        _packet: NetworkPacket,
        _further_packets: &mut PacketQueue<NetworkPacket>,
    ) {
        eprintln!("Drop packet");
    }
}

struct MulticastNode {
    next_nodes: Vec<NodeId>,
}

impl AcceptWork<NetworkPacket> for MulticastNode {
    fn do_work(&mut self, packet: NetworkPacket, further_packets: &mut PacketQueue<NetworkPacket>) {
        eprintln!("Multicast packet to: {:?}", self.next_nodes);
        for next_node in self.next_nodes.iter() {
            further_packets.push(*next_node, packet.clone())
        }
    }
}

struct EgressNode {
    socket_id: SocketId,
    channel: std::sync::mpsc::Sender<(SocketId, NetworkPacket)>,
}

impl AcceptWork<NetworkPacket> for EgressNode {
    fn do_work(
        &mut self,
        packet: NetworkPacket,
        _further_packets: &mut PacketQueue<NetworkPacket>,
    ) {
        eprintln!("Sending packet of size: {}", packet.data.len());
        self.channel.send((self.socket_id, packet)).unwrap();
    }
}

type NodeType = Box<dyn AcceptWork<NetworkPacket> + Send>;
type GM = GraphMaintainer<NetworkPacket, NodeType>;

struct Server {
    graph: GM,
    sockets: SocketMaintainer,

    port_to_ingress_node: HashMap<u16, NodeId>,
    port_to_egress_node: HashMap<u16, NodeId>,
}

impl Server {
    fn new(graph: GM, sockets: SocketMaintainer) -> Self {
        Self {
            graph,
            sockets,
            port_to_ingress_node: HashMap::new(),
            port_to_egress_node: HashMap::new(),
        }
    }

    fn add_socket(&mut self, port: u16) {
        let socket = UdpSocket::bind(SocketAddr::from(([127, 0, 0, 1], port)))
            .expect("Couldn't bind to address");

        let ingress_id = self.graph.get().insert_node(Box::new(DropNode {}));

        // Don't need that id anymore
        let logging_id = self.graph.get().insert_node(Box::new(LoggingNode {
            next_node: ingress_id,
        }));

        let socket = Socket::new(socket, logging_id, self.graph.get().packet_queue());
        let sock_id = self.sockets.insert(socket);

        let egress_id = self.graph.get().insert_node(Box::new(EgressNode {
            socket_id: sock_id,
            channel: self.sockets.send_packets_channel(),
        }));

        self.graph.get().update_node(
            ingress_id,
            Box::new(MulticastNode {
                next_nodes: vec![egress_id],
            }),
        );

        self.port_to_ingress_node.insert(port, ingress_id);
        self.port_to_egress_node.insert(port, egress_id);
    }

    fn run_sockets(&mut self) {
        self.sockets.run_receive();
    }
}

fn main() {
    let g = graph_maintainer::GraphMaintainer::new();
    g.run_threads(10);

    let mut server = Server::new(g, SocketMaintainer::new());

    for port in 3400..3500 {
        server.add_socket(port);
    }

    send_packets();

    server.run_sockets();
}

fn send_packets() {
    std::thread::spawn(move || {
        let mut socks = vec![];
        for port in 3500..3501 {
            let socket = UdpSocket::bind(SocketAddr::from(([127, 0, 0, 1], port)))
                .expect("Couldn't bind to address");
            socket
                .connect(SocketAddr::from(([127, 0, 0, 1], port - 100)))
                .unwrap();
            socks.push(socket);
        }
        loop {
            for sock in &socks {
                sock.send("THIS IS A MESSAGE!".as_bytes()).unwrap();
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    });
}
