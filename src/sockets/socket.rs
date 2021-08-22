use std::{net::UdpSocket, sync::Arc};

use crate::engine::graph::{GraphPacketQueue, NodeId};
use std::os::unix::io::{AsRawFd, RawFd};

#[derive(Clone)]
pub struct NetworkPacket {
    pub data: Vec<u8>,
}

pub struct Socket {
    socket: UdpSocket,

    target_node_id: NodeId,
    packet_queue: Arc<GraphPacketQueue<NetworkPacket>>,
}

impl Socket {
    pub fn get_fd(&self) -> RawFd {
        self.socket.as_raw_fd()
    }

    pub fn new(
        socket: UdpSocket,
        target_node_id: NodeId,
        packet_queue: Arc<GraphPacketQueue<NetworkPacket>>,
    ) -> Self {
        socket.set_nonblocking(true).unwrap();
        Self {
            socket,
            target_node_id,
            packet_queue,
        }
    }

    pub fn read_and_push(&self) {
        let mut buf = vec![0u8;1472];
        if let Ok(bytes) = self.socket.recv(&mut buf[..]) {
            println!("Read {} bytes from socket: {}", bytes, self.get_fd());
            buf.truncate(bytes);
            let packet = NetworkPacket {
                data: buf,
            };
            self.packet_queue.push(self.target_node_id, packet);
        }
    }

    pub fn write(&self, packet: NetworkPacket) {
        let mut buf = packet.data.as_slice();
        while !buf.is_empty() {
            let bytes = self.socket.send(buf).unwrap();
            buf = &buf[bytes..];
        }
    }
}
