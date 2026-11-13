//! Lounge state: who is online, chat, invitations, open tables, match rooms
//! and tournaments.
//!
//! Everything here is in memory and synchronous, behind one mutex: methods
//! are short and never wait for I/O. Work that does (the database, shot
//! simulation) is handed to the effect worker as an [`Effect`]; its results
//! come back through methods such as [`Lounge::commit_shot`].

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use glam::DVec2;
use rand::Rng;
use snookered_core::rating::{self, EloConfig, Rating};
use snookered_core::stats::ShotStats;
use snookered_core::{BallId, EightBallGame, GameOverReason, PocketId, Shot, ShotReport};
use snookered_proto::replay::{Replay, Stroke, shot_difficulty};
use snookered_proto::{
    AimState, ChatKind, ChatLine, ClientMsg, FeaturedMatch, MAX_ROOM_PASSWORD, MatchResult,
    PlayerInfo, PlayerStatus, SPECTATOR_SEAT, ServerMsg, TableInfo, TournamentInfo,
    TournamentState, clean_chat, clean_text,
};
use tokio::sync::Notify;
use tokio::sync::mpsc::{Sender, UnboundedSender, error::TrySendError};

use crate::limits::Bucket;
use crate::now;
use crate::store::{LiveMatch, MatchRecord, TournamentRecord, User};

mod tournaments;

pub use tournaments::{Cup, Joined};

pub fn tournaments_max_fee() -> u32 {
    tournaments::MAX_ENTRY_FEE
}
use tournaments::Tournament;

/// Tunable server rules.
#[derive(Debug, Clone)]
pub struct Rules {
    pub shot_clock: Duration,
    /// How long a disconnected player has to come back before forfeiting.
    pub reconnect_grace: Duration,
    pub chat_history: usize,
    pub motd: String,
    /// LAN server: guests without accounts, no ratings.
    pub lan: bool,
    /// Simultaneous connections from one address.
    pub max_connections_per_ip: usize,
    /// A tournament player who is not online this long after their match is
    /// ready loses it.
    pub tournament_no_show: Duration,
    pub max_open_tournaments: usize,
    /// The weekly cup the server organises (none if not configured).
    pub cup: Option<Cup>,
}

impl Default for Rules {
    fn default() -> Self {
        Rules {
            shot_clock: Duration::from_secs(30),
            reconnect_grace: Duration::from_secs(60),
            chat_history: 60,
            motd: format!("Welcome to {}! Be kind, play fair.", snookered_proto::GAME_NAME),
            lan: false,
            max_connections_per_ip: 12,
            tournament_no_show: Duration::from_secs(120),
            max_open_tournaments: 20,
            cup: None,
        }
    }
}

/// Slow work for the effect worker (see `lib.rs`).
#[derive(Debug)]
pub enum Effect {
    /// Simulate a shot and give the outcome to [`Lounge::commit_shot`].
    Simulate {
        room: u64,
        turn: u32,
        game: Box<EightBallGame>,
        shot: Shot,
        called: Option<PocketId>,
    },
    /// Store a finished match (players are notified of achievements).
    RecordMatch {
        record: MatchRecord,
        keys: [String; 2],
        /// The same two players played too many rated games today: no coins.
        farmed: bool,
    },
    RecordTournament(TournamentRecord),
    /// Save a game in progress (after every stroke).
    SaveLive(LiveMatch),
    /// The game ended (or was handed over): forget the saved copy.
    DeleteLive(i64),
    /// Add a player's shots by difficulty band: (band, attempts, made).
    ShotStats {
        user_id: i64,
        bands: Vec<(usize, u32, u32)>,
    },
    /// Give (or give back) coins: a tournament prize or refund.
    Pay {
        user_id: i64,
        name: String,
        coins: i64,
        reason: String,
        reference: String,
    },
}

/// A restarted server gives the players of a saved game this long to come back.
const RESTORE_GRACE: Duration = Duration::from_secs(180);
/// Rated games between the same two players in a day: from the third the
/// rating changes are halved, from the fifth they stop (no farming).
const PAIR_WINDOW: Duration = Duration::from_secs(24 * 3600);
/// Letters of invitation codes (no 0/O, 1/I/L).
const CODE_LETTERS: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
const CODE_LIFETIME: Duration = Duration::from_secs(30 * 60);

/// Extra time after a shot's simulated duration before the next shot clock
/// starts (the players watch the balls stop and read the result).
const SHOT_MARGIN: Duration = Duration::from_millis(1500);
/// Aim and cue-ball updates are relayed at most this often.
const AIM_RELAY: Duration = Duration::from_millis(33);
const MATCH_CHAT_KEEP: usize = 40;
/// Opening and closing tables, tournaments and the queue are announced to
/// the whole lounge: a dozen at once, then one a second (a person clicking
/// never gets near it; a script can't flood everyone through it).
const ACTION_BURST: f64 = 12.0;
const ACTION_RATE: f64 = 1.0;
/// A tournament still taking names is dropped when its host has been gone
/// this long (it would hold one of the few tournament slots forever).
const HOSTLESS_TOURNAMENT: Duration = Duration::from_secs(300);

pub struct Session {
    pub conn_id: u64,
    pub user: User,
    pub guest: bool,
    pub ip: String,
    tx: Sender<ServerMsg>,
    kill: Arc<Notify>,
    pub room: Option<u64>,
    pub table: Option<u64>,
    /// Users this player blocked (ids).
    pub blocked: HashSet<i64>,
    chat_bucket: Bucket,
    last_chat: String,
    repeats: u32,
    invite_bucket: Bucket,
    /// Lounge actions everyone hears about (tables, tournaments, the queue).
    action_bucket: Bucket,
    last_aim: Option<Instant>,
    /// Achievement codes (they unlock reactions).
    achievements: Vec<String>,
    reaction_bucket: Bucket,
    /// The player whose games this one is watching.
    watching: Option<String>,
    /// VIP badge, cue and chalk (from the wallet).
    pub look: Look,
}

struct Room {
    id: u64,
    /// Lower-case usernames.
    keys: [String; 2],
    /// The players' accounts when the match started (kept for disconnected players).
    users: [User; 2],
    guests: [bool; 2],
    rated: bool,
    game: EightBallGame,
    deadline: Instant,
    disconnected: [Option<Instant>; 2],
    finished: bool,
    rematch: [bool; 2],
    present: [bool; 2],
    breaker: usize,
    /// The shooter's latest aim this turn (played if the shot clock runs out).
    last_aim: Option<AimState>,
    /// The shot being simulated (no other shot or aim is accepted meanwhile).
    busy: Option<u32>,
    shot_at: Instant,
    /// Turns each seat has had (for "break and run").
    turns: [u32; 2],
    /// (tournament, round, index)
    tournament: Option<(u64, usize, usize)>,
    chat: VecDeque<ChatLine>,
    /// Reactions from spectators, all together (a crowd must stay quiet).
    crowd: Bucket,
    /// Spectator -> the seat they cheer for.
    cheers: HashMap<String, usize>,
    /// The whole game so far (saved after every stroke; the replay).
    replay: Replay,
    /// Each seat's numbers this game.
    stats: [ShotStats; 2],
    /// Each seat's measurable pots: (difficulty band, made).
    bands: [Vec<(usize, bool)>; 2],
    /// The cloth and ball set of this table.
    table_look: [String; 2],
    /// Rebuilt after a restart: the players have until then to come back.
    grace_until: Option<Instant>,
}

struct Table {
    id: u64,
    host: String,
    rated: bool,
    password: Option<String>,
}

#[derive(Default)]
pub struct Lounge {
    pub rules: Rules,
    sessions: HashMap<String, Session>,
    /// (from, to) -> rated, lower-case names.
    invites: HashMap<(String, String), bool>,
    rooms: HashMap<u64, Room>,
    tables: BTreeMap<u64, Table>,
    tournaments: BTreeMap<u64, Tournament>,
    next_id: u64,
    chat: VecDeque<ChatLine>,
    /// Player -> who is watching their games (lower-case names).
    watchers: HashMap<String, HashSet<String>>,
    /// "Play now": who is waiting for an opponent.
    queue: Vec<Queued>,
    /// Pairs the queue matched recently (not matched again right away).
    recent_pairs: VecDeque<(String, String, Instant)>,
    effects: Option<UnboundedSender<Effect>>,
    next_guest_id: i64,
    /// The server is restarting: no new games; running ones are handed to
    /// the next server at this time (or when a player drops).
    draining: Option<Instant>,
    /// Invitation codes -> (table, created).
    codes: HashMap<String, (u64, Instant)>,
    /// The game shown on the lounge's "TV".
    featured: Option<FeaturedMatch>,
    last_featured: Option<Instant>,
    /// Rated games between two players (sorted keys) recently.
    pair_games: HashMap<(String, String), VecDeque<Instant>>,
    /// Waits in the "Play now" queue (seconds), for the staff panel.
    pub queue_waits: VecDeque<u32>,
    /// The notes on the board (the store has them; this is what players see).
    board: Vec<snookered_proto::BoardNote>,
}

struct Queued {
    key: String,
    rated: bool,
    since: Instant,
    last_status: Instant,
}

/// How far apart two ratings may be after waiting `secs`: close at first,
/// wider every second, anyone after two minutes.
pub fn queue_range(secs: f32) -> u32 {
    if secs >= 120.0 { u32::MAX } else { (50.0 + secs * 6.0).min(800.0) as u32 }
}

pub fn key(name: &str) -> String {
    name.to_lowercase()
}

fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn clean_password(p: Option<String>) -> Option<String> {
    p.map(|p| clean_text(&p, MAX_ROOM_PASSWORD)).filter(|p| !p.is_empty())
}

pub fn player_info(user: &User, guest: bool, status: PlayerStatus) -> PlayerInfo {
    PlayerInfo {
        username: user.username.clone(),
        rating: user.rating,
        games: user.games,
        wins: user.wins,
        status,
        country: user.country.clone(),
        role: user.role,
        guest,
        vip: false,
        cue: String::new(),
        chalk: String::new(),
        frame: String::new(),
        name_style: String::new(),
        effect: String::new(),
    }
}

