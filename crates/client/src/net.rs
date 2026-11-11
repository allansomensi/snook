//! Connection to the Snook server: a WebSocket client on a background
//! thread, talking to the game through channels.
//!
//! The thread runs a small single-threaded async runtime and waits on both
//! the socket and the game's outgoing queue at once, so a message is sent
//! the moment the game queues it and handed to the game the moment it
//! arrives. (An earlier version alternated between short blocking reads and
//! writes; on Windows short socket timeouts are rounded up, which delayed
//! every shot, aim update and chat line.)

use std::io::ErrorKind;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use snook_proto::{ClientMsg, PROTOCOL_VERSION, ServerMsg, parse_server};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tungstenite::Message;
use tungstenite::client::IntoClientRequest;

/// What the network thread reports to the game.
#[derive(Debug)]
pub enum NetEvent {
    Connected,
    Message(Box<ServerMsg>),
    /// The connection could not be made or was lost.
    Closed(String),
}

/// A live (or connecting) connection. Dropping it closes the socket.
pub struct Connection {
    tx: UnboundedSender<ClientMsg>,
    rx: Receiver<NetEvent>,
    closed: bool,
}

const PING_EVERY: Duration = Duration::from_secs(15);
/// The server pings back within seconds; silence this long means a dead link.
const SILENCE_LIMIT: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

impl Connection {
    /// Starts connecting to `url` (`ws://host:port/ws` or `wss://…`).
    pub fn open(url: &str) -> Connection {
        let (tx, out_rx) = unbounded_channel::<ClientMsg>();
        let (in_tx, rx) = channel::<NetEvent>();
        let url = url.trim().to_string();
        std::thread::Builder::new()
            .name("snook-net".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build();
                let reason = match runtime {
                    Ok(rt) => match rt.block_on(run(&url, out_rx, &in_tx)) {
                        Ok(()) => "Disconnected.".to_string(),
                        Err(e) => e,
                    },
                    Err(e) => format!("Network error: {e}."),
                };
                let _ = in_tx.send(NetEvent::Closed(reason));
            })
            .expect("spawn network thread");
        Connection { tx, rx, closed: false }
    }

    pub fn send(&self, msg: ClientMsg) {
        let _ = self.tx.send(msg);
    }

    /// Next event, if any (never blocks).
    pub fn poll(&mut self) -> Option<NetEvent> {
        if self.closed {
            return None;
        }
        match self.rx.try_recv() {
            Ok(e) => {
                if matches!(e, NetEvent::Closed(_)) {
                    self.closed = true;
                }
                Some(e)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.closed = true;
                Some(NetEvent::Closed("Disconnected.".into()))
            }
        }
    }
}

fn friendly(e: &tungstenite::Error) -> String {
    match e {
        tungstenite::Error::Io(io) => match io.kind() {
            ErrorKind::ConnectionRefused => {
                "The server is not answering (connection refused).".into()
            }
            ErrorKind::TimedOut | ErrorKind::WouldBlock => {
                "The server took too long to answer.".into()
            }
            _ => crate::i18n::tw("Network error: {e}.", &[("e", &io)]),
        },
        tungstenite::Error::Tls(t) => {
            crate::i18n::tw("Secure connection failed: {e}.", &[("e", &t)])
        }
        tungstenite::Error::Url(_) | tungstenite::Error::HttpFormat(_) => {
            "The server address is not valid (example: wss://snook.example.com/ws).".into()
        }
        tungstenite::Error::Http(r) => crate::i18n::tw(
            "The server refused the connection (HTTP {code}).",
            &[("code", &r.status())],
        ),
        other => crate::i18n::tw("Connection error: {e}.", &[("e", &other)]),
    }
}

