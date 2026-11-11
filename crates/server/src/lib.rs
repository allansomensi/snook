//! # snook_server
//!
//! The Snook game server: accounts, the lounge (who's online, chat,
//! invitations, open tables), authoritative eight-ball matches with a shot
//! clock and reconnection, tournaments, friends, achievements, Elo ratings,
//! the global leaderboard and a web panel for the staff.
//!
//! Endpoints:
//! - `GET /ws`: the game (WebSocket, JSON messages from [`snook_proto`]);
//! - `GET /health`: `ok`, for monitoring;
//! - `GET /api/status`, `GET /api/leaderboard`: public JSON;
//! - `/admin`: the staff panel (see [`admin`]).

pub mod admin;
pub mod lan;
pub mod limits;
pub mod lounge;
pub mod store;
pub mod tournament;

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use argon2::Argon2;
use argon2::password_hash::{
    PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng,
};
use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use rand::RngCore;
use sha2::{Digest, Sha256};
use snook_core::GameOverReason;
use snook_core::rating::EloConfig;
use snook_proto::{
    ACHIEVEMENTS, ClientMsg, FriendInfo, HistoryEntry, LeaderboardEntry, PROTOCOL_VERSION,
    PlayerStatus, Profile, Relation, Role, ServerMsg, clean_text, parse_client, valid_country,
    valid_password, valid_username,
};
use tokio::net::TcpListener;
use tokio::sync::{Notify, Semaphore, mpsc};

use crate::limits::{AuthLimiter, Bucket};
use crate::lounge::{Effect, Lounge, Rules, key, player_info};
use crate::store::{FriendOutcome, FriendState, NewReport, NewUser, SessionKind, Store, User};

/// Unix time in seconds.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// How a game-over reason is stored.
pub fn reason_code(r: GameOverReason) -> &'static str {
    match r {
        GameOverReason::PocketedEight => "pocketed_eight",
        GameOverReason::EightPocketedEarly => "eight_pocketed_early",
        GameOverReason::EightInWrongPocket => "eight_in_wrong_pocket",
        GameOverReason::EightPocketedOnFoul => "eight_pocketed_on_foul",
        GameOverReason::EightOffTable => "eight_off_table",
        GameOverReason::Resignation => "resignation",
        GameOverReason::Abandoned => "abandoned",
    }
}

pub fn parse_reason(code: &str) -> GameOverReason {
    match code {
        "pocketed_eight" | "PocketedEight" => GameOverReason::PocketedEight,
        "eight_pocketed_early" | "EightPocketedEarly" => GameOverReason::EightPocketedEarly,
        "eight_in_wrong_pocket" | "EightInWrongPocket" => GameOverReason::EightInWrongPocket,
        "eight_pocketed_on_foul" | "EightPocketedOnFoul" => GameOverReason::EightPocketedOnFoul,
        "eight_off_table" | "EightOffTable" => GameOverReason::EightOffTable,
        "abandoned" | "Abandoned" => GameOverReason::Abandoned,
        _ => GameOverReason::Resignation,
    }
}

/// Server configuration (usually from environment variables, see `main.rs`).
#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    /// PostgreSQL connection string; `None` keeps everything in memory.
    pub database_url: Option<String>,
    pub database_pool: u32,
    pub rules: Rules,
    /// Usernames that are made administrators when they sign in.
    pub admins: Vec<String>,
    /// Headers with the client's address when behind a proxy, tried in
    /// order (`cf-connecting-ip,true-client-ip,x-forwarded-for`…).
    pub client_ip_header: Option<String>,
    /// With `x-forwarded-for`: how many proxies append to it (the address is
    /// taken that many entries from the right).
    pub proxy_hops: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            bind: "0.0.0.0:7878".parse().expect("address"),
            database_url: None,
            database_pool: 5,
            rules: Rules::default(),
            admins: Vec::new(),
            client_ip_header: None,
            proxy_hops: 1,
        }
    }
}

pub struct Server {
    pub store: Store,
    lounge: Mutex<Lounge>,
    next_conn: AtomicU64,
    auth: Mutex<AuthLimiter>,
    effects: Mutex<Option<mpsc::UnboundedReceiver<Effect>>>,
    pub config: Config,
    pub started: Instant,
    /// Password hashing is slow and memory-hungry: a few at a time.
    hashing: Semaphore,
    leaderboard: Mutex<Option<(Instant, Vec<LeaderboardEntry>)>>,
    connections: Mutex<HashMap<IpAddr, usize>>,
}