/// What others see of a player's shop items.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Look {
    pub vip: bool,
    pub cue: String,
    pub chalk: String,
    /// The table they bring (as the host, or the higher rated player).
    pub cloth: String,
    pub balls: String,
    pub frame: String,
    pub name_style: String,
    pub effect: String,
}

pub fn dressed(mut info: PlayerInfo, look: &Look) -> PlayerInfo {
    info.vip = look.vip;
    info.cue = look.cue.clone();
    info.chalk = look.chalk.clone();
    info.frame = look.frame.clone();
    info.name_style = look.name_style.clone();
    info.effect = look.effect.clone();
    info
}

/// What the admin panel shows about the live server.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LiveRoom {
    pub id: u64,
    pub players: [String; 2],
    pub rated: bool,
    pub shots: u32,
    pub finished: bool,
    pub tournament: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LiveSnapshot {
    pub players: Vec<PlayerInfo>,
    pub rooms: Vec<LiveRoom>,
    pub tables: usize,
    pub tournaments: Vec<TournamentInfo>,
}

impl Lounge {
    pub fn new(rules: Rules, effects: UnboundedSender<Effect>) -> Self {
        Lounge {
            rules,
            // Ids from different runs of the server never meet (saved games
            // keep theirs across a restart).
            next_id: (now().max(0) as u64) << 16,
            effects: Some(effects),
            next_guest_id: -1,
            ..Default::default()
        }
    }

    fn effect(&self, e: Effect) {
        if let Some(tx) = &self.effects {
            let _ = tx.send(e);
        }
    }

    fn new_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// Sends to one player. A player whose connection can't keep up is
    /// disconnected rather than buffering without limit.
    fn send(&self, k: &str, msg: ServerMsg) {
        if let Some(s) = self.sessions.get(k) {
            match s.tx.try_send(msg) {
                Ok(()) | Err(TrySendError::Closed(_)) => {}
                Err(TrySendError::Full(_)) => {
                    tracing::warn!("{k}: outgoing queue full, closing the connection");
                    s.kill.notify_one();
                }
            }
        }
    }

    fn broadcast(&self, msg: &ServerMsg) {
        for k in self.sessions.keys() {
            self.send(k, msg.clone());
        }
    }

    fn status(&self, k: &str) -> PlayerStatus {
        match self.sessions.get(k) {
            Some(s) if s.room.is_some() => PlayerStatus::Playing,
            Some(s) if s.table.is_some() => PlayerStatus::Waiting,
            _ => PlayerStatus::Available,
        }
    }

    fn info(&self, k: &str) -> Option<PlayerInfo> {
        self.sessions
            .get(k)
            .map(|s| dressed(player_info(&s.user, s.guest, self.status(k)), &s.look))
    }

    fn announce(&self, k: &str) {
        if let Some(player) = self.info(k) {
            self.broadcast(&ServerMsg::PlayerUpdated { player });
        }
    }

    fn display(&self, k: &str) -> String {
        self.sessions.get(k).map(|s| s.user.username.clone()).unwrap_or_else(|| k.to_string())
    }

    fn error(&self, k: &str, message: impl Into<String>) {
        self.send(k, ServerMsg::Error { message: message.into() });
    }

    pub fn online_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_online(&self, name: &str) -> bool {
        self.sessions.contains_key(&key(name))
    }

    pub fn player(&self, name: &str) -> Option<PlayerInfo> {
        self.info(&key(name))
    }

    pub fn session_user(&self, name: &str) -> Option<&User> {
        self.sessions.get(&key(name)).map(|s| &s.user)
    }

    /// Distinct client addresses online (if this is 1 with many players, the
    /// proxy header is not configured).
    pub fn distinct_addresses(&self) -> usize {
        self.sessions.values().map(|s| s.ip.as_str()).collect::<HashSet<_>>().len()
    }

    /// A unique id for a LAN guest (negative, never stored).
    pub fn guest_id(&mut self) -> i64 {
        self.next_guest_id -= 1;
        self.next_guest_id
    }

    /// A user finished signing in on connection `conn_id`.
    #[allow(clippy::too_many_arguments)]
    pub fn join(
        &mut self,
        conn_id: u64,
        user: User,
        guest: bool,
        tx: Sender<ServerMsg>,
        kill: Arc<Notify>,
        token: String,
        ip: String,
        blocked: HashSet<i64>,
    ) {
        let k = key(&user.username);
        if self.sessions.contains_key(&k) {
            // Signed in again from somewhere else: whatever the old connection
            // had going in the lounge (an open table, a place in the queue,
            // invitations, a game being watched) ends with it. A match in
            // progress carries over below.
            self.close_table_of(&k);
            self.drop_invites_of(&k);
            self.stop_watching(&k);
            self.queue.retain(|q| q.key != k);
        }
        if let Some(old) = self.sessions.remove(&k) {
            let _ = old
                .tx
                .try_send(ServerMsg::Kicked { reason: "You signed in from another place.".into() });
            old.kill.notify_one();
        }
        let room = self
            .rooms
            .values()
            .find(|r| r.keys.contains(&k) && r.present[r.seat_of(&k)])
            .map(|r| r.id);
        let info = player_info(
            &user,
            guest,
            if room.is_some() { PlayerStatus::Playing } else { PlayerStatus::Available },
        );
        let _ = tx.try_send(ServerMsg::AuthOk { player: info, token });
        let _ =
            tx.try_send(ServerMsg::Welcome { motd: self.rules.motd.clone(), lan: self.rules.lan });
        self.sessions.insert(
            k.clone(),
            Session {
                conn_id,
                user,
                guest,
                ip,
                tx,
                kill,
                room,
                table: None,
                blocked,
                chat_bucket: Bucket::new(5.0, 0.8),
                last_chat: String::new(),
                repeats: 0,
                invite_bucket: Bucket::new(6.0, 0.2),
                action_bucket: Bucket::new(ACTION_BURST, ACTION_RATE),
                last_aim: None,
                achievements: Vec::new(),
                reaction_bucket: Bucket::new(3.0, 0.5),
                watching: None,
                look: Look::default(),
            },
        );
        let players: Vec<PlayerInfo> = self.sessions.keys().filter_map(|k| self.info(k)).collect();
        self.send(&k, ServerMsg::Lounge { players });
        let blocked = &self.sessions[&k].blocked;
        let lines = self
            .chat
            .iter()
            .filter(|l| {
                !self.sessions.get(&key(&l.from)).is_some_and(|s| blocked.contains(&s.user.id))
            })
            .cloned()
            .collect();
        self.send(&k, ServerMsg::ChatHistory { lines });
        let tables = self.tables.values().filter_map(|t| self.table_info(t)).collect();
        self.send(&k, ServerMsg::Tables { tables });
        let tournaments = self.tournaments.values().map(|t| self.tournament_info(t)).collect();
        self.send(&k, ServerMsg::Tournaments { tournaments });
        self.send(&k, ServerMsg::Featured { featured: self.featured.clone() });
        if !self.rules.lan {
            self.send(&k, ServerMsg::Board { notes: self.board.clone() });
        }
        if let Some(until) = self.draining {
            let secs = until.saturating_duration_since(Instant::now()).as_secs() as u32;
            self.send(&k, ServerMsg::Draining { secs });
        }
        self.announce(&k);
        // Back to a running match.
        if let Some(id) = room {
            let clock = self.rules.shot_clock.as_secs() as u32;
            let room = self.rooms.get_mut(&id).expect("room exists");
            let seat = room.seat_of(&k);
            room.disconnected[seat] = None;
            if room.grace_until.is_some() && room.disconnected.iter().all(Option::is_none) {
                // Both are back after a restart: the clock starts now.
                room.grace_until = None;
                room.deadline = Instant::now() + self.rules.shot_clock;
            }
            let opponent = room.keys[1 - seat].clone();
            let msg = room.started_msg(seat, &self.sessions, clock, false);
            self.send(&k, msg);
            self.send(&opponent, ServerMsg::OpponentConnection { connected: true, grace_secs: 0 });
        }
    }

    /// Connection `conn_id` closed.
    pub fn leave(&mut self, username: &str, conn_id: u64) {
        let k = key(username);
        if self.sessions.get(&k).is_none_or(|s| s.conn_id != conn_id) {
            return; // A newer connection replaced this one.
        }
        self.close_table_of(&k);
        self.drop_invites_of(&k);
        self.stop_watching(&k);
        self.queue.retain(|q| q.key != k);
        let session = self.sessions.remove(&k).expect("checked");
        if let Some(id) = session.room
            && let Some(room) = self.rooms.get_mut(&id)
        {
            let seat = room.seat_of(&k);
            let opponent = room.keys[1 - seat].clone();
            if room.finished {
                room.present[seat] = false;
                self.send(&opponent, ServerMsg::OpponentLeft);
                self.end_watching_of(&k, id);
                self.close_room_if_empty(id);
            } else if self.draining.is_some() && room.busy.is_none() {
                // This server is going away: the game moves to the next one
                // (the opponent reconnects there too).
                self.hand_off(id);
            } else {
                room.disconnected[seat] = Some(Instant::now());
                let grace = self.rules.reconnect_grace.as_secs() as u32;
                self.send(
                    &opponent,
                    ServerMsg::OpponentConnection { connected: false, grace_secs: grace },
                );
            }
        }
        // A registered tournament player who leaves before the start is
        // taken off the list (a guest can't come back as the same player).
        let waiting: Vec<u64> = self
            .tournaments
            .values()
            .filter(|t| t.state == TournamentState::Registering && t.players.contains(&k))
            .map(|t| t.id)
            .collect();
        for id in waiting {
            if session.guest {
                self.leave_tournament(&k, id);
            }
        }
        self.broadcast(&ServerMsg::PlayerLeft { username: session.user.username });
    }

    /// Disconnects a player (signed in elsewhere, banned…).
    pub fn kick(&mut self, name: &str, reason: &str) -> bool {
        let k = key(name);
        match self.sessions.get(&k) {
            Some(s) => {
                let _ = s.tx.try_send(ServerMsg::Kicked { reason: reason.to_string() });
                s.kill.notify_one();
                true
            }
            None => false,
        }
    }

