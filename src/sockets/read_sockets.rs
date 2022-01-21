use std::collections::HashMap;
use std::net::SocketAddr;
use std::os::unix::io::RawFd;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use crate::sockets::socket::Socket;

use nix::sys::epoll;

use crossbeam_channel::{bounded, Receiver, Sender};

use crate::engine::spin_rw::SpinRwLock;
use crate::sockets::socket::NetworkPacket;

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct SocketId(usize);

pub struct SocketMaintainer {
    id_counter: AtomicUsize,

    sockets: Arc<SpinRwLock<HashMap<SocketId, Arc<Socket>>>>,
    epoll_wrapper: Arc<EpollWrapper>,
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
        let (s, r) = bounded(100);
        Self {
            id_counter: AtomicUsize::new(0),

            sockets: Arc::new(SpinRwLock::new(HashMap::new())),
            epoll_wrapper: Arc::new(EpollWrapper::new()),
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
        self.sockets.write().get().insert(id, Arc::new(socket));
        self.epoll_wrapper.add_fd(fd, id);
        id
    }

    pub fn send_packets_channel(&self) -> Sender<(SocketId, NetworkPacket)> {
        self.send_packets_sender.clone()
    }

    pub fn run_receive(&self, worker_threads: usize) {
        for _ in 0..worker_threads {
            let socks = Arc::clone(&self.sockets);
            let epoll = Arc::clone(&self.epoll_wrapper);
            std::thread::spawn(move || {
                let mut ids = [SocketId(0); 64];
                loop {
                    let ready_fds = epoll.poll(&mut ids);
                    for fd in ready_fds {
                        if let Some(sock) = socks.read().get().get(fd) {
                            sock.read_and_push();
                        }
                    }
                }
            });
        }
    }

    pub fn run_send(&self, worker_threads: usize) {
        for _ in 0..worker_threads {
            let work_recv = self.send_packets.clone();
            let socks = Arc::clone(&self.sockets);
            std::thread::spawn(move || {
                loop {
                    let (sock_id, packet) = work_recv.recv().unwrap();
                    if let Some(sock) = socks.read().get().get(&sock_id) {
                        sock.as_ref()
                            .socket
                            .connect(SocketAddr::from((
                                [127, 0, 0, 1],
                                sock.socket.local_addr().unwrap().port() + 100,
                            )))
                            .unwrap();

                        sock.write(packet);
                        //let sock = Arc::clone(sock);
                        //worker_pool
                        //    .execute(move || sock.as_ref().write(packet))
                    }
                }
            });
        }
    }
}
