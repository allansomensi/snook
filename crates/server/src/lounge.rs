//! Lounge state: who is online, chat, invitations and match rooms.
//!
//! All methods are synchronous and run under one mutex; they never block on
//! I/O other than short SQLite queries. Messages to players go through each
//! session's unbounded channel, drained by its WebSocket writer task.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use glam::DVec2;
use rand::Rng;
use snook_core::rating::{self, EloConfig, Rating};
use snook_core::{BallId, EightBallGame};
use snook_proto::{
    AimState, ChatLine, ClientMsg, LeaderboardEntry, MatchResult, PlayerInfo, PlayerStatus,
    ServerMsg, clean_chat,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::db::{Db, UserRow, now};

/// Tunable server rules.
#[derive(Debug, Clone)]
pub struct Rules {
    pub shot_clock: Duration,
    /// How long a disconnected player has to come back before forfeiting.
    pub reconnect_grace: Duration,
    /// Rated games between the same two players per day before games become casual.
    pub max_daily_rated_pair: u32,
    pub chat_history: usize,
    pub chat_interval: Duration,
    pub motd: String,
}

impl Default for Rules {
    fn default() -> Self {
        Rules {
            shot_clock: Duration::from_secs(30),
            reconnect_grace: Duration::from_secs(60),
            max_daily_rated_pair: 5,
            chat_history: 50,
            chat_interval: Duration::from_millis(700),
            motd: "Welcome to Snook! Be kind, play fair.".into(),
        }
    }
}

pub struct Session {
    pub conn_id: u64,
    pub user: UserRow,
    pub tx: UnboundedSender<ServerMsg>,
    pub room: Option<u64>,
    last_chat: Option<Instant>,
    /// Aim and cue-ball updates are relayed at most every 40 ms.
    last_aim: Option<Instant>,
}

struct Room {
    id: u64,
    /// Lower-case usernames.
    keys: [String; 2],
    user_ids: [i64; 2],
    rated: bool,
    game: EightBallGame,
    deadline: Instant,
    disconnected: [Option<Instant>; 2],
    finished: bool,
    rematch: [bool; 2],
    present: [bool; 2],
    breaker: usize,
    /// Profiles at the start, shown while a player is disconnected.
    profiles: [PlayerInfo; 2],
    /// The shooter's latest aim this turn (played if the shot clock runs out).
    last_aim: Option<AimState>,
}

#[derive(Default)]
pub struct Lounge {
    pub rules: Rules,
    sessions: HashMap<String, Session>,
    /// (from, to) -> rated, lower-case names.
    invites: HashMap<(String, String), bool>,
    rooms: HashMap<u64, Room>,
    next_room: u64,
    chat: VecDeque<ChatLine>,
}

fn key(name: &str) -> String {
    name.to_lowercase()
}

impl Lounge {
    pub fn new(rules: Rules) -> Self {
        Lounge { rules, next_room: 1, ..Default::default() }
    }

    fn send(&self, k: &str, msg: ServerMsg) {
        if let Some(s) = self.sessions.get(k) {
            let _ = s.tx.send(msg);
        }
    }

    fn broadcast(&self, msg: &ServerMsg) {
        for s in self.sessions.values() {
            let _ = s.tx.send(msg.clone());
        }
    }

    fn info(&self, user: &UserRow) -> PlayerInfo {
        let playing = self.sessions.get(&key(&user.username)).is_some_and(|s| s.room.is_some());
        PlayerInfo {
            username: user.username.clone(),
            rating: user.rating,
            games: user.games,
            wins: user.wins,
            status: if playing { PlayerStatus::Playing } else { PlayerStatus::Available },
        }
    }

    fn announce(&self, k: &str) {
        if let Some(s) = self.sessions.get(k) {
            self.broadcast(&ServerMsg::PlayerUpdated { player: self.info(&s.user) });
        }
    }

    pub fn online_count(&self) -> usize {
        self.sessions.len()
    }

    /// A user finished logging in on connection `conn_id`.
    pub fn join(
        &mut self,
        conn_id: u64,
        user: UserRow,
        tx: UnboundedSender<ServerMsg>,
        token: String,
    ) {
        let k = key(&user.username);
        if let Some(old) = self.sessions.remove(&k) {
            let _ = old
                .tx
                .send(ServerMsg::Error { message: "You logged in from another place.".into() });
        }
        let room = self
            .rooms
            .values()
            .find(|r| r.keys.contains(&k) && r.present[r.seat_of(&k)])
            .map(|r| r.id);
        let info = PlayerInfo {
            username: user.username.clone(),
            rating: user.rating,
            games: user.games,
            wins: user.wins,
            status: if room.is_some() { PlayerStatus::Playing } else { PlayerStatus::Available },
        };
        let _ = tx.send(ServerMsg::AuthOk { player: info, token });
        let _ = tx.send(ServerMsg::Welcome { motd: self.rules.motd.clone() });
        self.sessions.insert(
            k.clone(),
            Session { conn_id, user, tx, room, last_chat: None, last_aim: None },
        );
        let players: Vec<PlayerInfo> = self.sessions.values().map(|s| self.info(&s.user)).collect();
        self.send(&k, ServerMsg::Lounge { players });
        self.send(&k, ServerMsg::ChatHistory { lines: self.chat.iter().cloned().collect() });
        self.announce(&k);
        // Reconnect to a running match.
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
    pub fn leave(&mut self, db: &Db, username: &str, conn_id: u64) {
        let k = key(username);
        if self.sessions.get(&k).is_none_or(|s| s.conn_id != conn_id) {
            return; // A newer connection replaced this one.
        }
        let session = self.sessions.remove(&k).expect("checked");
        self.drop_invites_of(&k);
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
        let _ = db;
        self.broadcast(&ServerMsg::PlayerLeft { username: session.user.username });
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

    fn display(&self, k: &str) -> String {
        self.sessions.get(k).map(|s| s.user.username.clone()).unwrap_or_else(|| k.to_string())
    }

    fn close_room_if_empty(&mut self, id: u64) {
        if self.rooms.get(&id).is_some_and(|r| !r.present[0] && !r.present[1]) {
            self.rooms.remove(&id);
        }
    }

    fn error(&self, k: &str, message: impl Into<String>) {
        self.send(k, ServerMsg::Error { message: message.into() });
    }

    /// Handles a message from a logged-in player.
    pub fn handle(&mut self, db: &Db, username: &str, msg: ClientMsg) {
        let k = key(username);
        match msg {
            ClientMsg::Chat { text } => self.chat(&k, &text),
            ClientMsg::MatchChat { text } => self.match_chat(&k, &text),
            ClientMsg::Invite { to, rated } => self.invite(&k, &key(&to), rated),
            ClientMsg::CancelInvite { to } => {
                let to = key(&to);
                if self.invites.remove(&(k.clone(), to.clone())).is_some() {
                    self.send(&to, ServerMsg::InviteCancelled { from: self.display(&k) });
                }
            }
            ClientMsg::AnswerInvite { from, accept } => self.answer(db, &k, &key(&from), accept),
            ClientMsg::PlaceCueBall { position } => self.place_cue_ball(&k, position),
            ClientMsg::Aim { aim } => self.aim(&k, aim),
            ClientMsg::Shoot { shot, called_pocket, cue_ball } => {
                self.shoot(db, &k, shot, called_pocket, cue_ball)
            }
            ClientMsg::Resign => self.resign(db, &k),
            ClientMsg::Rematch => self.rematch(db, &k),
            ClientMsg::LeaveMatch => self.leave_match(&k),
            ClientMsg::Leaderboard => match db.leaderboard(100) {
                Ok(entries) => self.send(&k, ServerMsg::Leaderboard { entries }),
                Err(e) => {
                    tracing::error!("leaderboard query failed: {e}");
                    self.error(&k, "The leaderboard is unavailable right now.");
                }
            },
            ClientMsg::Ping => self.send(&k, ServerMsg::Pong),
            // Handled by the connection before login.
            ClientMsg::Hello { .. }
            | ClientMsg::Register { .. }
            | ClientMsg::Login { .. }
            | ClientMsg::Resume { .. }
            | ClientMsg::Logout => {}
        }
    }

    fn chat(&mut self, k: &str, text: &str) {
        let text = clean_chat(text);
        if text.is_empty() {
            return;
        }
        let interval = self.rules.chat_interval;
        let Some(s) = self.sessions.get_mut(k) else { return };
        if s.last_chat.is_some_and(|t| t.elapsed() < interval) {
            self.error(k, "You're sending messages too quickly.");
            return;
        }
        s.last_chat = Some(Instant::now());
        let line = ChatLine { from: s.user.username.clone(), text, at: now() };
        self.chat.push_back(line.clone());
        while self.chat.len() > self.rules.chat_history {
            self.chat.pop_front();
        }
        self.broadcast(&ServerMsg::Chat { line });
    }

    fn match_chat(&mut self, k: &str, text: &str) {
        let text = clean_chat(text);
        let Some(room) = self.room_of(k) else { return };
        if text.is_empty() {
            return;
        }
        let line = ChatLine { from: self.display(k), text, at: now() };
        for p in room.keys.clone() {
            self.send(&p, ServerMsg::MatchChat { line: line.clone() });
        }
    }

    fn room_of(&self, k: &str) -> Option<&Room> {
        self.sessions.get(k).and_then(|s| s.room).and_then(|id| self.rooms.get(&id))
    }

    fn available(&self, k: &str) -> bool {
        self.sessions.get(k).is_some_and(|s| s.room.is_none())
    }

    fn invite(&mut self, from: &str, to: &str, rated: bool) {
        if from == to {
            return self.send(
                from,
                ServerMsg::InviteFailed { reason: "You can't invite yourself.".into() },
            );
        }
        if !self.sessions.contains_key(to) {
            return self.send(
                from,
                ServerMsg::InviteFailed { reason: "That player is not online.".into() },
            );
        }
        if !self.available(from) || !self.available(to) {
            return self.send(
                from,
                ServerMsg::InviteFailed { reason: "That player is in a game.".into() },
            );
        }
        if self.invites.contains_key(&(from.to_string(), to.to_string())) {
            return;
        }
        if let Some(rated_back) = self.invites.get(&(to.to_string(), from.to_string())).copied() {
            // They already invited us: treat this as accepting.
            let _ = rated_back;
            return self
                .send(from, ServerMsg::Invited { from: self.display(to), rated: rated_back });
        }
        self.invites.insert((from.to_string(), to.to_string()), rated);
        self.send(to, ServerMsg::Invited { from: self.display(from), rated });
        self.send(from, ServerMsg::InviteSent { to: self.display(to), rated });
    }

    fn answer(&mut self, db: &Db, me: &str, from: &str, accept: bool) {
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
        self.drop_invites_of(from);
        self.drop_invites_of(me);
        let mut seats = [from.to_string(), me.to_string()];
        if rand::thread_rng().gen_bool(0.5) {
            seats.swap(0, 1);
        }
        self.start_room(db, seats, rated, 0);
    }

    fn start_room(&mut self, db: &Db, keys: [String; 2], mut rated: bool, breaker: usize) {
        let user_ids = [self.sessions[&keys[0]].user.id, self.sessions[&keys[1]].user.id];
        if rated {
            let recent = db.recent_rated_between(user_ids[0], user_ids[1], 24 * 3600).unwrap_or(0);
            if recent >= self.rules.max_daily_rated_pair {
                rated = false;
                for k in &keys {
                    self.error(k, "You've played many rated games against each other today: this one is casual.");
                }
            }
        }
        let id = self.next_room;
        self.next_room += 1;
        let seed: u64 = rand::thread_rng().r#gen();
        let profiles =
            [self.info(&self.sessions[&keys[0]].user), self.info(&self.sessions[&keys[1]].user)];
        let room = Room {
            id,
            keys: keys.clone(),
            user_ids,
            rated,
            game: EightBallGame::new(seed, breaker),
            deadline: Instant::now() + self.rules.shot_clock,
            disconnected: [None, None],
            finished: false,
            rematch: [false, false],
            present: [true, true],
            breaker,
            profiles,
            last_aim: None,
        };
        let clock = self.rules.shot_clock.as_secs() as u32;
        for (seat, k) in keys.iter().enumerate() {
            if let Some(s) = self.sessions.get_mut(k) {
                s.room = Some(id);
            }
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
    fn my_turn(&mut self, k: &str) -> Option<(u64, usize)> {
        let id = self.sessions.get(k)?.room?;
        let room = self.rooms.get(&id)?;
        let seat = room.seat_of(k);
        (!room.finished && room.game.current_player() == seat).then_some((id, seat))
    }

    fn place_cue_ball(&mut self, k: &str, p: DVec2) {
        // The final position comes with the shot, so dropping some is harmless.
        if !self.aim_allowed(k) {
            return;
        }
        let Some((id, seat)) = self.my_turn(k) else { return };
        let room = self.rooms.get_mut(&id).expect("room");
        if room.game.ball_in_hand().is_some() && room.game.place_cue_ball(p) {
            let opponent = room.keys[1 - seat].clone();
            self.send(&opponent, ServerMsg::OpponentCueBall { position: p });
        }
    }

    /// True if `k` may relay another aim update now (drops floods).
    fn aim_allowed(&mut self, k: &str) -> bool {
        let Some(s) = self.sessions.get_mut(k) else { return false };
        let now = Instant::now();
        if s.last_aim.is_some_and(|t| now.duration_since(t) < Duration::from_millis(40)) {
            return false;
        }
        s.last_aim = Some(now);
        true
    }

    fn aim(&mut self, k: &str, aim: AimState) {
        let finite = aim.yaw.is_finite()
            && aim.elevation.is_finite()
            && aim.tip.is_finite()
            && aim.power.is_finite();
        if !finite {
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
        db: &Db,
        k: &str,
        shot: snook_core::Shot,
        called: Option<snook_core::PocketId>,
        cue_ball: Option<DVec2>,
    ) {
        let Some((id, seat)) = self.my_turn(k) else {
            return self.error(k, "It's not your turn.");
        };
        if let Err(e) = self.play_shot(db, id, seat, shot, called, cue_ball, false) {
            self.error(k, e);
        }
    }

    /// Plays a shot for `seat` in room `id` and sends the result to both players.
    #[allow(clippy::too_many_arguments)]
    fn play_shot(
        &mut self,
        db: &Db,
        id: u64,
        seat: usize,
        shot: snook_core::Shot,
        called: Option<snook_core::PocketId>,
        cue_ball: Option<DVec2>,
        timeout: bool,
    ) -> Result<(), String> {
        let clock = self.rules.shot_clock;
        let room = self.rooms.get_mut(&id).ok_or("No such game.")?;
        if let Some(p) = cue_ball
            && room.game.ball_in_hand().is_some()
            && !room.game.place_cue_ball(p)
            && room.game.ball(BallId::CUE).xy() != p
        {
            return Err("The cue ball can't be placed there.".into());
        }
        let start = room.game.clone();
        let mut pending =
            room.game.prepare_shot(&shot, called).map_err(|e| format!("Can't shoot: {e}."))?;
        let shot = pending.shot;
        pending.simulation.run_to_rest();
        let duration = pending.simulation.time();
        let report = room.game.finish_shot(pending);
        let end = room.game.clone();
        room.last_aim = None;
        // Leave time for the clients to animate the shot.
        room.deadline = Instant::now() + clock + Duration::from_secs_f64(duration + 2.0);
        let clock_left = (clock.as_secs_f64() + duration + 2.0) as f32;
        let msg = ServerMsg::ShotPlayed {
            seat,
            shot,
            called_pocket: called,
            start,
            end,
            report,
            clock_left,
            timeout,
        };
        let keys = room.keys.clone();
        let over = room.game.game_over().is_some();
        for p in &keys {
            self.send(p, msg.clone());
        }
        if over {
            self.finish(db, id);
        }
        Ok(())
    }

    /// The shot clock ran out: play the shot at full power where the player
    /// was aiming (or at the nearest legal ball if they never aimed).
    fn timeout_shot(&mut self, db: &Db, id: u64) {
        let Some(room) = self.rooms.get(&id) else { return };
        let seat = room.game.current_player();
        let game = &room.game;
        let aim = room.last_aim;
        let max = game.params().max_cue_speed;
        let shot = snook_core::Shot {
            yaw: aim.map(|a| a.yaw).unwrap_or_else(|| game.default_aim()),
            elevation: aim.map(|a| a.elevation).unwrap_or(0.0),
            tip: aim.map(|a| a.tip).unwrap_or(DVec2::ZERO),
            speed: max,
        };
        let called = game.needs_called_pocket().then(|| {
            aim.and_then(|a| a.called_pocket).unwrap_or_else(|| game.nearest_pocket(BallId::EIGHT))
        });
        if self.play_shot(db, id, seat, shot, called, None, true).is_err() {
            // Should not happen; fall back to passing the turn.
            let clock = self.rules.shot_clock;
            if let Some(room) = self.rooms.get_mut(&id) {
                room.game.time_foul();
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

    fn resign(&mut self, db: &Db, k: &str) {
        let Some(id) = self.sessions.get(k).and_then(|s| s.room) else { return };
        let Some(room) = self.rooms.get_mut(&id) else { return };
        if room.finished {
            return;
        }
        let seat = room.seat_of(k);
        room.game.resign(seat);
        self.finish(db, id);
    }

    /// Settles ratings and notifies both players.
    fn finish(&mut self, db: &Db, id: u64) {
        let Some(room) = self.rooms.get_mut(&id) else { return };
        let Some(over) = room.game.game_over() else { return };
        if room.finished {
            return;
        }
        room.finished = true;
        room.rematch = [false, false];
        let (rated, user_ids, shots) = (room.rated, room.user_ids, room.game.shots_taken());
        let keys = room.keys.clone();
        let users: Vec<Option<UserRow>> =
            user_ids.iter().map(|id| db.user_by_id(*id).ok().flatten()).collect();
        let mut delta = [0, 0];
        let mut new_ratings = [0, 0];
        let config = EloConfig::default();
        if let (Some(a), Some(b)) = (&users[0], &users[1]) {
            new_ratings = [a.rating, b.rating];
            if rated {
                let (w, l) = if over.winner == 0 { (a, b) } else { (b, a) };
                let change = rating::apply_result(&config, w.rating(), l.rating());
                let (rw, rl): (Rating, Rating) = (change.winner, change.loser);
                let _ = db.update_rating(w.id, rw, true);
                let _ = db.update_rating(l.id, rl, false);
                delta[over.winner] = change.winner_delta;
                delta[1 - over.winner] = change.loser_delta;
                new_ratings[over.winner] = rw.rating;
                new_ratings[1 - over.winner] = rl.rating;
            }
        }
        let reason = format!("{:?}", over.reason);
        if let Err(e) = db.record_match(user_ids, over.winner, &reason, rated, delta, shots) {
            tracing::error!("could not record match: {e}");
        }
        // Refresh the cached profiles.
        for (k, uid) in keys.iter().zip(user_ids) {
            if let (Some(s), Ok(Some(fresh))) = (self.sessions.get_mut(k), db.user_by_id(uid)) {
                s.user = fresh;
            }
        }
        let result = MatchResult {
            winner: over.winner,
            reason: over.reason,
            rated,
            rating_delta: delta,
            new_ratings,
        };
        for k in &keys {
            self.send(k, ServerMsg::MatchEnded { result: result.clone() });
            self.announce(k);
        }
        tracing::info!(room = id, "match finished: {:?}", over);
    }

    fn rematch(&mut self, db: &Db, k: &str) {
        let Some(id) = self.sessions.get(k).and_then(|s| s.room) else { return };
        let Some(room) = self.rooms.get_mut(&id) else { return };
        if !room.finished {
            return;
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
            self.rooms.remove(&id);
            for p in &keys {
                if let Some(s) = self.sessions.get_mut(p) {
                    s.room = None;
                }
            }
            self.start_room(db, keys, rated, breaker);
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
    }

    /// Called periodically: shot clocks and abandoned games.
    pub fn tick(&mut self, db: &Db) {
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
            if room.disconnected.iter().any(Option::is_some) {
                // The clock is paused while someone is reconnecting.
                room.deadline = room.deadline.max(nowi + Duration::from_secs(5));
                continue;
            }
            if nowi > room.deadline {
                timeouts.push(room.id);
            }
        }
        for id in timeouts {
            self.timeout_shot(db, id);
        }
        for id in finished {
            self.finish(db, id);
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
    }

    pub fn leaderboard(db: &Db) -> Vec<LeaderboardEntry> {
        db.leaderboard(100).unwrap_or_default()
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
        let player = |k: &String| match sessions.get(k) {
            Some(s) => PlayerInfo {
                username: s.user.username.clone(),
                rating: s.user.rating,
                games: s.user.games,
                wins: s.user.wins,
                status: PlayerStatus::Playing,
            },
            None => {
                let seat = if self.keys[0] == *k { 0 } else { 1 };
                PlayerInfo { status: PlayerStatus::Playing, ..self.profiles[seat].clone() }
            }
        };
        let left = self.deadline.saturating_duration_since(Instant::now()).as_secs_f32();
        ServerMsg::MatchStarted {
            match_id: self.id,
            seat,
            players: [player(&self.keys[0]), player(&self.keys[1])],
            rated: self.rated,
            game: self.game.clone(),
            shot_clock_secs: clock,
            clock_left: left,
        }
    }
}