/// Drops connections that send nothing (clients ping every ~15 s).
const IDLE_TIMEOUT: Duration = Duration::from_secs(75);
const MAX_MESSAGE_BYTES: usize = 16 * 1024;
const OUTGOING_QUEUE: usize = 512;
/// Game sessions last this long; they are renewed when used after a day.
const SESSION_LIFETIME: i64 = 60 * 60 * 24 * 30;
const SESSION_RENEW_AFTER: i64 = 60 * 60 * 24;
const RESERVED_NAMES: &[&str] =
    &["admin", "administrator", "snook", "moderator", "mod", "system", "staff", "support", "guest"];

impl Server {
    pub async fn new(config: Config) -> Result<Arc<Self>, Box<dyn std::error::Error>> {
        let store = match &config.database_url {
            #[cfg(feature = "postgres")]
            Some(url) => Store::postgres(url, config.database_pool).await?,
            #[cfg(not(feature = "postgres"))]
            Some(_) => return Err("this build has no database support".into()),
            None => Store::memory(),
        };
        let _ = store.purge_sessions(now()).await;
        let (tx, rx) = mpsc::unbounded_channel();
        Ok(Arc::new(Server {
            store,
            lounge: Mutex::new(Lounge::new(config.rules.clone(), tx)),
            next_conn: AtomicU64::new(1),
            auth: Mutex::new(AuthLimiter::default()),
            effects: Mutex::new(Some(rx)),
            config,
            started: Instant::now(),
            hashing: Semaphore::new(2),
            leaderboard: Mutex::new(None),
            connections: Mutex::new(HashMap::new()),
        }))
    }

    /// Runs `f` with the lounge locked.
    pub fn with<R>(&self, f: impl FnOnce(&mut Lounge) -> R) -> R {
        let mut lounge = self.lounge.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut lounge)
    }

    fn auth(&self) -> std::sync::MutexGuard<'_, AuthLimiter> {
        self.auth.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn is_admin_name(&self, name: &str) -> bool {
        self.config.admins.iter().any(|a| a.eq_ignore_ascii_case(name))
    }

    /// The client's address: from the first configured proxy header that is
    /// present (e.g. `cf-connecting-ip,x-forwarded-for`), else the peer.
    pub fn client_ip(&self, peer: SocketAddr, headers: &HeaderMap) -> IpAddr {
        let Some(names) = &self.config.client_ip_header else { return peer.ip() };
        for name in names.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            let Some(value) = headers.get(name).and_then(|v| v.to_str().ok()) else { continue };
            let parts: Vec<&str> = value.split(',').map(str::trim).collect();
            let pick = if name.eq_ignore_ascii_case("x-forwarded-for") {
                parts.len().checked_sub(self.config.proxy_hops.max(1)).and_then(|i| parts.get(i))
            } else {
                parts.first()
            };
            if let Some(ip) = pick.and_then(|p| p.parse().ok()) {
                return ip;
            }
        }
        peer.ip()
    }

    pub async fn leaderboard(&self) -> Vec<LeaderboardEntry> {
        if let Some((at, list)) =
            self.leaderboard.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
            && at.elapsed() < Duration::from_secs(15)
        {
            return list.clone();
        }
        match self.store.leaderboard(100).await {
            Ok(list) => {
                *self.leaderboard.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some((Instant::now(), list.clone()));
                list
            }
            Err(e) => {
                tracing::error!("leaderboard query failed: {e}");
                Vec::new()
            }
        }
    }

    fn invalidate_leaderboard(&self) {
        *self.leaderboard.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    pub async fn hash_password(&self, password: &str) -> Option<String> {
        let _permit = self.hashing.acquire().await.ok()?;
        let password = password.to_string();
        tokio::task::spawn_blocking(move || {
            let salt = SaltString::generate(&mut OsRng);
            Argon2::default().hash_password(password.as_bytes(), &salt).ok().map(|h| h.to_string())
        })
        .await
        .ok()
        .flatten()
    }

    pub async fn verify_password(&self, password: &str, hash: &str) -> bool {
        let Ok(_permit) = self.hashing.acquire().await else { return false };
        let (password, hash) = (password.to_string(), hash.to_string());
        tokio::task::spawn_blocking(move || {
            PasswordHash::new(&hash)
                .is_ok_and(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok())
        })
        .await
        .unwrap_or(false)
    }

    /// Unlocks every achievement `user` has earned and tells them.
    pub async fn check_achievements(&self, user: &User) {
        let friends = self.store.friend_count(user.id).await.unwrap_or(0);
        let mut stats = user.stats(now());
        stats.friends = friends;
        let earned: Vec<&str> = stats.earned().collect();
        match self.store.add_achievements(user.id, &earned, now()).await {
            Ok(new) => {
                let all = self.store.achievements(user.id).await.unwrap_or_default();
                let codes: Vec<String> = all.into_iter().map(|(code, _)| code).collect();
                self.with(|l| {
                    for code in new {
                        l.notify(&user.username, ServerMsg::AchievementUnlocked { code });
                    }
                    l.set_achievements(&user.username, codes);
                });
            }
            Err(e) => tracing::error!("achievements for {}: {e}", user.username),
        }
    }
}