    /// Replaces the cached account of an online player (after a change by
    /// the staff, a new country, a finished match…).
    pub fn refresh_user(&mut self, user: User) {
        let k = key(&user.username);
        if let Some(s) = self.sessions.get_mut(&k) {
            s.user = user;
            self.announce(&k);
        }
    }

    pub fn set_blocked(&mut self, name: &str, blocked: HashSet<i64>) {
        if let Some(s) = self.sessions.get_mut(&key(name)) {
            s.blocked = blocked;
        }
    }

    /// Sends a message to a player if they are online.
    pub fn notify(&self, name: &str, msg: ServerMsg) {
        self.send(&key(name), msg);
    }

    /// A message from the staff in the lounge chat.
    pub fn announce_text(&mut self, text: &str) {
        let text = clean_chat(text);
        if text.is_empty() {
            return;
        }
        let line = ChatLine {
            from: snookered_proto::GAME_NAME.into(),
            text,
            at: now(),
            kind: ChatKind::Announcement,
            rating: 0,
        };
        self.push_chat(line.clone());
        self.broadcast(&ServerMsg::Chat { line });
    }

    /// New notes on the board: everyone sees them at once.
    pub fn set_board(&mut self, notes: Vec<snookered_proto::BoardNote>) {
        self.board = notes;
        self.broadcast(&ServerMsg::Board { notes: self.board.clone() });
    }

    pub fn set_motd(&mut self, motd: &str) {
        self.rules.motd = clean_text(motd, 300);
    }

    fn push_chat(&mut self, line: ChatLine) {
        self.chat.push_back(line);
        while self.chat.len() > self.rules.chat_history {
            self.chat.pop_front();
        }
    }

    /// Recent chat lines involving `target`, attached to a report.
    pub fn report_context(&self, reporter: &str, target: &str) -> String {
        let (r, t) = (key(reporter), key(target));
        let mut lines: Vec<String> = self
            .chat
            .iter()
            .filter(|l| key(&l.from) == t)
            .rev()
            .take(10)
            .map(|l| format!("[lounge] {}: {}", l.from, l.text))
            .collect();
        lines.reverse();
        if let Some(room) =
            self.rooms.values().find(|room| room.keys.contains(&r) && room.keys.contains(&t))
        {
            lines.extend(room.chat.iter().map(|l| format!("[match] {}: {}", l.from, l.text)));
        }
        lines.join("\n")
    }

    pub fn live(&self) -> LiveSnapshot {
        let mut players: Vec<PlayerInfo> =
            self.sessions.keys().filter_map(|k| self.info(k)).collect();
        players.sort_by_key(|p| p.username.to_lowercase());
        let mut rooms: Vec<LiveRoom> = self
            .rooms
            .values()
            .map(|r| LiveRoom {
                id: r.id,
                players: [r.users[0].username.clone(), r.users[1].username.clone()],
                rated: r.rated,
                shots: r.game.shots_taken(),
                finished: r.finished,
                tournament: r.tournament.is_some(),
            })
            .collect();
        rooms.sort_by_key(|r| r.id);
        LiveSnapshot {
            players,
            rooms,
            tables: self.tables.len(),
            tournaments: self.tournaments.values().map(|t| self.tournament_info(t)).collect(),
        }
    }

    fn drop_invites_of(&mut self, k: &str) {
        let affected: Vec<(String, String)> =
            self.invites.keys().filter(|(f, t)| f == k || t == k).cloned().collect();
        for (from, to) in affected {
            self.invites.remove(&(from.clone(), to.clone()));
            if from == k {
                self.send(&to, ServerMsg::InviteCancelled { from: self.display(&from) });
            } else {
                self.send(&from, ServerMsg::InviteDeclined { by: self.display(&to) });
            }
        }
    }

    fn close_room_if_empty(&mut self, id: u64) {
        if self.rooms.get(&id).is_some_and(|r| !r.present[0] && !r.present[1]) {
            self.rooms.remove(&id);
        }
    }

    /// Handles a message from a signed-in player.
    pub fn handle(&mut self, username: &str, msg: ClientMsg) {
        let k = key(username);
        let announced = matches!(
            msg,
            ClientMsg::OpenTable { .. }
                | ClientMsg::CloseTable
                | ClientMsg::JoinTable { .. }
                | ClientMsg::QueueJoin { .. }
                | ClientMsg::QueueLeave
                | ClientMsg::Tournaments
                | ClientMsg::CreateTournament { .. }
                | ClientMsg::JoinTournament { .. }
                | ClientMsg::LeaveTournament { .. }
                | ClientMsg::StartTournament { .. }
                | ClientMsg::CreateInviteCode { .. }
                | ClientMsg::JoinByCode { .. }
        );
        if announced && !self.sessions.get_mut(&k).is_some_and(|s| s.action_bucket.take()) {
            return self.error(&k, "Slow down a little.");
        }
        match msg {
            ClientMsg::Chat { text } => self.chat(&k, &text),
            ClientMsg::MatchChat { text } => self.match_chat(&k, &text),
            ClientMsg::React { code } => self.react(&k, &code),
            ClientMsg::Cheer { seat } => self.cheer(&k, seat),
            ClientMsg::StopSpectating => self.stop_watching(&k),
            ClientMsg::QueueJoin { rated } => self.queue_join(&k, rated),
            ClientMsg::QueueLeave => self.queue_leave(&k, true),
            ClientMsg::Invite { to, rated } => self.invite(&k, &key(&to), rated),
            ClientMsg::CancelInvite { to } => {
                let to = key(&to);
                if self.invites.remove(&(k.clone(), to.clone())).is_some() {
                    self.send(&to, ServerMsg::InviteCancelled { from: self.display(&k) });
                }
            }
            ClientMsg::AnswerInvite { from, accept } => self.answer(&k, &key(&from), accept),
            ClientMsg::OpenTable { rated, password } => self.open_table(&k, rated, password),
            ClientMsg::JoinTable { id, password } => self.join_table(&k, id, password),
            ClientMsg::CloseTable => {
                self.close_table_of(&k);
                self.announce(&k);
            }
            ClientMsg::PlaceCueBall { position } => self.place_cue_ball(&k, position),
            ClientMsg::Aim { aim } => self.aim(&k, aim),
            ClientMsg::Shoot { turn, shot, called_pocket, cue_ball } => {
                self.shoot(&k, turn, shot, called_pocket, cue_ball)
            }
            ClientMsg::Resign => self.resign(&k),
            ClientMsg::Rematch => self.rematch(&k),
            ClientMsg::LeaveMatch => self.leave_match(&k),
            ClientMsg::Tournaments => {
                let tournaments =
                    self.tournaments.values().map(|t| self.tournament_info(t)).collect();
                self.send(&k, ServerMsg::Tournaments { tournaments });
            }
            ClientMsg::CreateTournament { name, size, rated, password, entry_fee, format } => {
                // Tournaments with a fee come through the connection (it takes the coins).
                self.create_tournament(&k, &name, size, rated, password, entry_fee, format, false);
            }
            ClientMsg::WatchMatch { match_id } => self.watch_match(&k, match_id),
            ClientMsg::CreateInviteCode { rated } => self.create_code(&k, rated),
            ClientMsg::JoinByCode { code } => self.join_by_code(&k, &code),
            ClientMsg::JoinTournament { id, password } => {
                self.join_tournament(&k, id, password, false);
            }
            ClientMsg::LeaveTournament { id } => self.leave_tournament(&k, id),
            ClientMsg::StartTournament { id } => self.start_tournament(&k, id),
            ClientMsg::Ping => self.send(&k, ServerMsg::Pong),
            // Handled by the connection (they need the database).
            ClientMsg::Hello { .. }
            | ClientMsg::Register { .. }
            | ClientMsg::Login { .. }
            | ClientMsg::Resume { .. }
            | ClientMsg::Guest { .. }
            | ClientMsg::Logout
            | ClientMsg::Leaderboard
            | ClientMsg::Profile { .. }
            | ClientMsg::SetCountry { .. }
            | ClientMsg::Friends
            | ClientMsg::Spectate { .. }
            | ClientMsg::Shop
            | ClientMsg::BuyItem { .. }
            | ClientMsg::Equip { .. }
            | ClientMsg::BuyCoins { .. }
            | ClientMsg::SubscribeVip
            | ClientMsg::ManageVip
            | ClientMsg::GiftVip { .. }
            | ClientMsg::BuyWelcomePack
            | ClientMsg::FriendRequest { .. }
            | ClientMsg::FriendAnswer { .. }
            | ClientMsg::FriendRemove { .. }
            | ClientMsg::Block { .. }
            | ClientMsg::Unblock { .. }
            | ClientMsg::Report { .. }
            | ClientMsg::Account
            | ClientMsg::ChangePassword { .. }
            | ClientMsg::SetRecoveryEmail { .. }
            | ClientMsg::DeleteAccount { .. }
            | ClientMsg::GiftItem { .. }
            | ClientMsg::Missions
            | ClientMsg::Replays
            | ClientMsg::Replay { .. } => {}
        }
    }

    /// Checks the chat limits; the text to send, or None (with an error sent).
    fn chat_allowed(&mut self, k: &str, text: &str) -> Option<String> {
        let text = clean_chat(text);
        if text.is_empty() {
            return None;
        }
        let nowt = now();
        let s = self.sessions.get_mut(k)?;
        if s.user.is_muted(nowt) {
            let mins = (s.user.muted_until.unwrap_or(nowt) - nowt + 59) / 60;
            self.error(k, format!("You are muted for {mins} more minutes."));
            return None;
        }
        if !s.chat_bucket.take() {
            self.error(k, "You're sending messages too quickly.");
            return None;
        }
        if text.eq_ignore_ascii_case(&s.last_chat) {
            s.repeats += 1;
            if s.repeats >= 2 {
                self.error(k, "Please don't repeat the same message.");
                return None;
            }
        } else {
            s.repeats = 0;
            s.last_chat = text.clone();
        }
        Some(text)
    }

    fn chat(&mut self, k: &str, text: &str) {
        let Some(text) = self.chat_allowed(k, text) else { return };
        let s = &self.sessions[k];
        let sender = s.user.id;
        let line = ChatLine {
            from: s.user.username.clone(),
            text,
            at: now(),
            kind: ChatKind::Player,
            rating: if s.guest { 0 } else { s.user.rating },
        };
        self.push_chat(line.clone());
        let msg = ServerMsg::Chat { line };
        for (other, session) in &self.sessions {
            if !session.blocked.contains(&sender) {
                self.send(other, msg.clone());
            }
        }
    }

