//! Local network games: the game hosts a server (in memory, guests only)
//! and announces it with a UDP broadcast so others on the network find it.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use snook_proto::{LAN_DISCOVERY_PORT, LAN_GAME_PORT, LanBeacon, PROTOCOL_VERSION};

use crate::lounge::Rules;
use crate::{Config, Server};

/// A LAN server running in the background. Dropping it stops the server.
pub struct LanHost {
    pub port: u16,
    stop: Arc<AtomicBool>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
}

impl LanHost {
    /// Starts a LAN server named after `host` on [`LAN_GAME_PORT`] (or any
    /// free port if that one is taken).
    pub fn start(host: &str) -> std::io::Result<LanHost> {
        let std_listener = std::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, LAN_GAME_PORT))
            .or_else(|_| std::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)))?;
        std_listener.set_nonblocking(true)?;
        let port = std_listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("snook-lan")
            .build()?;
        let rules = Rules {
            lan: true,
            motd: format!("LAN game hosted by {host}."),
            max_connections_per_ip: 64,
            reconnect_grace: Duration::from_secs(90),
            ..Rules::default()
        };
        std::thread::Builder::new().name("snook-lan".into()).spawn(move || {
            runtime.block_on(async move {
                let config = Config {
                    bind: SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)),
                    rules,
                    ..Config::default()
                };
                let Ok(server) = Server::new(config).await else { return };
                let Ok(listener) = tokio::net::TcpListener::from_std(std_listener) else { return };
                tokio::select! {
                    _ = crate::serve(listener, server) => {}
                    _ = rx => {}
                }
            });
        })?;
        // The beacon.
        let beacon_stop = stop.clone();
        let name = host.to_string();
        std::thread::Builder::new().name("snook-lan-beacon".into()).spawn(move || {
            let Ok(sock) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) else { return };
            let _ = sock.set_broadcast(true);
            let beacon = LanBeacon {
                game: "snook".into(),
                version: PROTOCOL_VERSION,
                host: name,
                port,
                players: 0,
            };
            let payload = serde_json::to_vec(&beacon).unwrap_or_default();
            while !beacon_stop.load(Ordering::Relaxed) {
                let _ = sock.send_to(&payload, (Ipv4Addr::BROADCAST, LAN_DISCOVERY_PORT));
                std::thread::sleep(Duration::from_millis(1000));
            }
        })?;
        Ok(LanHost { port, stop, shutdown: Some(tx) })
    }
}

impl Drop for LanHost {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

/// A LAN game someone is hosting.
#[derive(Debug, Clone, PartialEq)]
pub struct FoundGame {
    pub host: String,
    pub address: SocketAddr,
    pub seen: Instant,
}

impl FoundGame {
    pub fn url(&self) -> String {
        format!("ws://{}/ws", self.address)
    }
}

/// Listens for LAN beacons (non-blocking; call [`Discovery::poll`] each frame).
pub struct Discovery {
    socket: Option<UdpSocket>,
    pub games: Vec<FoundGame>,
}

impl Discovery {
    pub fn new() -> Discovery {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, LAN_DISCOVERY_PORT)).ok();
        if let Some(s) = &socket {
            let _ = s.set_nonblocking(true);
        }
        Discovery { socket, games: Vec::new() }
    }

    /// False if another program already listens on the discovery port.
    pub fn listening(&self) -> bool {
        self.socket.is_some()
    }

    pub fn poll(&mut self) {
        let Some(sock) = &self.socket else { return };
        let mut buf = [0u8; 1024];
        while let Ok((n, from)) = sock.recv_from(&mut buf) {
            let Ok(b) = serde_json::from_slice::<LanBeacon>(&buf[..n]) else { continue };
            if b.game != "snook" || b.version != PROTOCOL_VERSION {
                continue;
            }
            let address = SocketAddr::new(from.ip(), b.port);
            let host: String = b.host.chars().take(24).collect();
            match self.games.iter_mut().find(|g| g.address == address) {
                Some(g) => {
                    g.seen = Instant::now();
                    g.host = host;
                }
                None => self.games.push(FoundGame { host, address, seen: Instant::now() }),
            }
        }
        self.games.retain(|g| g.seen.elapsed() < Duration::from_secs(5));
    }
}

impl Default for Discovery {
    fn default() -> Self {
        Self::new()
    }
}