pub fn router(server: Arc<Server>) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/ws", get(ws_handler))
        .route("/api/status", get(api_status))
        .route("/api/leaderboard", get(api_leaderboard))
        .merge(admin::router())
        .with_state(server)
}

fn public_json(value: serde_json::Value) -> Response {
    let mut res = axum::Json(value).into_response();
    let h = res.headers_mut();
    h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=15"));
    res
}

async fn api_status(State(server): State<Arc<Server>>) -> Response {
    let online = server.with(|l| l.online_count());
    public_json(serde_json::json!({
        "online": online,
        "version": env!("CARGO_PKG_VERSION"),
        "protocol": PROTOCOL_VERSION,
    }))
}

async fn api_leaderboard(State(server): State<Arc<Server>>) -> Response {
    let entries = server.leaderboard().await;
    public_json(serde_json::to_value(entries).unwrap_or_default())
}

/// Runs the server on an already-bound listener until the process stops.
pub async fn serve(listener: TcpListener, server: Arc<Server>) -> std::io::Result<()> {
    use axum::serve::ListenerExt;
    spawn_background(server.clone());
    // No Nagle delay: small game messages go out at once.
    let listener = listener.tap_io(|tcp| {
        let _ = tcp.set_nodelay(true);
    });
    axum::serve(listener, router(server).into_make_service_with_connect_info::<SocketAddr>()).await
}

/// The ticker (shot clocks, tournaments), the effect worker and housekeeping.
pub fn spawn_background(server: Arc<Server>) {
    let ticker = server.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(250));
        let mut last_sweep = Instant::now();
        loop {
            interval.tick().await;
            ticker.with(|l| l.tick());
            if last_sweep.elapsed() > Duration::from_secs(600) {
                last_sweep = Instant::now();
                ticker.auth().sweep();
                let _ = ticker.store.purge_sessions(now()).await;
            }
        }
    });
    let rx = server.effects.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(mut rx) = rx {
        tokio::spawn(async move {
            while let Some(effect) = rx.recv().await {
                run_effect(&server, effect).await;
            }
        });
    }
}

async fn run_effect(server: &Arc<Server>, effect: Effect) {
    match effect {
        Effect::Simulate { room, turn, game, shot, called } => {
            let server = server.clone();
            tokio::spawn(async move {
                let outcome = tokio::task::spawn_blocking(move || {
                    let mut game = *game;
                    let mut pending =
                        game.prepare_shot(&shot, called).map_err(|e| e.to_string())?;
                    pending.simulation.run_to_rest();
                    let duration = pending.simulation.time();
                    let report = game.finish_shot(pending);
                    Ok((game, report, duration))
                })
                .await
                .unwrap_or_else(|e| Err(format!("simulation panicked: {e}")));
                server.with(|l| l.commit_shot(room, turn, outcome));
            });
        }
        Effect::RecordMatch { record, keys } => match server.store.record_match(&record).await {
            Ok(users) => {
                server.invalidate_leaderboard();
                for user in users.into_iter().flatten() {
                    server.with(|l| l.refresh_user(user.clone()));
                    server.check_achievements(&user).await;
                }
            }
            Err(e) => tracing::error!("could not record the match {keys:?}: {e}"),
        },
        Effect::RecordTournament(t) => {
            if let Err(e) = server.store.record_tournament(&t).await {
                tracing::error!("could not record the tournament {}: {e}", t.name);
                return;
            }
            for id in &t.players {
                if let Ok(Some(user)) = server.store.user_by_id(*id).await {
                    server.with(|l| l.refresh_user(user.clone()));
                    server.check_achievements(&user).await;
                }
            }
        }
    }
}

async fn ws_handler(
    State(server): State<Arc<Server>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let ip = server.client_ip(peer, &headers);
    {
        let mut conns = server.connections.lock().unwrap_or_else(|e| e.into_inner());
        let n = conns.entry(ip).or_default();
        if *n >= server.config.rules.max_connections_per_ip {
            return (axum::http::StatusCode::TOO_MANY_REQUESTS, "too many connections")
                .into_response();
        }
        *n += 1;
    }
    let guard = ConnGuard { server: server.clone(), ip };
    ws.max_message_size(MAX_MESSAGE_BYTES).max_frame_size(MAX_MESSAGE_BYTES).on_upgrade(
        move |socket| async move {
            handle_socket(server, socket, ip).await;
            drop(guard);
        },
    )
}

