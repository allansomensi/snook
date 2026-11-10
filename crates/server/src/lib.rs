//! # snook_server
//!
//! The Snook game server: accounts, the lounge (who's online, chat,
//! invitations), authoritative eight-ball matches with a shot clock and
//! reconnection, Elo ratings and the global leaderboard.
//!
//! Clients connect to `GET /ws` (WebSocket, JSON messages from
//! [`snook_proto`]); `GET /health` answers `ok` for monitoring.

pub mod db;
pub mod lounge;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use argon2::Argon2;
use argon2::password_hash::{
    PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng,
};
use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use rand::RngCore;
use snook_core::rating::EloConfig;
use snook_proto::{
    ClientMsg, PROTOCOL_VERSION, ServerMsg, parse_client, valid_password, valid_username,
};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use crate::db::{Db, UserRow};
use crate::lounge::{Lounge, Rules};

/// Server configuration (usually from environment variables, see `main.rs`).
#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    /// SQLite file; `None` keeps everything in memory (tests).
    pub database: Option<PathBuf>,
    pub rules: Rules,
}

pub struct Server {
    db: Mutex<Db>,
    lounge: Mutex<Lounge>,
    next_conn: AtomicU64,
}

/// Closes a connection after this many failed logins.
const MAX_FAILED_LOGINS: u32 = 5;
/// Drops connections that send nothing (clients ping every ~20 s).
const IDLE_TIMEOUT: Duration = Duration::from_secs(90);
const MAX_MESSAGE_BYTES: usize = 64 * 1024;

impl Server {
    pub fn new(config: &Config) -> Result<Arc<Self>, Box<dyn std::error::Error>> {
        let db = match &config.database {
            Some(path) => Db::open(path)?,
            None => Db::open_in_memory()?,
        };
        db.purge_expired_tokens()?;
        Ok(Arc::new(Server {
            db: Mutex::new(db),
            lounge: Mutex::new(Lounge::new(config.rules.clone())),
            next_conn: AtomicU64::new(1),
        }))
    }

    fn with<R>(&self, f: impl FnOnce(&mut Lounge, &Db) -> R) -> R {
        let mut lounge = self.lounge.lock().unwrap_or_else(|e| e.into_inner());
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut lounge, &db)
    }
}

pub fn router(server: Arc<Server>) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/ws", get(ws_handler))
        .with_state(server)
}

/// Runs the server on an already-bound listener until the process stops.
pub async fn serve(listener: TcpListener, server: Arc<Server>) -> std::io::Result<()> {
    let ticker = server.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        loop {
            interval.tick().await;
            ticker.with(|lounge, db| lounge.tick(db));
        }
    });
    axum::serve(listener, router(server)).await
}

async fn ws_handler(State(server): State<Arc<Server>>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.max_message_size(MAX_MESSAGE_BYTES).on_upgrade(move |socket| handle_socket(server, socket))
}

fn new_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hash_password(password: &str) -> Option<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default().hash_password(password.as_bytes(), &salt).ok().map(|h| h.to_string())
}

fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .is_ok_and(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok())
}

enum Auth {
    Ok(UserRow, String),
    Failed(String),
}

