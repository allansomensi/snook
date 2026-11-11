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
use snook_core::rating::{self, EloConfig, Rating};
use snook_core::{BallId, EightBallGame, GameOverReason, PocketId, Shot, ShotReport};
use snook_proto::{
    AimState, ChatKind, ChatLine, ClientMsg, MAX_NAME_LEN, MAX_ROOM_PASSWORD, MatchResult,
    PlayerInfo, PlayerStatus, ServerMsg, TableInfo, TournamentInfo, TournamentState, clean_chat,
    clean_text,
};
use tokio::sync::Notify;
use tokio::sync::mpsc::{Sender, UnboundedSender, error::TrySendError};

use crate::limits::Bucket;
use crate::now;
use crate::store::{MatchRecord, TournamentRecord, User};
use crate::tournament::Bracket;

/// Tunable server rules.
#[derive(Debug, Clone)]
pub struct Rules {
    pub shot_clock: Duration,
    /// How long a disconnected player has to come back before forfeiting.
    pub reconnect_grace: Duration,
    /// Rated games between the same two players per day before games become casual.
    pub max_daily_rated_pair: u32,
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
}

impl Default for Rules {
    fn default() -> Self {
        Rules {
            shot_clock: Duration::from_secs(30),
            reconnect_grace: Duration::from_secs(60),
            max_daily_rated_pair: 3,
            chat_history: 60,
            motd: "Welcome to Snook! Be kind, play fair.".into(),
            lan: false,
            max_connections_per_ip: 12,
            tournament_no_show: Duration::from_secs(120),
            max_open_tournaments: 20,
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
    },
    RecordTournament(TournamentRecord),
}

/// Extra time after a shot's simulated duration before the next shot clock
/// starts (the players watch the balls stop and read the result).
const SHOT_MARGIN: Duration = Duration::from_millis(1500);
/// Aim and cue-ball updates are relayed at most this often.
const AIM_RELAY: Duration = Duration::from_millis(33);
const MATCH_CHAT_KEEP: usize = 40;

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
    last_aim: Option<Instant>,
    /// Achievement codes (they unlock reactions).
    achievements: Vec<String>,
    reaction_bucket: Bucket,
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
}

struct Table {
    id: u64,
    host: String,
    rated: bool,
    password: Option<String>,
}