/// Counts open connections per address.
struct ConnGuard {
    server: Arc<Server>,
    ip: IpAddr,
}

impl Drop for ConnGuard {
    fn drop(&mut self) {
        let mut conns = self.server.connections.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = conns.get_mut(&self.ip) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                conns.remove(&self.ip);
            }
        }
    }
}

pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex(&bytes)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Tokens are stored hashed: a leaked database can't be used to sign in.
pub fn token_hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

enum Auth {
    Ok { user: Box<User>, token: String, guest: bool },
    Failed(String),
}

fn ban_message(user: &User) -> String {
    let reason = user.ban_reason.as_deref().filter(|r| !r.is_empty()).unwrap_or("not given");
    match user.banned_until {
        Some(t) if t >= i64::MAX / 2 => format!("This account is banned. Reason: {reason}."),
        Some(t) => {
            let days = ((t - now()) as f64 / 86_400.0).ceil().max(1.0) as i64;
            format!("This account is banned for {days} more day(s). Reason: {reason}.")
        }
        None => String::new(),
    }
}

async fn new_session(server: &Server, user: &User) -> Option<String> {
    let token = new_token();
    let t = now();
    match server
        .store
        .create_session(&token_hash(&token), user.id, SessionKind::Game, t, t + SESSION_LIFETIME)
        .await
    {
        Ok(()) => Some(token),
        Err(e) => {
            tracing::error!("could not store a session: {e}");
            None
        }
    }
}

/// Makes configured administrators admins (bootstrap for a new server).
async fn promote_if_admin(server: &Server, mut user: User) -> User {
    if server.is_admin_name(&user.username)
        && user.role != Role::Admin
        && server.store.set_role(user.id, Role::Admin).await.is_ok()
    {
        user.role = Role::Admin;
        tracing::info!("{} is now an administrator", user.username);
    }
    user
}