async fn authenticate(server: &Arc<Server>, msg: ClientMsg) -> Option<Auth> {
    let server = server.clone();
    let result = tokio::task::spawn_blocking(move || match msg {
        ClientMsg::Register { username, password } => {
            let username = username.trim().to_string();
            if !valid_username(&username) {
                return Auth::Failed("Usernames have 3–16 letters, digits or underscores.".into());
            }
            if !valid_password(&password) {
                return Auth::Failed("Passwords need at least 6 characters.".into());
            }
            let Some(hash) = hash_password(&password) else {
                return Auth::Failed("Could not create the account.".into());
            };
            let db = server.db.lock().unwrap_or_else(|e| e.into_inner());
            match db.create_user(&username, &hash, EloConfig::default().initial_rating) {
                Ok(Some(user)) => {
                    let token = new_token();
                    let _ = db.store_token(&token, user.id);
                    tracing::info!("new account: {}", user.username);
                    Auth::Ok(user, token)
                }
                Ok(None) => Auth::Failed("That username is taken.".into()),
                Err(e) => {
                    tracing::error!("register failed: {e}");
                    Auth::Failed("Could not create the account.".into())
                }
            }
        }
        ClientMsg::Login { username, password } => {
            let user = server
                .db
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .user_by_name(username.trim())
                .ok()
                .flatten();
            match user {
                Some(user) if verify_password(&password, &user.password_hash) => {
                    let token = new_token();
                    let _ = server
                        .db
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .store_token(&token, user.id);
                    Auth::Ok(user, token)
                }
                _ => Auth::Failed("Wrong username or password.".into()),
            }
        }
        ClientMsg::Resume { token } => {
            match server.db.lock().unwrap_or_else(|e| e.into_inner()).user_by_token(&token) {
                Ok(Some(user)) => Auth::Ok(user, token),
                _ => Auth::Failed("Your session expired. Please log in again.".into()),
            }
        }
        _ => Auth::Failed("Please log in first.".into()),
    })
    .await;
    result.ok()
}

async fn handle_socket(server: Arc<Server>, socket: WebSocket) {
    let conn_id = server.next_conn.fetch_add(1, Ordering::Relaxed);
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerMsg>();

    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let close = matches!(&msg, ServerMsg::Error { message } if message.starts_with("You logged in from another"));
            if sink.send(Message::Text(msg.to_json().into())).await.is_err() {
                break;
            }
            if close {
                let _ = sink.send(Message::Close(None)).await;
                break;
            }
        }
    });

    let mut hello = false;
    let mut user: Option<UserRow> = None;
    let mut failures = 0;
    let mut token_in_use: Option<String> = None;
    loop {
        let next = match tokio::time::timeout(IDLE_TIMEOUT, stream.next()).await {
            Ok(Some(Ok(m))) => m,
            _ => break,
        };
        let text = match next {
            Message::Text(t) => t,
            Message::Close(_) => break,
            _ => continue,
        };
        let Ok(msg) = parse_client(text.as_str()) else {
            let _ = tx.send(ServerMsg::Error { message: "Malformed message.".into() });
            continue;
        };
        if !hello {
            match msg {
                ClientMsg::Hello { version } if version == PROTOCOL_VERSION => hello = true,
                ClientMsg::Hello { .. } => {
                    let _ = tx.send(ServerMsg::AuthError {
                        reason: "This version of Snook is out of date. Please update the game."
                            .into(),
                    });
                    break;
                }
                _ => break,
            }
            continue;
        }
        match (&user, msg) {
            (
                None,
                msg @ (ClientMsg::Register { .. }
                | ClientMsg::Login { .. }
                | ClientMsg::Resume { .. }),
            ) => match authenticate(&server, msg).await {
                Some(Auth::Ok(row, token)) => {
                    server.with(|lounge, _| {
                        lounge.join(conn_id, row.clone(), tx.clone(), token.clone())
                    });
                    token_in_use = Some(token);
                    user = Some(row);
                }
                Some(Auth::Failed(reason)) => {
                    failures += 1;
                    let _ = tx.send(ServerMsg::AuthError { reason });
                    if failures >= MAX_FAILED_LOGINS {
                        break;
                    }
                }
                None => break,
            },
            (None, ClientMsg::Ping) => {
                let _ = tx.send(ServerMsg::Pong);
            }
            (None, _) => {
                let _ = tx.send(ServerMsg::AuthError { reason: "Please log in first.".into() });
            }
            (Some(_), ClientMsg::Logout) => {
                if let Some(t) = token_in_use.take() {
                    server.with(|_, db| {
                        let _ = db.delete_token(&t);
                    });
                }
                break;
            }
            (Some(u), msg) => {
                let name = u.username.clone();
                server.with(|lounge, db| lounge.handle(db, &name, msg));
            }
        }
    }

    if let Some(u) = user {
        server.with(|lounge, db| lounge.leave(db, &u.username, conn_id));
    }
    drop(tx);
    let _ = tokio::time::timeout(Duration::from_secs(2), writer).await;
}