    fn match_chat(&mut self, k: &str, text: &str) {
        if self.room_of(k).is_none() {
            return;
        }
        let Some(text) = self.chat_allowed(k, text) else { return };
        let s = &self.sessions[k];
        let line = ChatLine {
            from: s.user.username.clone(),
            text,
            at: now(),
            kind: ChatKind::Player,
            rating: if s.guest { 0 } else { s.user.rating },
        };
        let sender = s.user.id;
        let Some(id) = s.room else { return };
        let Some(room) = self.rooms.get_mut(&id) else { return };
        room.chat.push_back(line.clone());
        while room.chat.len() > MATCH_CHAT_KEEP {
            room.chat.pop_front();
        }
        for p in room.keys.clone() {
            let blocked = self.sessions.get(&p).is_some_and(|s| s.blocked.contains(&sender));
            if !blocked {
                self.send(&p, ServerMsg::MatchChat { line: line.clone() });
            }
        }
    }

    /// Sets what others see of `name`'s shop items (and tells everyone).
    pub fn set_look(&mut self, name: &str, look: Look) {
        let k = key(name);
        let Some(s) = self.sessions.get_mut(&k) else { return };
        if s.look != look {
            s.look = look;
            self.announce(&k);
        }
    }

    /// Records what `name` has unlocked and tells them (reactions follow).
    pub fn set_achievements(&mut self, name: &str, codes: Vec<String>) {
        let k = key(name);
        let Some(s) = self.sessions.get_mut(&k) else { return };
        s.achievements = codes.clone();
        self.send(&k, ServerMsg::MyAchievements { codes });
    }

    /// A reaction to everyone at the table (it must be unlocked; a few a
    /// second at most; muted players and blocked senders are not shown).
    fn react(&mut self, k: &str, code: &str) {
        let Some(r) = snookered_proto::reaction(code) else { return };
        let Some(s) = self.sessions.get_mut(k) else { return };
        if !r.unlocked(&s.achievements) || !s.reaction_bucket.take() {
            return;
        }
        if s.user.is_muted(now()) {
            return;
        }
        let sender = s.user.id;
        let (id, seat) = match (s.room, s.watching.clone()) {
            (Some(id), _) => match self.rooms.get(&id) {
                Some(room) => (id, room.seat_of(k)),
                None => return,
            },
            // A spectator: through the room's crowd limit.
            (None, Some(friend)) => {
                let Some(id) = self.sessions.get(&friend).and_then(|f| f.room) else { return };
                let Some(room) = self.rooms.get_mut(&id) else { return };
                if !room.crowd.take() {
                    return;
                }
                (id, SPECTATOR_SEAT)
            }
            (None, None) => return,
        };
        let msg = ServerMsg::Reaction { seat, code: r.code.to_string() };
        let room = &self.rooms[&id];
        for p in room.keys.iter().cloned().chain(self.spectators(id)) {
            let blocked = self.sessions.get(&p).is_some_and(|s| s.blocked.contains(&sender));
            if !blocked {
                self.send(&p, msg.clone());
            }
        }
    }

    fn queue_join(&mut self, k: &str, rated: bool) {
        if self.refuse_if_draining(k) {
            return;
        }
        let Some(s) = self.sessions.get(k) else { return };
        if s.room.is_some() {
            return self.error(k, "Finish your game first.");
        }
        if self.in_tournament(k).is_some() {
            return self.error(k, "Players in a tournament can't start other games.");
        }
        let rated = rated && !s.guest && !self.rules.lan;
        self.queue.retain(|q| q.key != k);
        self.close_table_of(k);
        self.stop_watching(k);
        let now = Instant::now();
        self.queue.push(Queued { key: k.to_string(), rated, since: now, last_status: now });
        self.send(
            k,
            ServerMsg::Queue { searching: true, rated, waited_secs: 0, range: queue_range(0.0) },
        );
        self.match_queue();
    }

    /// Takes `k` out of the queue (`tell`: they asked, so confirm it).
    fn queue_leave(&mut self, k: &str, tell: bool) {
        let before = self.queue.len();
        self.queue.retain(|q| q.key != k);
        if tell || self.queue.len() != before {
            self.send(
                k,
                ServerMsg::Queue { searching: false, rated: false, waited_secs: 0, range: 0 },
            );
        }
    }

    /// Pairs whoever can be paired: the closest ratings first, within what
    /// both have waited for.
    fn match_queue(&mut self) {
        let now = Instant::now();
        self.recent_pairs.retain(|(_, _, t)| now.duration_since(*t) < Duration::from_secs(120));
        // Drop whoever can no longer play (left, started a game elsewhere).
        let gone: Vec<String> = self
            .queue
            .iter()
            .filter(|q| self.sessions.get(&q.key).is_none_or(|s| s.room.is_some()))
            .map(|q| q.key.clone())
            .collect();
        for k in gone {
            self.queue_leave(&k, false);
        }
        loop {
            let mut best: Option<(usize, usize, i32)> = None;
            for i in 0..self.queue.len() {
                for j in (i + 1)..self.queue.len() {
                    let (a, b) = (&self.queue[i], &self.queue[j]);
                    if a.rated != b.rated
                        || self.blocks(&a.key, &b.key)
                        || self.blocks(&b.key, &a.key)
                    {
                        continue;
                    }
                    let waited = |q: &Queued| now.duration_since(q.since).as_secs_f32();
                    // Players who often leave games wait longer for the next.
                    if waited(a) < self.abandon_penalty(&a.key)
                        || waited(b) < self.abandon_penalty(&b.key)
                    {
                        continue;
                    }
                    // Not the same opponent twice in a row, unless nobody else comes.
                    let recent = self.recent_pairs.iter().any(|(x, y, _)| {
                        (x == &a.key && y == &b.key) || (x == &b.key && y == &a.key)
                    });
                    if recent && waited(a).min(waited(b)) < 45.0 {
                        continue;
                    }
                    let ra = self.sessions[&a.key].user.rating;
                    let rb = self.sessions[&b.key].user.rating;
                    let gap = (ra - rb).unsigned_abs();
                    let allowed = queue_range(waited(a)).min(queue_range(waited(b)));
                    if gap <= allowed && best.is_none_or(|(_, _, g)| (gap as i32) < g) {
                        best = Some((i, j, gap as i32));
                    }
                }
            }
            let Some((i, j, _)) = best else { break };
            let b = self.queue.remove(j);
            let a = self.queue.remove(i);
            for q in [&a, &b] {
                self.queue_waits.push_back(now.duration_since(q.since).as_secs() as u32);
            }
            while self.queue_waits.len() > 500 {
                self.queue_waits.pop_front();
            }
            self.recent_pairs.push_back((a.key.clone(), b.key.clone(), now));
            for k in [&a.key, &b.key] {
                self.send(
                    k,
                    ServerMsg::Queue { searching: false, rated: a.rated, waited_secs: 0, range: 0 },
                );
            }
            self.start_room([a.key, b.key], a.rated, None, None);
        }
        // Keep the waiting players informed (every few seconds).
        let updates: Vec<(String, bool, u32)> = self
            .queue
            .iter_mut()
            .filter(|q| now.duration_since(q.last_status) >= Duration::from_secs(3))
            .map(|q| {
                q.last_status = now;
                (q.key.clone(), q.rated, now.duration_since(q.since).as_secs() as u32)
            })
            .collect();
        for (k, rated, secs) in updates {
            self.send(
                k.as_str(),
                ServerMsg::Queue {
                    searching: true,
                    rated,
                    waited_secs: secs,
                    range: queue_range(secs as f32),
                },
            );
        }
    }