async fn authenticate(server: &Arc<Server>, msg: ClientMsg, ip: &str) -> Auth {
    let fail = |s: &str| Auth::Failed(s.to_string());
    match msg {
        ClientMsg::Register { username, password, country } => {
            if server.config.rules.lan {
                return fail("This is a LAN server: join as a guest.");
            }
            let username = username.trim().to_string();
            if !valid_username(&username) {
                return fail("Usernames have 3–16 letters, digits or underscores.");
            }
            if RESERVED_NAMES.iter().any(|r| r.eq_ignore_ascii_case(&username))
                && !server.is_admin_name(&username)
            {
                return fail("That username is taken.");
            }
            if !valid_password(&password) {
                return fail("Passwords need at least 8 characters (not only digits).");
            }
            let country = country.trim().to_uppercase();
            let country = if valid_country(&country) { country } else { String::new() };
            if !server.auth().may_register(ip) {
                return fail("Too many new accounts from this network. Try again later.");
            }
            let Some(hash) = server.hash_password(&password).await else {
                return fail("Could not create the account.");
            };
            let role = if server.is_admin_name(&username) { Role::Admin } else { Role::Player };
            let new = NewUser {
                username: &username,
                password_hash: &hash,
                country: &country,
                rating: EloConfig::default().initial_rating,
                role,
                ip,
            };
            match server.store.create_user(&new).await {
                Ok(Some(user)) => {
                    server.auth().registered(ip);
                    tracing::info!("new account: {}", user.username);
                    match new_session(server, &user).await {
                        Some(token) => Auth::Ok { user: Box::new(user), token, guest: false },
                        None => fail("Could not create the account."),
                    }
                }
                Ok(None) => fail("That username is taken."),
                Err(e) => {
                    tracing::error!("register failed: {e}");
                    fail("Could not create the account.")
                }
            }
        }
        ClientMsg::Login { username, password } => {
            if server.config.rules.lan {
                return fail("This is a LAN server: join as a guest.");
            }
            let username = username.trim().to_string();
            if !server.auth().may_try(ip, &username) {
                return fail("Too many failed attempts. Wait a few minutes and try again.");
            }
            let user = match server.store.user_by_name(&username).await {
                Ok(u) => u,
                Err(e) => {
                    tracing::error!("login lookup failed: {e}");
                    return fail("The server is busy. Try again in a moment.");
                }
            };
            // Hash even for unknown names, so timing reveals nothing.
            let ok = match &user {
                Some(u) => server.verify_password(&password, &u.password_hash).await,
                None => {
                    let _ = server
                        .verify_password(&password, "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
                        .await;
                    false
                }
            };
            match user {
                Some(user) if ok => {
                    server.auth().succeeded(&username);
                    if user.is_banned(now()) {
                        return Auth::Failed(ban_message(&user));
                    }
                    let user = promote_if_admin(server, user).await;
                    match new_session(server, &user).await {
                        Some(token) => Auth::Ok { user: Box::new(user), token, guest: false },
                        None => fail("The server is busy. Try again in a moment."),
                    }
                }
                _ => {
                    server.auth().failed(ip, &username);
                    fail("Wrong username or password.")
                }
            }
        }
        ClientMsg::Resume { token } => {
            if server.config.rules.lan {
                return fail("This is a LAN server: join as a guest.");
            }
            let hash = token_hash(&token);
            match server.store.session_user(&hash, SessionKind::Game, now()).await {
                Ok(Some((user, created))) => {
                    if user.is_banned(now()) {
                        return Auth::Failed(ban_message(&user));
                    }
                    let user = promote_if_admin(server, user).await;
                    // Renew sessions in use, so active players stay signed in.
                    if now() - created > SESSION_RENEW_AFTER
                        && let Some(fresh) = new_session(server, &user).await
                    {
                        let _ = server.store.delete_session(&hash).await;
                        return Auth::Ok { user: Box::new(user), token: fresh, guest: false };
                    }
                    Auth::Ok { user: Box::new(user), token, guest: false }
                }
                Ok(None) => fail("Your session expired. Please log in again."),
                Err(e) => {
                    tracing::error!("resume failed: {e}");
                    fail("The server is busy. Try again in a moment.")
                }
            }
        }
        ClientMsg::Guest { name } => {
            if !server.config.rules.lan {
                return fail("Guests can only join LAN games. Create an account.");
            }
            let name = name.trim().to_string();
            if !valid_username(&name) {
                return fail("Names have 3–16 letters, digits or underscores.");
            }
            if server.with(|l| l.is_online(&name)) {
                return fail("Someone with that name is already here.");
            }
            let id = server.with(|l| l.guest_id());
            let t = now();
            let rating = EloConfig::default().initial_rating;
            let user = User {
                id,
                username: name,
                password_hash: String::new(),
                country: String::new(),
                role: Role::Player,
                rating,
                peak_rating: rating,
                games: 0,
                wins: 0,
                eight_wins: 0,
                casual_games: 0,
                streak: 0,
                best_streak: 0,
                runouts: 0,
                shutouts: 0,
                tournaments_played: 0,
                tournaments_won: 0,
                created_at: t,
                last_seen: t,
                banned_until: None,
                ban_reason: None,
                muted_until: None,
            };
            Auth::Ok { user: Box::new(user), token: String::new(), guest: true }
        }
        _ => fail("Please log in first."),
    }
}

async fn handle_socket(server: Arc<Server>, socket: WebSocket, ip: IpAddr) {
    let conn_id = server.next_conn.fetch_add(1, Ordering::Relaxed);
    let ip_text = ip.to_string();
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::channel::<ServerMsg>(OUTGOING_QUEUE);
    let kill = Arc::new(Notify::new());

    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let close = matches!(msg, ServerMsg::Kicked { .. });
            if sink.send(Message::Text(msg.to_json().into())).await.is_err() {
                break;
            }
            if close {
                let _ = sink.send(Message::Close(None)).await;
                break;
            }
        }
    });

    let send = |msg: ServerMsg| {
        let _ = tx.try_send(msg);
    };
    let mut hello = false;
    let mut user: Option<(User, bool)> = None;
    let mut failures = 0;
    let mut token_in_use: Option<String> = None;
    // ~40 messages a second sustained (aim updates are ~20), short bursts more.
    let mut flood = Bucket::new(120.0, 40.0);
    loop {
        let next = tokio::select! {
            m = tokio::time::timeout(IDLE_TIMEOUT, stream.next()) => m,
            _ = kill.notified() => break,
        };
        let next = match next {
            Ok(Some(Ok(m))) => m,
            _ => break,
        };
        let text = match next {
            Message::Text(t) => t,
            Message::Close(_) => break,
            _ => continue,
        };
        if !flood.take() {
            tracing::warn!("{ip_text}: message flood, closing");
            send(ServerMsg::Kicked { reason: "Too many messages.".into() });
            break;
        }
        let Ok(msg) = parse_client(text.as_str()) else {
            send(ServerMsg::Error { message: "Malformed message.".into() });
            continue;
        };
        if !hello {
            match msg {
                ClientMsg::Hello { version } if version == PROTOCOL_VERSION => hello = true,
                ClientMsg::Hello { .. } => {
                    send(ServerMsg::AuthError {
                        reason: "This version of Snook is out of date. Please update the game."
                            .into(),
                    });
                    break;
                }
                _ => break,
            }
            continue;
        }
        let Some((me, guest)) = user.clone() else {
            match msg {
                msg @ (ClientMsg::Register { .. }
                | ClientMsg::Login { .. }
                | ClientMsg::Resume { .. }
                | ClientMsg::Guest { .. }) => match authenticate(&server, msg, &ip_text).await {
                    Auth::Ok { user: row, token, guest } => {
                        let row = *row;
                        let blocked: HashSet<i64> = if guest {
                            HashSet::new()
                        } else {
                            server
                                .store
                                .blocked(row.id)
                                .await
                                .unwrap_or_default()
                                .iter()
                                .map(|u| u.id)
                                .collect()
                        };
                        server.with(|l| {
                            l.join(
                                conn_id,
                                row.clone(),
                                guest,
                                tx.clone(),
                                kill.clone(),
                                token.clone(),
                                ip_text.clone(),
                                blocked,
                            )
                        });
                        if !guest {
                            token_in_use = Some(token);
                            let _ = server.store.touch(row.id, now()).await;
                            let s = server.clone();
                            let r = row.clone();
                            tokio::spawn(async move { s.check_achievements(&r).await });
                            if let Ok(friends) = server.store.friends(row.id).await
                                && friends.iter().any(|(_, st)| *st == FriendState::Incoming)
                            {
                                send(friends_msg(&server, row.id, friends).await);
                            }
                        }
                        user = Some((row, guest));
                    }
                    Auth::Failed(reason) => {
                        failures += 1;
                        send(ServerMsg::AuthError { reason });
                        if failures >= 5 {
                            break;
                        }
                    }
                },
                ClientMsg::Ping => send(ServerMsg::Pong),
                ClientMsg::Leaderboard => {
                    send(ServerMsg::Leaderboard { entries: server.leaderboard().await })
                }
                ClientMsg::Profile { username } => {
                    if let Some(msg) = profile_msg(&server, None, &username).await {
                        send(msg);
                    }
                }
                _ => send(ServerMsg::AuthError { reason: "Please log in first.".into() }),
            }
            continue;
        };
        match msg {
            ClientMsg::Logout => {
                if let Some(t) = token_in_use.take() {
                    let _ = server.store.delete_session(&token_hash(&t)).await;
                }
                break;
            }
            ClientMsg::Leaderboard => {
                send(ServerMsg::Leaderboard { entries: server.leaderboard().await })
            }
            ClientMsg::Profile { username } => {
                let viewer = (!guest).then_some(&me);
                match profile_msg(&server, viewer, &username).await {
                    Some(msg) => send(msg),
                    None => send(ServerMsg::Error { message: "No such player.".into() }),
                }
            }
            msg if guest && is_account_msg(&msg) => {
                send(ServerMsg::Error { message: "Guests don't have this.".into() })
            }
            ClientMsg::SetCountry { country } => {
                let country = country.trim().to_uppercase();
                if valid_country(&country)
                    && server.store.set_country(me.id, &country).await.is_ok()
                {
                    let mut fresh = me.clone();
                    fresh.country = country;
                    server.with(|l| {
                        if let Some(u) = l.session_user(&fresh.username) {
                            let mut u = u.clone();
                            u.country = fresh.country.clone();
                            l.refresh_user(u);
                        }
                    });
                    user = Some((fresh, guest));
                    server.invalidate_leaderboard();
                }
            }
            ClientMsg::Friends => {
                if let Ok(list) = server.store.friends(me.id).await {
                    send(friends_msg(&server, me.id, list).await);
                }
            }
            ClientMsg::FriendRequest { username } => {
                friend_request(&server, &me, &username, &send).await
            }
            ClientMsg::FriendAnswer { username, accept } => {
                if let Ok(Some(other)) = server.store.user_by_name(&username).await {
                    if accept {
                        if server.store.friend_accept(me.id, other.id).await.unwrap_or(false) {
                            server.check_achievements(&me).await;
                            server.check_achievements(&other).await;
                            refresh_friends(&server, &other).await;
                        }
                    } else {
                        let _ = server.store.friend_remove(me.id, other.id).await;
                    }
                    refresh_friends(&server, &me).await;
                }
            }
            ClientMsg::FriendRemove { username } => {
                if let Ok(Some(other)) = server.store.user_by_name(&username).await {
                    let _ = server.store.friend_remove(me.id, other.id).await;
                    refresh_friends(&server, &me).await;
                    refresh_friends(&server, &other).await;
                }
            }
            ClientMsg::Block { username } => {
                if let Ok(Some(other)) = server.store.user_by_name(&username).await
                    && other.id != me.id
                {
                    let _ = server.store.block(me.id, other.id).await;
                    let _ = server.store.friend_remove(me.id, other.id).await;
                    update_blocked(&server, &me).await;
                    refresh_friends(&server, &me).await;
                    refresh_friends(&server, &other).await;
                }
            }
            ClientMsg::Unblock { username } => {
                if let Ok(Some(other)) = server.store.user_by_name(&username).await {
                    let _ = server.store.unblock(me.id, other.id).await;
                    update_blocked(&server, &me).await;
                    refresh_friends(&server, &me).await;
                }
            }
            ClientMsg::Report { username, reason, details } => {
                report(&server, &me, &username, reason, &details, &send).await
            }
            other => {
                let name = me.username.clone();
                server.with(|l| l.handle(&name, other));
            }
        }
    }

    if let Some((u, guest)) = user {
        server.with(|l| l.leave(&u.username, conn_id));
        if !guest {
            let _ = server.store.touch(u.id, now()).await;
        }
    }
    drop(tx);
    let _ = tokio::time::timeout(Duration::from_secs(2), writer).await;
}