async fn run(
    url: &str,
    mut out_rx: UnboundedReceiver<ClientMsg>,
    in_tx: &Sender<NetEvent>,
) -> Result<(), String> {
    let request = url.into_client_request().map_err(|e| friendly(&e))?;
    let uri = request.uri();
    let host = uri.host().ok_or("The server address has no host name.")?.to_string();
    let secure = uri.scheme_str() == Some("wss");
    let port = uri.port_u16().unwrap_or(if secure { 443 } else { 80 });
    let stream = tokio::time::timeout(
        CONNECT_TIMEOUT,
        tokio::net::TcpStream::connect((host.as_str(), port)),
    )
    .await
    .map_err(|_| "The server took too long to answer.".to_string())?
    .map_err(|e| match e.kind() {
        ErrorKind::NotFound => {
            crate::i18n::tw("Could not find the server “{host}”.", &[("host", &host)])
        }
        _ => friendly(&tungstenite::Error::Io(e)),
    })?;
    // No Nagle delay: small game messages go out at once.
    let _ = stream.set_nodelay(true);
    let (ws, _) = tokio::time::timeout(
        CONNECT_TIMEOUT,
        tokio_tungstenite::client_async_tls_with_config(request, stream, None, None),
    )
    .await
    .map_err(|_| "The server took too long to answer.".to_string())?
    .map_err(|e| friendly(&e))?;
    let (mut sink, mut source) = ws.split();
    let send_msg = |msg: &ClientMsg| Message::text(msg.to_json());
    sink.send(send_msg(&ClientMsg::Hello { version: PROTOCOL_VERSION }))
        .await
        .map_err(|e| friendly(&e))?;
    if in_tx.send(NetEvent::Connected).is_err() {
        return Ok(());
    }
    let mut ping = tokio::time::interval(PING_EVERY);
    ping.tick().await;
    let mut last_heard = tokio::time::Instant::now();
    loop {
        tokio::select! {
            out = out_rx.recv() => match out {
                Some(msg) => {
                    sink.send(send_msg(&msg)).await.map_err(|e| friendly(&e))?;
                    // Send whatever else is already queued in the same go.
                    while let Ok(more) = out_rx.try_recv() {
                        sink.send(send_msg(&more)).await.map_err(|e| friendly(&e))?;
                    }
                }
                None => {
                    // The game dropped the connection.
                    let _ = sink.send(Message::Close(None)).await;
                    return Ok(());
                }
            },
            incoming = source.next() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    last_heard = tokio::time::Instant::now();
                    match parse_server(text.as_str()) {
                        Ok(msg) => {
                            if in_tx.send(NetEvent::Message(Box::new(msg))).is_err() {
                                return Ok(());
                            }
                        }
                        Err(e) => eprintln!("ignoring malformed server message: {e}"),
                    }
                }
                Some(Ok(Message::Close(_))) | None => {
                    return Err("The server closed the connection.".into())
                }
                Some(Ok(_)) => last_heard = tokio::time::Instant::now(),
                Some(Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed)) => {
                    return Err("The server closed the connection.".into())
                }
                Some(Err(e)) => return Err(friendly(&e)),
            },
            _ = ping.tick() => {
                if last_heard.elapsed() > SILENCE_LIMIT {
                    return Err("Lost contact with the server.".into());
                }
                sink.send(send_msg(&ClientMsg::Ping)).await.map_err(|e| friendly(&e))?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn run_blocking(url: &str) -> Result<(), String> {
        let (_tx, out_rx) = unbounded_channel();
        let (in_tx, _rx) = channel();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(run(url, out_rx, &in_tx))
    }

    #[test]
    fn bad_addresses_fail_cleanly() {
        let err = run_blocking("not a url").unwrap_err();
        assert!(err.contains("wss://"), "{err}");
        let err = run_blocking("ws://127.0.0.1:1/ws").unwrap_err();
        assert!(
            ["refused", "recusada", "Network", "rede"].iter().any(|w| err.contains(w)),
            "{err}"
        );
    }

    /// Needs internet access: checks that TLS (wss://) is wired up.
    #[test]
    #[ignore]
    fn tls_handshake_reaches_http() {
        let err = run_blocking("wss://example.com/ws").unwrap_err();
        assert!(!err.contains("Secure connection failed"), "{err}");
    }

    fn wait_for(c: &mut Connection, what: &str, pred: impl Fn(&ServerMsg) -> bool) -> Instant {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match c.poll() {
                Some(NetEvent::Message(m)) if pred(&m) => return Instant::now(),
                Some(NetEvent::Closed(r)) => panic!("closed while waiting for {what}: {r}"),
                Some(_) => {}
                None => {
                    assert!(Instant::now() < deadline, "timed out waiting for {what}");
                    std::thread::yield_now();
                }
            }
        }
    }

    /// A chat line goes from one game to another through a server in
    /// milliseconds (it used to wait on socket read timeouts).
    #[test]
    fn messages_cross_the_server_at_once() {
        let host = snook_server::lan::LanHost::start("test").expect("host");
        let url = format!("ws://127.0.0.1:{}/ws", host.port);
        let mut a = Connection::open(&url);
        let mut b = Connection::open(&url);
        for (c, name) in [(&mut a, "alpha"), (&mut b, "bravo")] {
            // The hello is queued first, so the sign-in can go at once.
            c.send(ClientMsg::Guest { name: name.into() });
            wait_for(c, "auth", |m| matches!(m, ServerMsg::AuthOk { .. }));
        }
        let mut times = Vec::new();
        for k in 0..20 {
            // Chat is rate-limited: stay under it.
            std::thread::sleep(Duration::from_millis(1100));
            let text = format!("ping {k}");
            let sent = Instant::now();
            a.send(ClientMsg::Chat { text: text.clone() });
            let got = wait_for(
                &mut b,
                "chat",
                |m| matches!(m, ServerMsg::Chat { line } if line.text == text),
            );
            times.push(got - sent);
        }
        times.sort();
        let (median, worst) = (times[times.len() / 2], times[times.len() - 1]);
        println!("chat latency through the server: median {median:?}, worst {worst:?}");
        // The bug this guards against cost 1–2 s per message; shared CI
        // machines have the odd slow moment, so only the median is tight.
        assert!(median < Duration::from_millis(40), "median {median:?}");
        assert!(worst < Duration::from_millis(500), "worst {worst:?}");
    }
}