    /// Everyone watching room `id` (not its players).
    fn spectators(&self, id: u64) -> Vec<String> {
        let Some(room) = self.rooms.get(&id) else { return Vec::new() };
        let mut out: Vec<String> = room
            .keys
            .iter()
            .filter_map(|k| self.watchers.get(k))
            .flatten()
            .filter(|w| !room.keys.contains(w))
            .cloned()
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Sends to both players and everyone watching.
    fn send_room(&self, id: u64, msg: &ServerMsg) {
        let Some(room) = self.rooms.get(&id) else { return };
        for k in room.keys.iter().cloned().chain(self.spectators(id)) {
            self.send(&k, msg.clone());
        }
    }

    /// Tells everyone at the table how many are watching, and who they
    /// cheer for.
    fn announce_spectators(&self, id: u64) {
        let count = self.spectators(id).len() as u32;
        self.send_room(id, &ServerMsg::Spectators { count });
        self.send_room(id, &ServerMsg::Support { counts: self.support(id) });
    }

    /// How many of those watching room `id` cheer for each seat.
    fn support(&self, id: u64) -> [u32; 2] {
        let mut counts = [0, 0];
        let Some(room) = self.rooms.get(&id) else { return counts };
        for w in self.spectators(id) {
            if let Some(&seat) = room.cheers.get(&w) {
                counts[seat % 2] += 1;
            }
        }
        counts
    }

    /// A spectator picks a side (or changes their mind).
    fn cheer(&mut self, k: &str, seat: usize) {
        if seat > 1 {
            return;
        }
        let Some(s) = self.sessions.get_mut(k) else { return };
        if !s.reaction_bucket.take() {
            return;
        }
        let Some(friend) = s.watching.clone() else { return };
        let Some(id) = self.sessions.get(&friend).and_then(|f| f.room) else { return };
        let Some(room) = self.rooms.get_mut(&id) else { return };
        if room.keys.iter().any(|p| p == k) || room.cheers.get(k) == Some(&seat) {
            return;
        }
        room.cheers.insert(k.to_string(), seat);
        self.send_room(id, &ServerMsg::Support { counts: self.support(id) });
    }

    fn send_spectator_start(&self, w: &str, id: u64, seat: usize) {
        if let Some(room) = self.rooms.get(&id) {
            let clock = self.rules.shot_clock.as_secs() as u32;
            self.send(w, room.started_msg(seat, &self.sessions, clock, true));
        }
    }

    /// `k` starts watching `friend`'s games (the caller checked they may).
    pub fn spectate(&mut self, k: &str, friend: &str) {
        let friend = key(friend);
        let Some(me) = self.sessions.get(k) else { return };
        if me.room.is_some() {
            return self.error(k, "Finish your game first.");
        }
        if friend == k {
            return;
        }
        let Some(id) = self.sessions.get(&friend).and_then(|f| f.room) else {
            return self.error(k, "Your friend isn't playing right now.");
        };
        if self.blocks(&friend, k) {
            return self.error(k, "You can't watch this game.");
        }
        self.stop_watching(k);
        self.watchers.entry(friend.clone()).or_default().insert(k.to_string());
        if let Some(s) = self.sessions.get_mut(k) {
            s.watching = Some(friend.clone());
        }
        let seat = self.rooms[&id].seat_of(&friend);
        self.send_spectator_start(k, id, seat);
        self.announce_spectators(id);
    }

    /// `k` stops watching (if they were).
    pub fn stop_watching(&mut self, k: &str) {
        let Some(friend) = self.sessions.get_mut(k).and_then(|s| s.watching.take()) else {
            return;
        };
        if let Some(set) = self.watchers.get_mut(&friend) {
            set.remove(k);
            if set.is_empty() {
                self.watchers.remove(&friend);
            }
        }
        if let Some(id) = self.sessions.get(&friend).and_then(|f| f.room) {
            self.announce_spectators(id);
        }
    }

    /// `k` left their game: whoever was watching them goes back too.
    fn end_watching_of(&mut self, k: &str, room: u64) {
        let Some(set) = self.watchers.remove(k) else { return };
        for w in set {
            if let Some(s) = self.sessions.get_mut(&w) {
                s.watching = None;
            }
            self.send(
                &w,
                ServerMsg::SpectateEnded { reason: "Your friend left the table.".into() },
            );
        }
        self.announce_spectators(room);
    }

    fn room_of(&self, k: &str) -> Option<&Room> {
        self.sessions.get(k).and_then(|s| s.room).and_then(|id| self.rooms.get(&id))
    }

    fn available(&self, k: &str) -> bool {
        self.sessions.get(k).is_some_and(|s| s.room.is_none())
    }

    /// In a tournament that is still going (and not yet knocked out of it).
    fn in_tournament(&self, k: &str) -> Option<u64> {
        self.tournaments
            .values()
            .find(|t| match t.state {
                TournamentState::Registering => t.players.iter().any(|p| p == k),
                TournamentState::Running => t.alive(k),
                TournamentState::Finished => false,
            })
            .map(|t| t.id)
    }

    /// Still playing in a running tournament: no other games until knocked out.
    fn tournament_busy(&self, k: &str) -> bool {
        self.tournaments.values().any(|t| t.state == TournamentState::Running && t.alive(k))
    }

    fn is_guest(&self, k: &str) -> bool {
        self.sessions.get(k).is_none_or(|s| s.guest)
    }

    fn blocks(&self, who: &str, other: &str) -> bool {
        match (self.sessions.get(who), self.sessions.get(other)) {
            (Some(a), Some(b)) => a.blocked.contains(&b.user.id),
            _ => false,
        }
    }

    fn invite(&mut self, from: &str, to: &str, rated: bool) {
        let fail = |l: &Self, reason: &str| {
            l.send(from, ServerMsg::InviteFailed { reason: reason.into() })
        };
        if self.draining.is_some() {
            return fail(self, "The server is restarting: new games start in a moment.");
        }
        if from == to {
            return fail(self, "You can't invite yourself.");
        }
        if !self.sessions.contains_key(to) {
            return fail(self, "That player is not online.");
        }
        if !self.available(from) || !self.available(to) {
            return fail(self, "That player is in a game.");
        }
        if self.tournament_busy(from) || self.tournament_busy(to) {
            return fail(self, "Players in a tournament can't start other games.");
        }
        if self.blocks(to, from) {
            return fail(self, "That player is not accepting invitations.");
        }
        if self.invites.contains_key(&(from.to_string(), to.to_string())) {
            return;
        }
        if !self.sessions.get_mut(from).is_some_and(|s| s.invite_bucket.take()) {
            return fail(self, "You're sending invitations too quickly.");
        }
        let rated = rated && !self.is_guest(from) && !self.is_guest(to);
        if let Some(rated_back) = self.invites.get(&(to.to_string(), from.to_string())).copied() {
            // They already invited us: show their invitation again.
            return self
                .send(from, ServerMsg::Invited { from: self.display(to), rated: rated_back });
        }
        self.invites.insert((from.to_string(), to.to_string()), rated);
        self.send(to, ServerMsg::Invited { from: self.display(from), rated });
        self.send(from, ServerMsg::InviteSent { to: self.display(to), rated });
    }

    fn answer(&mut self, me: &str, from: &str, accept: bool) {
        let Some(rated) = self.invites.remove(&(from.to_string(), me.to_string())) else {
            return self.send(
                me,
                ServerMsg::InviteFailed { reason: "That invitation is no longer valid.".into() },
            );
        };
        if !accept {
            self.send(from, ServerMsg::InviteDeclined { by: self.display(me) });
            return;
        }
        if !self.available(from) || !self.available(me) {
            return self.send(
                me,
                ServerMsg::InviteFailed { reason: "That player is no longer available.".into() },
            );
        }
        self.start_room([from.to_string(), me.to_string()], rated, None, Some(from.to_string()));
    }

    fn table_info(&self, t: &Table) -> Option<TableInfo> {
        Some(TableInfo {
            id: t.id,
            host: self.info(&t.host)?,
            rated: t.rated,
            private: t.password.is_some(),
        })
    }

    fn open_table(&mut self, k: &str, rated: bool, password: Option<String>) {
        if self.refuse_if_draining(k) {
            return;
        }
        if !self.available(k) {
            return self.error(k, "Finish your game first.");
        }
        if self.tournament_busy(k) {
            return self.error(k, "Players in a tournament can't start other games.");
        }
        self.close_table_of(k);
        let id = self.new_id();
        let table = Table {
            id,
            host: k.to_string(),
            rated: rated && !self.is_guest(k),
            password: clean_password(password),
        };
        if let Some(s) = self.sessions.get_mut(k) {
            s.table = Some(id);
        }
        self.announce(k);
        if let Some(info) = self.table_info(&table) {
            self.broadcast(&ServerMsg::TableOpened { table: info });
        }
        self.tables.insert(id, table);
    }

    fn close_table_of(&mut self, k: &str) {
        let Some(id) = self.sessions.get_mut(k).and_then(|s| s.table.take()) else { return };
        self.tables.remove(&id);
        self.broadcast(&ServerMsg::TableClosed { id });
    }

    fn join_table(&mut self, k: &str, id: u64, password: Option<String>) {
        if self.refuse_if_draining(k) {
            return;
        }
        let Some(t) = self.tables.get(&id) else {
            return self.error(k, "That table is no longer open.");
        };
        let host = t.host.clone();
        if host == k {
            return;
        }
        if let Some(expected) = &t.password
            && !ct_eq(expected, &clean_password(password).unwrap_or_default())
        {
            return self.error(k, "Wrong table password.");
        }
        if !self.available(k) {
            return self.error(k, "Finish your game first.");
        }
        if self.tournament_busy(k) {
            return self.error(k, "Players in a tournament can't start other games.");
        }
        if self.blocks(&host, k) {
            return self.error(k, "You can't join this table.");
        }
        let rated = t.rated && !self.is_guest(k);
        self.close_table_of(k);
        self.close_table_of(&host);
        self.codes.retain(|_, (t, _)| *t != id);
        self.start_room([host.clone(), k.to_string()], rated, None, Some(host));
    }

    /// `host`: whose table it is (their cloth and balls); else the higher
    /// rated player's.
    fn start_room(
        &mut self,
        mut keys: [String; 2],
        mut rated: bool,
        tournament: Option<(u64, usize, usize)>,
        host: Option<String>,
    ) {
        if !keys.iter().all(|k| self.sessions.contains_key(k)) {
            return;
        }
        if self.draining.is_some() {
            for k in &keys {
                self.error(k, "The server is restarting: new games start in a moment.");
            }
            return;
        }
        if tournament.is_none() && rand::thread_rng().gen_bool(0.5) {
            keys.swap(0, 1);
        }
        for k in &keys {
            self.drop_invites_of(k);
            self.close_table_of(k);
            self.queue_leave(k, false);
        }
        let users = [self.sessions[&keys[0]].user.clone(), self.sessions[&keys[1]].user.clone()];
        let guests = [self.sessions[&keys[0]].guest, self.sessions[&keys[1]].guest];
        rated &= !guests[0] && !guests[1];
        let host = host.unwrap_or_else(|| {
            if users[0].rating >= users[1].rating { keys[0].clone() } else { keys[1].clone() }
        });
        let id = self.new_id();
        let look = self.table_look_of(&host);
        self.create_room(id, keys, users, guests, rated, 0, tournament, look);
    }

    /// The cloth and ball set `host` brings to the table.
    fn table_look_of(&self, host: &str) -> [String; 2] {
        self.sessions
            .get(host)
            .map(|s| [s.look.cloth.clone(), s.look.balls.clone()])
            .unwrap_or_default()
    }

    #[allow(clippy::too_many_arguments)]
    fn create_room(
        &mut self,
        id: u64,
        keys: [String; 2],
        users: [User; 2],
        guests: [bool; 2],
        rated: bool,
        breaker: usize,
        tournament: Option<(u64, usize, usize)>,
        table_look: [String; 2],
    ) {
        let seed: u64 = rand::thread_rng().r#gen();
        let mut turns = [0, 0];
        turns[breaker] = 1;
        let room = Room {
            id,
            keys: keys.clone(),
            users,
            guests,
            rated,
            game: EightBallGame::new(seed, breaker),
            deadline: Instant::now() + self.rules.shot_clock,
            disconnected: [None, None],
            finished: false,
            rematch: [false, false],
            present: [true, true],
            breaker,
            last_aim: None,
            busy: None,
            shot_at: Instant::now(),
            turns,
            tournament,
            chat: VecDeque::new(),
            crowd: Bucket::new(4.0, 0.6),
            cheers: HashMap::new(),
            replay: Replay::new(seed, breaker),
            stats: Default::default(),
            bands: Default::default(),
            table_look,
            grace_until: None,
        };
        let clock = self.rules.shot_clock.as_secs() as u32;
        for k in &keys {
            self.stop_watching(k);
            if let Some(s) = self.sessions.get_mut(k) {
                s.room = Some(id);
            }
        }
        for (seat, k) in keys.iter().enumerate() {
            let msg = room.started_msg(seat, &self.sessions, clock, false);
            self.send(k, msg);
        }
        self.rooms.insert(id, room);
        for k in &keys {
            self.announce(k);
        }
        // Friends watching either player follow them to the new game.
        for (seat, k) in keys.iter().enumerate() {
            for w in self.watchers.get(k).cloned().unwrap_or_default() {
                self.send_spectator_start(&w, id, seat);
            }
        }
        self.announce_spectators(id);
        self.save_live(id);
        tracing::info!(room = id, rated, "match started: {} vs {}", keys[0], keys[1]);
    }

    /// Saves a game in progress (games with accounts only).
    fn save_live(&self, id: u64) {
        let Some(room) = self.rooms.get(&id) else { return };
        if self.rules.lan || room.guests.iter().any(|g| *g) || room.finished {
            return;
        }
        self.effect(Effect::SaveLive(LiveMatch {
            id: id as i64,
            players: [room.users[0].id, room.users[1].id],
            rated: room.rated,
            replay: room.replay.clone(),
            updated_at: now(),
        }));
    }

    /// The running room of `k` and their seat, if it is their turn.
    fn my_turn(&self, k: &str) -> Option<(u64, usize)> {
        let id = self.sessions.get(k)?.room?;
        let room = self.rooms.get(&id)?;
        let seat = room.seat_of(k);
        (!room.finished && room.busy.is_none() && room.game.current_player() == seat)
            .then_some((id, seat))
    }

    /// True if `k` may relay another aim update now (drops floods).
    fn aim_allowed(&mut self, k: &str) -> bool {
        let Some(s) = self.sessions.get_mut(k) else { return false };
        let now = Instant::now();
        if s.last_aim.is_some_and(|t| now.duration_since(t) < AIM_RELAY) {
            return false;
        }
        s.last_aim = Some(now);
        true
    }

    fn place_cue_ball(&mut self, k: &str, p: DVec2) {
        if !p.is_finite() {
            return;
        }
        let Some((id, seat)) = self.my_turn(k) else { return };
        // The final position comes with the shot, so dropping some is harmless.
        if !self.aim_allowed(k) {
            return;
        }
        let room = self.rooms.get_mut(&id).expect("room");
        if room.game.ball_in_hand().is_some() && room.game.place_cue_ball(p) {
            let opponent = room.keys[1 - seat].clone();
            self.send(&opponent, ServerMsg::OpponentCueBall { position: p });
            for w in self.spectators(id) {
                self.send(&w, ServerMsg::OpponentCueBall { position: p });
            }
        }
    }

    fn aim(&mut self, k: &str, aim: AimState) {
        if !aim.is_finite() {
            return;
        }
        let Some((id, seat)) = self.my_turn(k) else { return };
        // Always remember the newest aim; only relaying is throttled.
        if let Some(room) = self.rooms.get_mut(&id) {
            room.last_aim = Some(aim);
        }
        if !self.aim_allowed(k) {
            return;
        }
        let opponent = self.rooms[&id].keys[1 - seat].clone();
        self.send(&opponent, ServerMsg::OpponentAim { aim });
        for w in self.spectators(id) {
            self.send(&w, ServerMsg::OpponentAim { aim });
        }
    }

    fn shoot(
        &mut self,
        k: &str,
        turn: u32,
        shot: Shot,
        called: Option<PocketId>,
        cue_ball: Option<DVec2>,
    ) {
        let reject = |l: &Self, reason: &str| {
            if let Some(room) = l.room_of(k) {
                l.send(
                    k,
                    ServerMsg::ShotRejected {
                        turn,
                        reason: reason.into(),
                        game: room.game.clone(),
                    },
                );
            }
        };
        let Some((id, seat)) = self.my_turn(k) else {
            return reject(self, "It's not your turn.");
        };
        if self.rooms[&id].game.shots_taken() != turn {
            return reject(self, "That shot was for an earlier turn.");
        }
        let finite = shot.yaw.is_finite()
            && shot.elevation.is_finite()
            && shot.tip.is_finite()
            && shot.speed.is_finite()
            && cue_ball.is_none_or(|p| p.is_finite());
        if !finite {
            return reject(self, "Invalid shot.");
        }
        if let Err(e) = self.start_shot(id, seat, shot, called, cue_ball, false) {
            reject(self, &e);
        }
    }

    /// Accepts a shot: tells both players straight away and has it simulated.
    fn start_shot(
        &mut self,
        id: u64,
        seat: usize,
        shot: Shot,
        called: Option<PocketId>,
        cue_ball: Option<DVec2>,
        timeout: bool,
    ) -> Result<(), String> {
        let room = self.rooms.get_mut(&id).ok_or("No such game.")?;
        if let Some(p) = cue_ball
            && room.game.ball_in_hand().is_some()
            && room.game.ball(BallId::CUE).xy() != p
            && !room.game.place_cue_ball(p)
        {
            return Err("The cue ball can't be placed there.".into());
        }
        // Validate before telling anyone (cheap: the simulation isn't run).
        room.game.prepare_shot(&shot, called).map_err(|e| format!("Can't shoot: {e}."))?;
        let turn = room.game.shots_taken();
        let cue_ball = room.game.ball_in_hand().map(|_| room.game.ball(BallId::CUE).xy());
        room.busy = Some(turn);
        room.shot_at = Instant::now();
        room.last_aim = None;
        room.replay.strokes.push(Stroke::Shot { shot, called, cue_ball, timeout });
        let msg =
            ServerMsg::ShotTaken { seat, turn, shot, called_pocket: called, cue_ball, timeout };
        let game = Box::new(room.game.clone());
        self.send_room(id, &msg);
        self.effect(Effect::Simulate { room: id, turn, game, shot, called });
        Ok(())
    }

    /// The outcome of a simulated shot (from the effect worker).
    pub fn commit_shot(
        &mut self,
        id: u64,
        turn: u32,
        outcome: Result<(EightBallGame, ShotReport, f64), String>,
    ) {
        let clock = self.rules.shot_clock;
        let Some(room) = self.rooms.get_mut(&id) else { return };
        if room.busy != Some(turn) || room.finished {
            return;
        }
        room.busy = None;
        let (game, report, duration) = match outcome {
            Ok(o) => o,
            Err(e) => {
                tracing::error!(room = id, "shot simulation failed: {e}");
                room.replay.strokes.pop();
                let msg = ServerMsg::ShotRejected {
                    turn,
                    reason: "The server could not play that shot.".into(),
                    game: room.game.clone(),
                };
                room.deadline = Instant::now() + clock;
                self.send_room(id, &msg);
                return;
            }
        };
        let shooter = room.game.current_player();
        // The numbers for the end-of-game panel, the profile and the review
        // of aim assistance (measured on the table as it was).
        let in_hand = room.game.ball_in_hand().is_some();
        room.stats[shooter].add(&report, in_hand);
        if let Some(Stroke::Shot { shot, .. }) = room.replay.strokes.last() {
            room.stats[shooter].add_spin(shot, &report);
            if let Some(band) = shot_difficulty(&room.game, shot, &report) {
                room.bands[shooter].push(band);
            }
        }
        room.game = game;
        let next = room.game.current_player();
        if next != shooter {
            room.turns[next] += 1;
        }
        room.deadline = room.shot_at + Duration::from_secs_f64(duration) + SHOT_MARGIN + clock;
        let clock_left = room.deadline.saturating_duration_since(Instant::now()).as_secs_f32();
        let msg = ServerMsg::ShotResult { turn, end: room.game.clone(), report, clock_left };
        let over = room.game.game_over().is_some();
        self.send_room(id, &msg);
        if over {
            self.finish(id);
        } else {
            self.save_live(id);
        }
    }

    /// The shot clock ran out: play the shot at full power where the player
    /// was aiming (or at the nearest legal ball if they never aimed).
    fn timeout_shot(&mut self, id: u64) {
        let Some(room) = self.rooms.get(&id) else { return };
        let seat = room.game.current_player();
        let game = &room.game;
        let aim = room.last_aim;
        let shot = Shot {
            yaw: aim.map(|a| a.yaw).unwrap_or_else(|| game.default_aim()),
            elevation: aim.map(|a| a.elevation).unwrap_or(0.0),
            tip: aim.map(|a| a.tip).unwrap_or(DVec2::ZERO),
            speed: game.params().max_cue_speed,
        };
        let called = game.needs_called_pocket().then(|| {
            aim.and_then(|a| a.called_pocket).unwrap_or_else(|| game.nearest_pocket(BallId::EIGHT))
        });
        if self.start_shot(id, seat, shot, called, None, true).is_err() {
            // The cue ball is somewhere it can't be shot from: pass the turn.
            let clock = self.rules.shot_clock;
            if let Some(room) = self.rooms.get_mut(&id) {
                room.game.time_foul();
                room.replay.strokes.push(Stroke::TimeFoul);
                room.turns[room.game.current_player()] += 1;
                room.deadline = Instant::now() + clock;
                room.last_aim = None;
                let msg = ServerMsg::TimeFoul {
                    seat,
                    game: room.game.clone(),
                    clock_left: clock.as_secs_f32(),
                };
                self.send_room(id, &msg);
                self.save_live(id);
            }
        }
    }

    fn resign(&mut self, k: &str) {
        let Some(id) = self.sessions.get(k).and_then(|s| s.room) else { return };
        let Some(room) = self.rooms.get_mut(&id) else { return };
        if room.finished {
            return;
        }
        let seat = room.seat_of(k);
        room.game.resign(seat);
        room.replay.strokes.push(Stroke::Resign { seat });
        self.finish(id);
    }

    /// Settles ratings and notifies both players.
    fn finish(&mut self, id: u64) {
        let Some(room) = self.rooms.get(&id) else { return };
        let Some(over) = room.game.game_over() else { return };
        if room.finished {
            return;
        }
        // The freshest numbers: from the session if the player is online.
        let current = |seat: usize| -> User {
            self.sessions
                .get(&room.keys[seat])
                .map(|s| s.user.clone())
                .unwrap_or_else(|| room.users[seat].clone())
        };
        let users = [current(0), current(1)];
        let rated = room.rated;
        let mut delta = [0, 0];
        let mut new_ratings = [users[0].rating, users[1].rating];
        let keys = room.keys.clone();
        let repeats = if rated { self.count_pair_game(&keys) } else { 0 };
        let room = &self.rooms[&id];
        if rated {
            let (w, l) = (over.winner, 1 - over.winner);
            let change =
                rating::apply_result(&EloConfig::default(), users[w].rating(), users[l].rating());
            let (rw, rl): (Rating, Rating) = (change.winner, change.loser);
            delta[w] = change.winner_delta;
            delta[l] = change.loser_delta;
            new_ratings[w] = rw.rating;
            new_ratings[l] = rl.rating;
            // The same two players again and again: less and less at stake.
            let scale = match repeats {
                0..=2 => 1.0,
                3..=4 => 0.5,
                _ => 0.0,
            };
            if scale < 1.0 {
                for seat in 0..2 {
                    delta[seat] = (delta[seat] as f64 * scale).round() as i32;
                    new_ratings[seat] = users[seat].rating + delta[seat];
                }
            }
        }
        let loser = 1 - over.winner;
        let eight_win = over.reason == GameOverReason::PocketedEight;
        let shutout = eight_win
            && room.game.group_of(loser).is_some_and(|g| room.game.remaining(g).len() == 7);
        let record = MatchRecord {
            players: [users[0].id, users[1].id],
            winner: over.winner,
            reason: over.reason,
            rated,
            delta,
            ratings_after: new_ratings,
            shots: room.game.shots_taken(),
            tournament: room.tournament.map(|t| t.0 as i64),
            eight_win,
            runout: eight_win && room.turns[loser] == 0 && room.breaker == over.winner,
            shutout,
            ended_at: now(),
            replay: Some(room.replay.clone()),
            stats: room.stats,
            abandoned: (over.reason == GameOverReason::Abandoned).then_some(loser),
        };
        let farmed = repeats > 4;
        let guests = room.guests;
        let tournament = room.tournament;
        let result = MatchResult {
            winner: over.winner,
            reason: over.reason,
            rated,
            rating_delta: delta,
            new_ratings,
        };
        let room = self.rooms.get_mut(&id).expect("room");
        room.finished = true;
        room.rematch = [false, false];
        // Show the new numbers at once; the database catches up.
        for (seat, k) in keys.iter().enumerate() {
            if let Some(s) = self.sessions.get_mut(k) {
                crate::store::apply_match_numbers(&mut s.user, &record, seat);
            }
        }
        for w in self.spectators(id) {
            self.send(&w, ServerMsg::MatchEnded { result: result.clone() });
        }
        for k in &keys {
            self.send(k, ServerMsg::MatchEnded { result: result.clone() });
            self.announce(k);
        }
        if !guests[0] && !guests[1] {
            let room = &self.rooms[&id];
            for seat in 0..2 {
                let mut bands: BTreeMap<usize, (u32, u32)> = BTreeMap::new();
                for &(band, made) in &room.bands[seat] {
                    let e = bands.entry(band).or_default();
                    e.0 += 1;
                    e.1 += made as u32;
                }
                if !bands.is_empty() {
                    self.effect(Effect::ShotStats {
                        user_id: room.users[seat].id,
                        bands: bands.into_iter().map(|(b, (a, m))| (b, a, m)).collect(),
                    });
                }
            }
            self.effect(Effect::DeleteLive(id as i64));
            self.effect(Effect::RecordMatch { record, keys: keys.clone(), farmed });
        }
        tracing::info!(room = id, "match finished: {:?}", over);
        if let Some((tid, round, index)) = tournament {
            self.tournament_game_over(tid, round, index, &keys, over.winner);
        }
    }

    fn rematch(&mut self, k: &str) {
        let Some(id) = self.sessions.get(k).and_then(|s| s.room) else { return };
        let Some(room) = self.rooms.get_mut(&id) else { return };
        if !room.finished {
            return;
        }
        if room.tournament.is_some() {
            return self.error(k, "Tournament games have no rematch.");
        }
        let seat = room.seat_of(k);
        if !room.present[1 - seat] || !self.sessions.contains_key(&room.keys[1 - seat]) {
            return self.send(k, ServerMsg::OpponentLeft);
        }
        room.rematch[seat] = true;
        if room.rematch[1 - seat] {
            let keys = room.keys.clone();
            let rated = room.rated;
            let breaker = 1 - room.breaker;
            let guests = room.guests;
            let chat = room.chat.clone();
            let table_look = room.table_look.clone();
            self.rooms.remove(&id);
            for p in &keys {
                if let Some(s) = self.sessions.get_mut(p) {
                    s.room = None;
                }
            }
            let users =
                [self.sessions[&keys[0]].user.clone(), self.sessions[&keys[1]].user.clone()];
            if self.draining.is_some() {
                for k in &keys {
                    self.error(k, "The server is restarting: new games start in a moment.");
                }
                return;
            }
            let new_id = self.new_id();
            self.create_room(new_id, keys, users, guests, rated, breaker, None, table_look);
            if let Some(r) = self.rooms.get_mut(&new_id) {
                r.chat = chat;
            }
        } else {
            let opponent = room.keys[1 - seat].clone();
            self.send(&opponent, ServerMsg::RematchOffered);
        }
    }

    fn leave_match(&mut self, k: &str) {
        let Some(id) = self.sessions.get(k).and_then(|s| s.room) else { return };
        let Some(room) = self.rooms.get_mut(&id) else { return };
        if !room.finished {
            return self.error(k, "Resign first to leave a game in progress.");
        }
        let seat = room.seat_of(k);
        room.present[seat] = false;
        let opponent = room.keys[1 - seat].clone();
        if let Some(s) = self.sessions.get_mut(k) {
            s.room = None;
        }
        self.send(&opponent, ServerMsg::OpponentLeft);
        self.end_watching_of(k, id);
        self.close_room_if_empty(id);
        self.announce(k);
        if let Some(tid) = self.in_tournament(k) {
            self.tournament_progress(tid);
        }
    }

    /// Called periodically: shot clocks, abandoned games, tournaments.
    pub fn tick(&mut self) {
        if !self.queue.is_empty() {
            self.match_queue();
        }
        let nowi = Instant::now();
        let grace = self.rules.reconnect_grace;
        let mut finished = Vec::new();
        let mut timeouts = Vec::new();
        let mut dropped = Vec::new();
        for room in self.rooms.values_mut().filter(|r| !r.finished) {
            if room.grace_until.is_some_and(|t| nowi < t) {
                continue;
            }
            let gone = |s: usize| {
                room.disconnected[s].is_some_and(|t| nowi - t > grace)
                    || (room.grace_until.is_some() && room.disconnected[s].is_some())
            };
            if gone(0) && gone(1) {
                // Nobody came back: no result.
                dropped.push(room.id);
                continue;
            }
            if let Some(seat) = (0..2).find(|s| gone(*s)) {
                room.game.abandon(seat);
                room.replay.strokes.push(Stroke::Abandon { seat });
                finished.push(room.id);
                continue;
            }
            if room.busy.is_none() && nowi > room.deadline {
                timeouts.push(room.id);
            }
        }
        for id in dropped {
            if let Some(room) = self.rooms.remove(&id) {
                tracing::info!(room = id, "game dropped: both players gone");
                for k in &room.keys {
                    if let Some(s) = self.sessions.get_mut(k)
                        && s.room == Some(id)
                    {
                        s.room = None;
                    }
                }
                self.effect(Effect::DeleteLive(id as i64));
            }
        }
        for id in timeouts {
            self.timeout_shot(id);
        }
        // The server is going away: hand the remaining games over.
        if self.draining.is_some_and(|t| nowi >= t) {
            let running: Vec<u64> = self
                .rooms
                .values()
                .filter(|r| !r.finished && r.busy.is_none())
                .map(|r| r.id)
                .collect();
            for id in running {
                self.hand_off(id);
            }
        }
        if self.last_featured.is_none_or(|t| nowi.duration_since(t) > Duration::from_secs(4)) {
            self.last_featured = Some(nowi);
            self.update_featured();
        }
        self.codes.retain(|_, (_, at)| at.elapsed() < CODE_LIFETIME);
        for id in finished {
            self.finish(id);
            // The player who left is gone: close their side of the room.
            if let Some(room) = self.rooms.get_mut(&id) {
                for seat in 0..2 {
                    if room.disconnected[seat].is_some() {
                        room.present[seat] = false;
                    }
                }
            }
            self.close_room_if_empty(id);
        }
        // Finished rooms nobody is looking at any more.
        let stale: Vec<u64> = self
            .rooms
            .values()
            .filter(|r| {
                r.finished
                    && (0..2).all(|s| {
                        !r.present[s]
                            || self.sessions.get(&r.keys[s]).is_none_or(|x| x.room != Some(r.id))
                    })
            })
            .map(|r| r.id)
            .collect();
        for id in stale {
            self.rooms.remove(&id);
        }
        self.tournaments_tick(nowi);
    }
}

/// Restarts, the featured table, invitation codes, game limits.
impl Lounge {
    /// Refuses a new game while the server is restarting (with a message).
    fn refuse_if_draining(&self, k: &str) -> bool {
        if self.draining.is_some() {
            self.error(k, "The server is restarting: new games start in a moment.");
        }
        self.draining.is_some()
    }