fn is_account_msg(msg: &ClientMsg) -> bool {
    matches!(
        msg,
        ClientMsg::SetCountry { .. }
            | ClientMsg::Friends
            | ClientMsg::FriendRequest { .. }
            | ClientMsg::FriendAnswer { .. }
            | ClientMsg::FriendRemove { .. }
            | ClientMsg::Block { .. }
            | ClientMsg::Unblock { .. }
            | ClientMsg::Report { .. }
    )
}

async fn friends_msg(server: &Server, user_id: i64, list: Vec<(User, FriendState)>) -> ServerMsg {
    let blocked =
        server.store.blocked(user_id).await.unwrap_or_default().into_iter().map(|u| u.username);
    let mut friends = Vec::new();
    let mut incoming = Vec::new();
    let mut outgoing = Vec::new();
    server.with(|l| {
        for (u, state) in list {
            match state {
                FriendState::Friends => {
                    let online = l.player(&u.username);
                    friends.push(FriendInfo {
                        online: online.is_some(),
                        player: online
                            .unwrap_or_else(|| player_info(&u, false, PlayerStatus::Available)),
                    })
                }
                FriendState::Incoming => incoming.push(u.username),
                FriendState::Outgoing => outgoing.push(u.username),
            }
        }
    });
    friends.sort_by(|a, b| b.online.cmp(&a.online).then(a.player.username.cmp(&b.player.username)));
    ServerMsg::Friends { friends, incoming, outgoing, blocked: blocked.collect() }
}