struct Tournament {
    id: u64,
    name: String,
    host: String,
    size: u32,
    rated: bool,
    password: Option<String>,
    state: TournamentState,
    /// Lower-case usernames, in sign-up order.
    players: Vec<String>,
    /// Profiles of the players (for those who go offline).
    profiles: HashMap<String, (User, bool)>,
    bracket: Option<Bracket>,
    missing_since: HashMap<String, Instant>,
    finished_at: Option<Instant>,
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
    /// Rated games per pair of user ids in the last day.
    rated_pairs: HashMap<(i64, i64), VecDeque<Instant>>,
    effects: Option<UnboundedSender<Effect>>,
    next_guest_id: i64,
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
    }
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
            next_id: 1,
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
        self.sessions.get(k).map(|s| player_info(&s.user, s.guest, self.status(k)))
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
                last_aim: None,
                achievements: Vec::new(),
                reaction_bucket: Bucket::new(3.0, 0.5),
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
        self.announce(&k);
        // Back to a running match.
        if let Some(id) = room {
            let clock = self.rules.shot_clock.as_secs() as u32;
            let room = self.rooms.get_mut(&id).expect("room exists");
            let seat = room.seat_of(&k);
            room.disconnected[seat] = None;
            let opponent = room.keys[1 - seat].clone();
            let msg = room.started_msg(seat, &self.sessions, clock);
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
        let session = self.sessions.remove(&k).expect("checked");
        if let Some(id) = session.room
            && let Some(room) = self.rooms.get_mut(&id)
        {
            let seat = room.seat_of(&k);
            let opponent = room.keys[1 - seat].clone();
            if room.finished {
                room.present[seat] = false;
                self.send(&opponent, ServerMsg::OpponentLeft);
                self.close_room_if_empty(id);
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
            from: "Snook".into(),
            text,
            at: now(),
            kind: ChatKind::Announcement,
            rating: 0,
        };
        self.push_chat(line.clone());
        self.broadcast(&ServerMsg::Chat { line });
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
        match msg {
            ClientMsg::Chat { text } => self.chat(&k, &text),
            ClientMsg::MatchChat { text } => self.match_chat(&k, &text),
            ClientMsg::React { code } => self.react(&k, &code),
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
            ClientMsg::CreateTournament { name, size, rated, password } => {
                self.create_tournament(&k, &name, size, rated, password)
            }
            ClientMsg::JoinTournament { id, password } => self.join_tournament(&k, id, password),
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
            | ClientMsg::FriendRequest { .. }
            | ClientMsg::FriendAnswer { .. }
            | ClientMsg::FriendRemove { .. }
            | ClientMsg::Block { .. }
            | ClientMsg::Unblock { .. }
            | ClientMsg::Report { .. } => {}
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
        let Some(r) = snook_proto::reaction(code) else { return };
        let Some(s) = self.sessions.get_mut(k) else { return };
        if !r.unlocked(&s.achievements) || !s.reaction_bucket.take() {
            return;
        }
        if s.user.is_muted(now()) {
            return;
        }
        let sender = s.user.id;
        let Some(room) = self.room_of(k) else { return };
        let seat = room.seat_of(k);
        for p in room.keys.clone() {
            let blocked = self.sessions.get(&p).is_some_and(|s| s.blocked.contains(&sender));
            if !blocked {
                self.send(&p, ServerMsg::Reaction { seat, code: r.code.to_string() });
            }
        }
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
                TournamentState::Running => {
                    t.players.iter().any(|p| p == k)
                        && t.bracket.as_ref().is_some_and(|b| b.alive(k))
                }
                TournamentState::Finished => false,
            })
            .map(|t| t.id)
    }

    /// Still playing in a running tournament: no other games until knocked out.
    fn tournament_busy(&self, k: &str) -> bool {
        self.tournaments.values().any(|t| {
            t.state == TournamentState::Running
                && t.players.iter().any(|p| p == k)
                && t.bracket.as_ref().is_some_and(|b| b.alive(k))
        })
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
        self.start_room([from.to_string(), me.to_string()], rated, None);
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
        self.start_room([host, k.to_string()], rated, None);
    }

    fn rated_today(&mut self, a: i64, b: i64) -> u32 {
        let pair = (a.min(b), a.max(b));
        let list = self.rated_pairs.entry(pair).or_default();
        while list.front().is_some_and(|t| t.elapsed() > Duration::from_secs(86_400)) {
            list.pop_front();
        }
        list.len() as u32
    }

    fn start_room(
        &mut self,
        mut keys: [String; 2],
        mut rated: bool,
        tournament: Option<(u64, usize, usize)>,
    ) {
        if !keys.iter().all(|k| self.sessions.contains_key(k)) {
            return;
        }
        if tournament.is_none() && rand::thread_rng().gen_bool(0.5) {
            keys.swap(0, 1);
        }
        for k in &keys {
            self.drop_invites_of(k);
            self.close_table_of(k);
        }
        let users = [self.sessions[&keys[0]].user.clone(), self.sessions[&keys[1]].user.clone()];
        let guests = [self.sessions[&keys[0]].guest, self.sessions[&keys[1]].guest];
        rated &= !guests[0] && !guests[1];
        if rated
            && tournament.is_none()
            && self.rated_today(users[0].id, users[1].id) >= self.rules.max_daily_rated_pair
        {
            rated = false;
            for k in &keys {
                self.error(
                    k,
                    "You've played many rated games against each other today: this one is casual.",
                );
            }
        }
        let id = self.new_id();
        self.create_room(id, keys, users, guests, rated, 0, tournament);
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
        };
        let clock = self.rules.shot_clock.as_secs() as u32;
        for k in &keys {
            if let Some(s) = self.sessions.get_mut(k) {
                s.room = Some(id);
            }
        }
        for (seat, k) in keys.iter().enumerate() {
            let msg = room.started_msg(seat, &self.sessions, clock);
            self.send(k, msg);
        }
        self.rooms.insert(id, room);
        for k in &keys {
            self.announce(k);
        }
        tracing::info!(room = id, rated, "match started: {} vs {}", keys[0], keys[1]);
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
        let msg =
            ServerMsg::ShotTaken { seat, turn, shot, called_pocket: called, cue_ball, timeout };
        let game = Box::new(room.game.clone());
        for p in room.keys.clone() {
            self.send(&p, msg.clone());
        }
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
                let msg = ServerMsg::ShotRejected {
                    turn,
                    reason: "The server could not play that shot.".into(),
                    game: room.game.clone(),
                };
                room.deadline = Instant::now() + clock;
                for p in room.keys.clone() {
                    self.send(&p, msg.clone());
                }
                return;
            }
        };
        let shooter = room.game.current_player();
        room.game = game;
        let next = room.game.current_player();
        if next != shooter {
            room.turns[next] += 1;
        }
        room.deadline = room.shot_at + Duration::from_secs_f64(duration) + SHOT_MARGIN + clock;
        let clock_left = room.deadline.saturating_duration_since(Instant::now()).as_secs_f32();
        let msg = ServerMsg::ShotResult { turn, end: room.game.clone(), report, clock_left };
        let over = room.game.game_over().is_some();
        for p in room.keys.clone() {
            self.send(&p, msg.clone());
        }
        if over {
            self.finish(id);
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
                room.turns[room.game.current_player()] += 1;
                room.deadline = Instant::now() + clock;
                room.last_aim = None;
                let msg = ServerMsg::TimeFoul {
                    seat,
                    game: room.game.clone(),
                    clock_left: clock.as_secs_f32(),
                };
                for k in room.keys.clone() {
                    self.send(&k, msg.clone());
                }
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
        if rated {
            let (w, l) = (over.winner, 1 - over.winner);
            let change =
                rating::apply_result(&EloConfig::default(), users[w].rating(), users[l].rating());
            let (rw, rl): (Rating, Rating) = (change.winner, change.loser);
            delta[w] = change.winner_delta;
            delta[l] = change.loser_delta;
            new_ratings[w] = rw.rating;
            new_ratings[l] = rl.rating;
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
        };
        let keys = room.keys.clone();
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
        if rated {
            let pair = (users[0].id.min(users[1].id), users[0].id.max(users[1].id));
            self.rated_pairs.entry(pair).or_default().push_back(Instant::now());
        }
        // Show the new numbers at once; the database catches up.
        for (seat, k) in keys.iter().enumerate() {
            if let Some(s) = self.sessions.get_mut(k) {
                crate::store::apply_match_numbers(&mut s.user, &record, seat);
            }
        }
        for k in &keys {
            self.send(k, ServerMsg::MatchEnded { result: result.clone() });
            self.announce(k);
        }
        if !guests[0] && !guests[1] {
            self.effect(Effect::RecordMatch { record, keys: keys.clone() });
        }
        tracing::info!(room = id, "match finished: {:?}", over);
        if let Some((tid, round, index)) = tournament {
            if let Some(b) = self.tournaments.get_mut(&tid).and_then(|t| t.bracket.as_mut()) {
                b.set_winner(
                    round,
                    index,
                    over.winner_seat_in(&keys, &b.rounds[round][index].players),
                );
            }
            self.tournament_progress(tid);
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
            self.rooms.remove(&id);
            for p in &keys {
                if let Some(s) = self.sessions.get_mut(p) {
                    s.room = None;
                }
            }
            let users =
                [self.sessions[&keys[0]].user.clone(), self.sessions[&keys[1]].user.clone()];
            let mut rated = rated;
            if rated
                && self.rated_today(users[0].id, users[1].id) >= self.rules.max_daily_rated_pair
            {
                rated = false;
                for k in &keys {
                    self.error(
                        k,
                        "You've played many rated games against each other today: this one is casual.",
                    );
                }
            }
            let new_id = self.new_id();
            self.create_room(new_id, keys, users, guests, rated, breaker, None);
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
        self.close_room_if_empty(id);
        self.announce(k);
        if let Some(tid) = self.in_tournament(k) {
            self.tournament_progress(tid);
        }
    }

    // Tournaments.

    fn tournament_info(&self, t: &Tournament) -> TournamentInfo {
        let profile = |k: &String| -> PlayerInfo {
            self.info(k).unwrap_or_else(|| {
                let (u, guest) = &t.profiles[k];
                player_info(u, *guest, PlayerStatus::Available)
            })
        };
        let display = |k: &str| {
            t.profiles.get(k).map(|(u, _)| u.username.clone()).unwrap_or_else(|| k.to_string())
        };
        TournamentInfo {
            id: t.id,
            name: t.name.clone(),
            host: display(&t.host),
            size: t.size,
            rated: t.rated,
            private: t.password.is_some(),
            state: t.state,
            players: t.players.iter().map(profile).collect(),
            rounds: t.bracket.as_ref().map(|b| b.to_info(display)).unwrap_or_default(),
            champion: t.bracket.as_ref().and_then(|b| b.champion()).map(|c| display(c)),
        }
    }

    fn broadcast_tournament(&self, id: u64) {
        if let Some(t) = self.tournaments.get(&id) {
            self.broadcast(&ServerMsg::TournamentUpdated { tournament: self.tournament_info(t) });
        }
    }

    fn create_tournament(
        &mut self,
        k: &str,
        name: &str,
        size: u32,
        rated: bool,
        password: Option<String>,
    ) {
        let name = clean_text(name, MAX_NAME_LEN);
        if name.chars().count() < 3 {
            return self.error(k, "Give the tournament a name (at least 3 characters).");
        }
        if ![4, 8, 16].contains(&size) {
            return self.error(k, "Tournaments have 4, 8 or 16 players.");
        }
        if self.in_tournament(k).is_some() {
            return self.error(k, "You are already in a tournament.");
        }
        let open =
            self.tournaments.values().filter(|t| t.state != TournamentState::Finished).count();
        if open >= self.rules.max_open_tournaments {
            return self.error(k, "There are too many tournaments right now. Join one!");
        }
        let id = self.new_id();
        let Some(s) = self.sessions.get(k) else { return };
        let mut profiles = HashMap::new();
        profiles.insert(k.to_string(), (s.user.clone(), s.guest));
        let t = Tournament {
            id,
            name,
            host: k.to_string(),
            size,
            rated: rated && !s.guest,
            password: clean_password(password),
            state: TournamentState::Registering,
            players: vec![k.to_string()],
            profiles,
            bracket: None,
            missing_since: HashMap::new(),
            finished_at: None,
        };
        self.tournaments.insert(id, t);
        self.broadcast_tournament(id);
    }

    fn join_tournament(&mut self, k: &str, id: u64, password: Option<String>) {
        let Some(t) = self.tournaments.get(&id) else {
            return self.error(k, "That tournament no longer exists.");
        };
        if t.state != TournamentState::Registering {
            return self.error(k, "That tournament has already started.");
        }
        if t.players.iter().any(|p| p == k) {
            return;
        }
        if t.players.len() >= t.size as usize {
            return self.error(k, "That tournament is full.");
        }
        if let Some(expected) = &t.password
            && !ct_eq(expected, &clean_password(password).unwrap_or_default())
        {
            return self.error(k, "Wrong tournament password.");
        }
        if self.in_tournament(k).is_some() {
            return self.error(k, "You are already in a tournament.");
        }
        if t.rated && self.is_guest(k) {
            return self.error(k, "Guests can't play rated tournaments.");
        }
        let Some(s) = self.sessions.get(k) else { return };
        let profile = (s.user.clone(), s.guest);
        let t = self.tournaments.get_mut(&id).expect("checked");
        t.players.push(k.to_string());
        t.profiles.insert(k.to_string(), profile);
        let full = t.players.len() >= t.size as usize;
        self.broadcast_tournament(id);
        if full {
            self.begin_tournament(id);
        }
    }

    fn leave_tournament(&mut self, k: &str, id: u64) {
        let Some(t) = self.tournaments.get_mut(&id) else { return };
        match t.state {
            TournamentState::Registering => {
                if t.host == k {
                    self.tournaments.remove(&id);
                    self.broadcast(&ServerMsg::TournamentRemoved { id });
                    return;
                }
                t.players.retain(|p| p != k);
                self.broadcast_tournament(id);
            }
            TournamentState::Running => {
                // Withdrawing: lose the current match.
                let Some(b) = t.bracket.as_mut() else { return };
                let mut room = None;
                for (r, round) in b.rounds.clone().iter().enumerate() {
                    for (i, slot) in round.iter().enumerate() {
                        if slot.winner.is_none()
                            && let Some(seat) =
                                slot.players.iter().position(|p| p.as_deref() == Some(k))
                        {
                            match slot.room {
                                Some(rid) => room = Some(rid),
                                None if slot.players[1 - seat].is_some() => {
                                    b.set_winner(r, i, 1 - seat)
                                }
                                None => {}
                            }
                        }
                    }
                }
                if room.is_some() {
                    self.resign(k);
                } else {
                    self.tournament_progress(id);
                }
            }
            TournamentState::Finished => {}
        }
    }

    fn start_tournament(&mut self, k: &str, id: u64) {
        let Some(t) = self.tournaments.get(&id) else { return };
        if t.host != k {
            return self.error(k, "Only the host can start the tournament.");
        }
        if t.state != TournamentState::Registering {
            return;
        }
        if t.players.len() < 2 {
            return self.error(k, "At least two players are needed.");
        }
        self.begin_tournament(id);
    }

    fn begin_tournament(&mut self, id: u64) {
        let ratings: HashMap<String, i32> = {
            let Some(t) = self.tournaments.get(&id) else { return };
            t.players
                .iter()
                .map(|p| {
                    let r = self
                        .sessions
                        .get(p)
                        .map(|s| s.user.rating)
                        .unwrap_or_else(|| t.profiles[p].0.rating);
                    (p.clone(), r)
                })
                .collect()
        };
        let t = self.tournaments.get_mut(&id).expect("checked");
        let mut seeded = t.players.clone();
        seeded.sort_by(|a, b| ratings[b].cmp(&ratings[a]).then(a.cmp(b)));
        t.bracket = Some(Bracket::new(&seeded));
        t.state = TournamentState::Running;
        // Open tables and invitations of the players are withdrawn.
        for p in seeded {
            self.close_table_of(&p);
            self.drop_invites_of(&p);
            self.announce(&p);
        }
        self.tournament_progress(id);
    }

    /// Starts every tournament match that can be played, gives walkovers to
    /// players whose opponent didn't show up, and crowns the champion.
    fn tournament_progress(&mut self, id: u64) {
        let Some(t) = self.tournaments.get(&id) else { return };
        if t.state != TournamentState::Running {
            return;
        }
        let Some(bracket) = t.bracket.as_ref() else { return };
        let no_show = self.rules.tournament_no_show;
        let mut starts = Vec::new();
        let mut walkovers = Vec::new();
        let mut missing_now = HashMap::new();
        for (r, i) in bracket.playable() {
            let slot = &bracket.rounds[r][i];
            if slot.room.is_some() {
                continue;
            }
            let players = [
                slot.players[0].clone().expect("playable"),
                slot.players[1].clone().expect("playable"),
            ];
            let online =
                [self.sessions.contains_key(&players[0]), self.sessions.contains_key(&players[1])];
            if online[0] && online[1] {
                if self.available(&players[0]) && self.available(&players[1]) {
                    starts.push((r, i, players));
                }
                continue;
            }
            let mut gone = [false, false];
            for seat in 0..2 {
                if !online[seat] {
                    let since =
                        t.missing_since.get(&players[seat]).copied().unwrap_or_else(Instant::now);
                    missing_now.insert(players[seat].clone(), since);
                    gone[seat] = since.elapsed() > no_show;
                }
            }
            match gone {
                [true, false] => walkovers.push((r, i, 1)),
                [false, true] | [true, true] => walkovers.push((r, i, 0)),
                _ => {}
            }
        }
        let t = self.tournaments.get_mut(&id).expect("checked");
        t.missing_since = missing_now;
        let rated = t.rated;
        if let Some(b) = t.bracket.as_mut() {
            for (r, i, seat) in walkovers {
                b.set_winner(r, i, seat);
            }
        }
        for (r, i, players) in starts {
            let room_id = self.new_id();
            if let Some(b) = self.tournaments.get_mut(&id).and_then(|t| t.bracket.as_mut()) {
                b.rounds[r][i].room = Some(room_id);
            }
            let users =
                [self.sessions[&players[0]].user.clone(), self.sessions[&players[1]].user.clone()];
            let guests = [self.sessions[&players[0]].guest, self.sessions[&players[1]].guest];
            for p in &players {
                self.drop_invites_of(p);
                self.close_table_of(p);
            }
            self.create_room(room_id, players, users, guests, rated, 0, Some((id, r, i)));
        }
        // A champion?
        let t = self.tournaments.get_mut(&id).expect("checked");
        let champion = t.bracket.as_ref().and_then(|b| b.champion()).cloned();
        if let Some(champion) = champion {
            t.state = TournamentState::Finished;
            t.finished_at = Some(Instant::now());
            let players: Vec<i64> = t
                .players
                .iter()
                .filter_map(|p| t.profiles.get(p))
                .filter(|(_, guest)| !guest)
                .map(|(u, _)| u.id)
                .collect();
            let record = TournamentRecord {
                name: t.name.clone(),
                host: t.profiles.get(&t.host).map(|(u, _)| u.id).unwrap_or(0),
                size: t.size,
                rated: t.rated,
                winner: t.profiles.get(&champion).filter(|(_, g)| !g).map(|(u, _)| u.id),
                players,
                finished_at: now(),
            };
            let name = t.name.clone();
            let display =
                t.profiles.get(&champion).map(|(u, _)| u.username.clone()).unwrap_or(champion);
            if !record.players.is_empty() {
                self.effect(Effect::RecordTournament(record));
            }
            self.announce_text(&format!("{display} won the tournament \u{201c}{name}\u{201d}!"));
        }
        self.broadcast_tournament(id);
    }

    /// Called periodically: shot clocks, abandoned games, tournaments.
    pub fn tick(&mut self) {
        let nowi = Instant::now();
        let grace = self.rules.reconnect_grace;
        let mut finished = Vec::new();
        let mut timeouts = Vec::new();
        for room in self.rooms.values_mut().filter(|r| !r.finished) {
            if let Some(seat) =
                (0..2).find(|s| room.disconnected[*s].is_some_and(|t| nowi - t > grace))
            {
                room.game.abandon(seat);
                finished.push(room.id);
                continue;
            }
            if room.busy.is_none() && nowi > room.deadline {
                timeouts.push(room.id);
            }
        }
        for id in timeouts {
            self.timeout_shot(id);
        }
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
        let running: Vec<u64> = self
            .tournaments
            .values()
            .filter(|t| t.state == TournamentState::Running)
            .map(|t| t.id)
            .collect();
        for id in running {
            self.tournament_progress(id);
        }
        let expired: Vec<u64> = self
            .tournaments
            .values()
            .filter(|t| t.finished_at.is_some_and(|f| f.elapsed() > Duration::from_secs(600)))
            .map(|t| t.id)
            .collect();
        for id in expired {
            self.tournaments.remove(&id);
            self.broadcast(&ServerMsg::TournamentRemoved { id });
        }
    }
}

trait WinnerSeat {
    fn winner_seat_in(&self, keys: &[String; 2], slot: &[Option<String>; 2]) -> usize;
}

impl WinnerSeat for snook_core::GameOver {
    /// The bracket seat of the room's winner (room seats and bracket seats
    /// may differ).
    fn winner_seat_in(&self, keys: &[String; 2], slot: &[Option<String>; 2]) -> usize {
        let winner = &keys[self.winner];
        if slot[1].as_deref() == Some(winner.as_str()) { 1 } else { 0 }
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
    ) -> ServerMsg {
        let player = |s: usize| match sessions.get(&self.keys[s]) {
            Some(x) => player_info(&x.user, x.guest, PlayerStatus::Playing),
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
        }
    }
}
