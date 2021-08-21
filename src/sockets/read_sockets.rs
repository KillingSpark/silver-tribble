use std::collections::HashMap;
use std::os::unix::io::RawFd;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use crate::sockets::socket::Socket;

use nix::sys::epoll;
use threadpool::ThreadPool;

use std::sync::mpsc::{channel, Receiver, Sender};

use crate::sockets::socket::NetworkPacket;

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct SocketId(usize);

pub struct SocketMaintainer {
    id_counter: AtomicUsize,

    sockets: HashMap<SocketId, Arc<Socket>>,
    epoll_wrapper: EpollWrapper,
    worker_pool: ThreadPool,
    send_packets: Receiver<(SocketId, NetworkPacket)>,
    send_packets_sender: Sender<(SocketId, NetworkPacket)>,
}

struct EpollWrapper(RawFd);

impl EpollWrapper {
    fn new() -> Self {
        let epfd = epoll::epoll_create().unwrap();
        Self(epfd)
    }

    fn add_fd(&self, fd: RawFd, id: SocketId) {
        let mut event = epoll::EpollEvent::new(epoll::EpollFlags::EPOLLIN, id.0 as u64);
        epoll::epoll_ctl(self.0, epoll::EpollOp::EpollCtlAdd, fd, &mut event).unwrap();
    }

    fn poll<'a, const N: usize>(&self, ids: &'a mut [SocketId; N]) -> &'a mut [SocketId] {
        let mut events = [epoll::EpollEvent::new(epoll::EpollFlags::EPOLLIN, 0); N];

        let num_events = epoll::epoll_wait(self.0, &mut events[..], -1).unwrap();
        for idx in 0..num_events {
            ids[idx] = SocketId(events[idx].data() as usize)
        }
        &mut ids[0..num_events]
    }
}

impl SocketMaintainer {
    pub fn new() -> Self {
        let (s, r) = channel();
        Self {
            id_counter: AtomicUsize::new(0),

            sockets: HashMap::new(),
            epoll_wrapper: EpollWrapper::new(),
            worker_pool: ThreadPool::new(3),
            send_packets: r,
            send_packets_sender: s,
        }
    }

    fn next_id(&self) -> SocketId {
        let id = self
            .id_counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        SocketId(id)
    }

    pub fn insert(&mut self, socket: Socket) -> SocketId {
        let id = self.next_id();
        let fd = socket.get_fd();
        self.sockets.insert(id, Arc::new(socket));
        self.epoll_wrapper.add_fd(fd, id);
        id
    }

    pub fn send_packets_channel(&self) -> Sender<(SocketId, NetworkPacket)> {
        self.send_packets_sender.clone()
    }

    pub fn run_receive(&self) {
        let mut ids = [SocketId(0); 64];
        loop {
            let ready_fds = self.epoll_wrapper.poll(&mut ids);
            for fd in ready_fds {
                if let Some(sock) = self.sockets.get(fd) {
                    let sock = Arc::clone(sock);
                    self.worker_pool
                        .execute(move || sock.as_ref().read_and_push())
                }
            }
        }
    }

    pub fn run_send(&self) {
        loop {
            let (fd, packet) = self.send_packets.recv().unwrap();
            if let Some(sock) = self.sockets.get(&fd) {
                let sock = Arc::clone(sock);
                self.worker_pool
                    .execute(move || sock.as_ref().write(packet))
            }
        }
    }
}
