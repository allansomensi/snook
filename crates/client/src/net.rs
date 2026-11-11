//! Connection to the Snook server: a blocking WebSocket client running on a
//! background thread, talking to the game through channels.

use std::io::ErrorKind;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

use snook_proto::{ClientMsg, PROTOCOL_VERSION, ServerMsg, parse_server};
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
    tx: Sender<ClientMsg>,
    rx: Receiver<NetEvent>,
    closed: bool,
}

const PING_EVERY: Duration = Duration::from_secs(15);
/// The server pings back within seconds; silence this long means a dead link.
const SILENCE_LIMIT: Duration = Duration::from_secs(60);
/// How long a read waits before outgoing messages are checked again: short,
/// so a shot or aim update leaves within a few milliseconds.
const POLL: Duration = Duration::from_millis(4);

impl Connection {
    /// Starts connecting to `url` (`ws://host:port/ws` or `wss://…`).
    pub fn open(url: &str) -> Connection {
        let (tx, out_rx) = channel::<ClientMsg>();
        let (in_tx, rx) = channel::<NetEvent>();
        let url = url.trim().to_string();
        std::thread::Builder::new()
            .name("snook-net".into())
            .spawn(move || {
                let reason = match run(&url, &out_rx, &in_tx) {
                    Ok(()) => "Disconnected.".to_string(),
                    Err(e) => e,
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

fn run(url: &str, out_rx: &Receiver<ClientMsg>, in_tx: &Sender<NetEvent>) -> Result<(), String> {
    let request = url.into_client_request().map_err(|e| friendly(&e))?;
    let uri = request.uri();
    let host = uri.host().ok_or("The server address has no host name.")?.to_string();
    let secure = uri.scheme_str() == Some("wss");
    let port = uri.port_u16().unwrap_or(if secure { 443 } else { 80 });
    let addrs: Vec<_> = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|_| crate::i18n::tw("Could not find the server “{host}”.", &[("host", &host)]))?
        .collect();
    let mut last_err = None;
    let mut stream = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, Duration::from_secs(8)) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(e) => last_err = Some(e),
        }
    }
    let stream = stream.ok_or_else(|| {
        friendly(&tungstenite::Error::Io(
            last_err.unwrap_or_else(|| std::io::Error::from(ErrorKind::NotFound)),
        ))
    })?;
    let _ = stream.set_nodelay(true);
    stream.set_read_timeout(Some(Duration::from_secs(10))).map_err(|e| e.to_string())?;
    let (mut ws, _) = tungstenite::client_tls_with_config(
        request,
        stream.try_clone().map_err(|e| e.to_string())?,
        None,
        None,
    )
    .map_err(|e| match e {
        tungstenite::HandshakeError::Failure(e) => friendly(&e),
        tungstenite::HandshakeError::Interrupted(_) => "The server took too long to answer.".into(),
    })?;
    // Short timeouts from now on so sending and receiving can share the thread.
    stream.set_read_timeout(Some(POLL)).map_err(|e| e.to_string())?;

    let send = |ws: &mut tungstenite::WebSocket<_>, msg: &ClientMsg| -> Result<(), String> {
        ws.send(Message::text(msg.to_json())).map_err(|e| friendly(&e))
    };
    send(&mut ws, &ClientMsg::Hello { version: PROTOCOL_VERSION })?;
    if in_tx.send(NetEvent::Connected).is_err() {
        return Ok(());
    }

    let mut last_ping = Instant::now();
    let mut last_heard = Instant::now();
    loop {
        // Outgoing.
        loop {
            match out_rx.try_recv() {
                Ok(msg) => send(&mut ws, &msg)?,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    let _ = ws.close(None);
                    let _ = ws.flush();
                    return Ok(());
                }
            }
        }
        if last_ping.elapsed() > PING_EVERY {
            send(&mut ws, &ClientMsg::Ping)?;
            last_ping = Instant::now();
        }
        if last_heard.elapsed() > SILENCE_LIMIT {
            return Err("Lost contact with the server.".into());
        }
        // Incoming.
        match ws.read() {
            Ok(Message::Text(text)) => {
                last_heard = Instant::now();
                match parse_server(text.as_str()) {
                    Ok(msg) => {
                        if in_tx.send(NetEvent::Message(Box::new(msg))).is_err() {
                            return Ok(());
                        }
                    }
                    Err(e) => eprintln!("ignoring malformed server message: {e}"),
                }
            }
            Ok(Message::Close(_)) => return Err("The server closed the connection.".into()),
            Ok(_) => last_heard = Instant::now(),
            Err(tungstenite::Error::Io(e))
                if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                return Err("The server closed the connection.".into());
            }
            Err(e) => return Err(friendly(&e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_addresses_fail_cleanly() {
        let (_tx, out_rx) = channel();
        let (in_tx, _rx) = channel();
        let err = run("not a url", &out_rx, &in_tx).unwrap_err();
        assert!(err.contains("wss://"), "{err}");
        let err = run("ws://127.0.0.1:1/ws", &out_rx, &in_tx).unwrap_err();
        assert!(
            ["refused", "recusada", "Network", "rede"].iter().any(|w| err.contains(w)),
            "{err}"
        );
    }

    /// Needs internet access: checks that TLS (wss://) is wired up.
    #[test]
    #[ignore]
    fn tls_handshake_reaches_http() {
        let (_tx, out_rx) = channel();
        let (in_tx, _rx) = channel();
        let err = run("wss://example.com/ws", &out_rx, &in_tx).unwrap_err();
        assert!(!err.contains("Secure connection failed"), "{err}");
    }
}