/// Sends a fresh friend list to `user` if they are online.
async fn refresh_friends(server: &Server, user: &User) {
    if !server.with(|l| l.is_online(&user.username)) {
        return;
    }
    if let Ok(list) = server.store.friends(user.id).await {
        let msg = friends_msg(server, user.id, list).await;
        server.with(|l| l.notify(&user.username, msg));
    }
}

async fn update_blocked(server: &Server, user: &User) {
    let ids =
        server.store.blocked(user.id).await.unwrap_or_default().iter().map(|u| u.id).collect();
    server.with(|l| l.set_blocked(&user.username, ids));
}

async fn friend_request(server: &Server, me: &User, name: &str, send: &impl Fn(ServerMsg)) {
    let other = match server.store.user_by_name(name.trim()).await {
        Ok(Some(o)) if o.id != me.id => o,
        _ => return send(ServerMsg::Error { message: "No such player.".into() }),
    };
    let blocked_by_them =
        server.store.blocked(other.id).await.unwrap_or_default().iter().any(|u| u.id == me.id);
    if blocked_by_them {
        // Don't reveal the block: it looks like a sent request.
        return send(ServerMsg::Notice { text: "Friend request sent.".into() });
    }
    let current = server.store.friends(me.id).await.unwrap_or_default();
    if current.len() >= 300 {
        return send(ServerMsg::Error { message: "Your friend list is full.".into() });
    }
    match server.store.friend_request(me.id, other.id, now()).await {
        Ok(FriendOutcome::Sent) => {
            server.with(|l| {
                l.notify(
                    &other.username,
                    ServerMsg::FriendRequestReceived { from: me.username.clone() },
                )
            });
            send(ServerMsg::Notice { text: "Friend request sent.".into() });
            refresh_friends(server, &other).await;
        }
        Ok(FriendOutcome::Accepted) => {
            server.check_achievements(me).await;
            server.check_achievements(&other).await;
            refresh_friends(server, &other).await;
        }
        Ok(FriendOutcome::AlreadyFriends | FriendOutcome::AlreadySent) => {}
        Err(e) => tracing::error!("friend request: {e}"),
    }
    refresh_friends(server, me).await;
}