    /// Seconds a player who often leaves games waits in the queue before
    /// being paired (none for most players).
    fn abandon_penalty(&self, k: &str) -> f32 {
        let Some(s) = self.sessions.get(k) else { return 0.0 };
        let u = &s.user;
        let played = (u.games + u.casual_games).max(1);
        if u.abandons < 3 {
            return 0.0;
        }
        let rate = u.abandons as f32 / played as f32;
        // 10% of games left: 30 s; 30% or more: the full 90 s.
        (rate * 300.0).min(90.0)
    }

    /// Counts a rated game between these two; returns how many they played
    /// in the last day before this one.
    fn count_pair_game(&mut self, keys: &[String; 2]) -> usize {
        let mut pair = [keys[0].clone(), keys[1].clone()];
        pair.sort();
        let now = Instant::now();
        let list = self.pair_games.entry((pair[0].clone(), pair[1].clone())).or_default();
        while list.front().is_some_and(|t| now.duration_since(*t) > PAIR_WINDOW) {
            list.pop_front();
        }
        let before = list.len();
        list.push_back(now);
        if self.pair_games.len() > 10_000 {
            self.pair_games
                .retain(|_, l| l.back().is_some_and(|t| now.duration_since(*t) < PAIR_WINDOW));
        }
        before
    }

    /// The server got the signal to stop: no new games; games in progress
    /// go on for up to `secs` and are then handed to the next server.
    pub fn start_drain(&mut self, secs: u64) {
        if self.draining.is_some() {
            return;
        }
        self.draining = Some(Instant::now() + Duration::from_secs(secs));
        let minutes = secs.div_ceil(60).max(1);
        self.announce_text(&format!(
            "The server restarts in {minutes} min for an update. Games in progress go on; new games start right after."
        ));
        self.broadcast(&ServerMsg::Draining { secs: secs as u32 });
        let queued: Vec<String> = self.queue.iter().map(|q| q.key.clone()).collect();
        for k in queued {
            self.queue_leave(&k, true);
        }
        let hosts: Vec<String> = self.tables.values().map(|t| t.host.clone()).collect();
        for h in hosts {
            self.close_table_of(&h);
            self.announce(&h);
        }
        self.invites.clear();
    }

    pub fn has_room(&self, id: u64) -> bool {
        self.rooms.contains_key(&id)
    }

    pub fn is_draining(&self) -> bool {
        self.draining.is_some()
    }

    /// Games not finished yet.
    pub fn running_games(&self) -> usize {
        self.rooms.values().filter(|r| !r.finished).count()
    }

    /// Hands a game in progress to the next server: it stays saved, both
    /// players are disconnected (their game reconnects to the new server,
    /// which rebuilds the table) and the room is closed here.
    fn hand_off(&mut self, id: u64) {
        let Some(room) = self.rooms.remove(&id) else { return };
        if room.finished || room.guests.iter().any(|g| *g) {
            self.rooms.insert(id, room);
            return;
        }
        tracing::info!(room = id, "game handed to the next server");
        for k in &room.keys {
            if let Some(set) = self.watchers.remove(k) {
                for w in set {
                    if let Some(s) = self.sessions.get_mut(&w) {
                        s.watching = None;
                    }
                    self.send(
                        &w,
                        ServerMsg::SpectateEnded { reason: "The server is restarting.".into() },
                    );
                }
            }
            if let Some(s) = self.sessions.get_mut(k)
                && s.room == Some(id)
            {
                s.room = None;
                // Closing the connection: the game reconnects by itself.
                s.kill.notify_one();
            }
        }
    }