async fn report(
    server: &Server,
    me: &User,
    name: &str,
    reason: snook_proto::ReportReason,
    details: &str,
    send: &impl Fn(ServerMsg),
) {
    let Ok(Some(target)) = server.store.user_by_name(name).await else {
        return send(ServerMsg::Error { message: "No such player.".into() });
    };
    if target.id == me.id {
        return;
    }
    let key_ = format!("report:{}", me.id);
    {
        let mut auth = server.auth();
        if !auth.may_register(&key_) {
            return send(ServerMsg::Error {
                message: "You've sent many reports. Please wait before sending more.".into(),
            });
        }
        auth.registered(&key_);
    }
    let context = server.with(|l| l.report_context(&me.username, &target.username));
    let r = NewReport {
        reporter: me.id,
        target: target.id,
        reason: reason.name().to_string(),
        details: clean_text(details, 500),
        context,
        at: now(),
    };
    match server.store.create_report(&r).await {
        Ok(()) => {
            send(ServerMsg::Notice { text: "Thank you. The moderators will look into it.".into() })
        }
        Err(e) => tracing::error!("report: {e}"),
    }
}

async fn profile_msg(server: &Server, viewer: Option<&User>, name: &str) -> Option<ServerMsg> {
    // LAN guests have no stored profile: build one from the session.
    let user = match server.store.user_by_name(name.trim()).await.ok().flatten() {
        Some(u) => u,
        None => {
            let player = server.with(|l| l.player(name))?;
            return Some(ServerMsg::Profile {
                profile: Box::new(Profile {
                    player,
                    online: true,
                    stats: Default::default(),
                    created_at: now(),
                    rank: None,
                    achievements: Vec::new(),
                    history: Vec::new(),
                    relation: Relation::None,
                }),
            });
        }
    };
    let (achievements, history, rank, friends) = tokio::join!(
        server.store.achievements(user.id),
        server.store.history(user.id, 25),
        server.store.rank_of(user.id),
        server.store.friend_count(user.id),
    );
    let mut stats = user.stats(now());
    stats.friends = friends.unwrap_or(0);
    let relation = match viewer {
        None => Relation::None,
        Some(v) if v.id == user.id => Relation::Me,
        Some(v) => {
            let blocked = server
                .store
                .blocked(v.id)
                .await
                .unwrap_or_default()
                .iter()
                .any(|b| b.id == user.id);
            if blocked {
                Relation::Blocked
            } else {
                match server
                    .store
                    .friends(v.id)
                    .await
                    .unwrap_or_default()
                    .iter()
                    .find(|(u, _)| u.id == user.id)
                    .map(|(_, s)| *s)
                {
                    Some(FriendState::Friends) => Relation::Friend,
                    Some(FriendState::Outgoing) => Relation::RequestSent,
                    Some(FriendState::Incoming) => Relation::RequestReceived,
                    None => Relation::None,
                }
            }
        }
    };
    let online = server.with(|l| l.player(&user.username));
    let known: HashSet<&str> = ACHIEVEMENTS.iter().map(|a| a.code).collect();
    Some(ServerMsg::Profile {
        profile: Box::new(Profile {
            player: online
                .clone()
                .unwrap_or_else(|| player_info(&user, false, PlayerStatus::Available)),
            online: online.is_some(),
            stats,
            created_at: user.created_at,
            rank: rank.ok().flatten(),
            achievements: achievements
                .unwrap_or_default()
                .into_iter()
                .filter(|(c, _)| known.contains(c.as_str()))
                .collect(),
            history: history
                .unwrap_or_default()
                .into_iter()
                .map(|h| HistoryEntry {
                    opponent: h.opponent,
                    opponent_country: h.opponent_country,
                    opponent_rating: h.opponent_rating,
                    won: h.won,
                    reason: parse_reason(&h.reason),
                    rated: h.rated,
                    delta: h.delta,
                    rating_after: h.rating_after,
                    tournament: h.tournament,
                    at: h.at,
                })
                .collect(),
            relation,
        }),
    })
}

/// Lower-case key of a username (re-exported for the admin panel).
pub fn user_key(name: &str) -> String {
    key(name)
}