    /// Rebuilds a saved game after a restart (or a hand-over). The players
    /// are away until they reconnect. False if it is already here.
    pub fn restore(&mut self, live: &LiveMatch, users: [User; 2], rebuilt: Rebuilt) -> bool {
        let id = live.id as u64;
        let keys = [key(&users[0].username), key(&users[1].username)];
        if self.rooms.contains_key(&id)
            || self.rooms.values().any(|r| !r.finished && r.keys.iter().any(|k| keys.contains(k)))
        {
            return false;
        }
        let nowi = Instant::now();
        let room = Room {
            id,
            keys: keys.clone(),
            users,
            guests: [false, false],
            rated: live.rated,
            game: rebuilt.game,
            deadline: nowi + RESTORE_GRACE + self.rules.shot_clock,
            disconnected: [Some(nowi), Some(nowi)],
            finished: false,
            rematch: [false, false],
            present: [true, true],
            breaker: live.replay.breaker,
            last_aim: None,
            busy: None,
            shot_at: nowi,
            turns: rebuilt.turns,
            tournament: None,
            chat: VecDeque::new(),
            crowd: Bucket::new(4.0, 0.6),
            cheers: HashMap::new(),
            replay: live.replay.clone(),
            stats: rebuilt.stats,
            bands: rebuilt.bands,
            table_look: Default::default(),
            grace_until: Some(nowi + RESTORE_GRACE),
        };
        self.rooms.insert(id, room);
        self.next_id = self.next_id.max(id + 1);
        for k in &keys {
            if let Some(s) = self.sessions.get_mut(k) {
                s.room = Some(id);
            }
        }
        tracing::info!(room = id, "game restored: {} vs {}", keys[0], keys[1]);
        true
    }

    /// Picks the game on the lounge's "TV": the highest rated game being
    /// played (tells everyone when it changes).
    fn update_featured(&mut self) {
        let best = self
            .rooms
            .values()
            .filter(|r| !r.finished && r.grace_until.is_none() && !r.guests.iter().any(|g| *g))
            .max_by_key(|r| (r.users[0].rating + r.users[1].rating, r.id));
        let featured = best.map(|r| {
            let player = |s: usize| match self.sessions.get(&r.keys[s]) {
                Some(x) => dressed(player_info(&x.user, x.guest, PlayerStatus::Playing), &x.look),
                None => player_info(&r.users[s], r.guests[s], PlayerStatus::Playing),
            };
            FeaturedMatch {
                match_id: r.id,
                players: [player(0), player(1)],
                rated: r.rated,
                spectators: self.spectators(r.id).len() as u32,
            }
        });
        if featured != self.featured {
            self.featured = featured.clone();
            self.broadcast(&ServerMsg::Featured { featured });
        }
    }

    /// `k` watches any game in progress (the featured table).
    fn watch_match(&mut self, k: &str, id: u64) {
        let Some(room) = self.rooms.get(&id).filter(|r| !r.finished) else {
            return self.error(k, "That game is over.");
        };
        if room.keys.iter().any(|p| p == k) {
            return;
        }
        if self.sessions.get(k).is_none_or(|s| s.room.is_some()) {
            return self.error(k, "Finish your game first.");
        }
        let target = room.keys[0].clone();
        if room.keys.iter().any(|p| self.blocks(p, k)) {
            return self.error(k, "You can't watch this game.");
        }
        self.stop_watching(k);
        self.watchers.entry(target.clone()).or_default().insert(k.to_string());
        if let Some(s) = self.sessions.get_mut(k) {
            s.watching = Some(target);
        }
        self.send_spectator_start(k, id, 0);
        self.announce_spectators(id);
    }

    /// Opens a private table whose password is a short code friends can use.
    fn create_code(&mut self, k: &str, rated: bool) {
        if self.refuse_if_draining(k) {
            return;
        }
        let mut rng = rand::thread_rng();
        let code: String = loop {
            let c: String = (0..6)
                .map(|_| CODE_LETTERS[rng.gen_range(0..CODE_LETTERS.len())] as char)
                .collect();
            if !self.codes.contains_key(&c) {
                break c;
            }
        };
        self.open_table(k, rated, Some(code.clone()));
        let Some(table) = self.sessions.get(k).and_then(|s| s.table) else { return };
        self.codes.insert(code.clone(), (table, Instant::now()));
        self.send(k, ServerMsg::InviteCode { code });
    }

    /// Sits at the table of an invitation code.
    fn join_by_code(&mut self, k: &str, code: &str) {
        let code: String = code
            .trim()
            .trim_start_matches("snookered://join/")
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_uppercase();
        let Some((table, _)) = self.codes.get(&code).copied() else {
            return self.error(k, "That code is not valid (or the table was closed).");
        };
        self.join_table(k, table, Some(code));
    }
}

/// A saved game replayed up to now (done away from the lounge: it simulates
/// every shot).
pub struct Rebuilt {
    pub game: EightBallGame,
    pub stats: [ShotStats; 2],
    pub bands: [Vec<(usize, bool)>; 2],
    pub turns: [u32; 2],
}

impl Rebuilt {
    pub fn from(replay: &Replay) -> Result<Rebuilt, String> {
        let mut game = replay.start();
        let mut stats = [ShotStats::default(); 2];
        let mut bands: [Vec<(usize, bool)>; 2] = Default::default();
        let mut turns = [0, 0];
        turns[replay.breaker % 2] = 1;
        for stroke in &replay.strokes {
            let before = game.clone();
            let shooter = game.current_player();
            let in_hand = game.ball_in_hand().is_some();
            let report = snookered_proto::replay::apply(&mut game, stroke)?;
            if let (Some(report), Stroke::Shot { shot, .. }) = (report, stroke) {
                stats[shooter].add(&report, in_hand);
                stats[shooter].add_spin(shot, &report);
                if let Some(b) = shot_difficulty(&before, shot, &report) {
                    bands[shooter].push(b);
                }
            }
            let next = game.current_player();
            if next != shooter {
                turns[next] += 1;
            }
        }
        if game.game_over().is_some() {
            return Err("the game is over".into());
        }
        Ok(Rebuilt { game, stats, bands, turns })
    }
}

impl Room {
    fn seat_of(&self, k: &str) -> usize {
        if self.keys[0] == k { 0 } else { 1 }
    }

    fn started_msg(
        &self,
        seat: usize,
        sessions: &HashMap<String, Session>,
        clock: u32,
        spectating: bool,
    ) -> ServerMsg {
        let player = |s: usize| match sessions.get(&self.keys[s]) {
            Some(x) => dressed(player_info(&x.user, x.guest, PlayerStatus::Playing), &x.look),
            None => player_info(&self.users[s], self.guests[s], PlayerStatus::Playing),
        };
        let left = self.deadline.saturating_duration_since(Instant::now()).as_secs_f32();
        ServerMsg::MatchStarted {
            match_id: self.id,
            seat,
            players: [player(0), player(1)],
            rated: self.rated,
            game: self.game.clone(),
            shot_clock_secs: clock,
            clock_left: left,
            tournament: self.tournament.map(|t| t.0),
            spectating,
            table_look: self.table_look.clone(),
        }
    }
}
