//! Tournaments: knockout brackets and arenas, entry fees and prizes, and the
//! cup the server organises every week.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use snookered_proto::{
    ArenaStanding, MAX_NAME_LEN, PlayerInfo, PlayerStatus, ServerMsg, TournamentFormat,
    TournamentInfo, TournamentState, clean_text,
};

use super::{Effect, HOSTLESS_TOURNAMENT, Lounge, clean_password, ct_eq, player_info};
use crate::now;
use crate::store::{TournamentRecord, User};
use crate::tournament::Bracket;

/// The round number arena games are filed under (they have no bracket).
pub(super) const ARENA_ROUND: usize = usize::MAX;
/// Most players in an arena.
const ARENA_MAX: usize = 64;
/// How long an arena lasts (the weekly cup sets its own).
const ARENA_LENGTH: Duration = Duration::from_secs(30 * 60);
/// The entry fee goes to the prizes: 70% to the champion, 20% to the
/// runner-up; the rest leaves circulation (the house's share).
const PRIZE_SHARES: [u32; 2] = [70, 20];
pub const MAX_ENTRY_FEE: u32 = 1000;
/// The weekly cup opens for sign-up this long before it starts.
const CUP_SIGN_UP: i64 = 2 * 3600;

/// The weekly cup: an arena organised by the server.
#[derive(Debug, Clone, PartialEq)]
pub struct Cup {
    pub name: String,
    /// Day of the week (0 = Sunday … 6 = Saturday) and local time.
    pub weekday: u32,
    pub hour: u32,
    pub minute: u32,
    /// Local time minus UTC, in minutes (Brasília: -180).
    pub utc_offset: i32,
    pub minutes: u32,
    pub entry_fee: u32,
}

impl Cup {
    /// From `SNOOKERED_CUP`-style text: `sat 20:00` (optionally `60m`, `fee=100`,
    /// `utc-3`).
    pub fn parse(text: &str) -> Option<Cup> {
        let mut cup = Cup {
            name: "Saturday Cup".into(),
            weekday: 6,
            hour: 20,
            minute: 0,
            utc_offset: -180,
            minutes: 60,
            entry_fee: 0,
        };
        let days = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
        let names = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
        let mut any = false;
        for part in text.split_whitespace() {
            let p = part.to_ascii_lowercase();
            if let Some(d) = days.iter().position(|d| p.starts_with(d)) {
                cup.weekday = d as u32;
                cup.name = format!("{} Cup", names[d]);
                any = true;
            } else if let Some((h, m)) = p.split_once(':') {
                cup.hour = h.parse().ok().filter(|h| *h < 24)?;
                cup.minute = m.parse().ok().filter(|m| *m < 60)?;
                any = true;
            } else if let Some(m) = p.strip_suffix('m') {
                cup.minutes = m.parse().ok().filter(|m| (10..=180).contains(m))?;
            } else if let Some(f) = p.strip_prefix("fee=") {
                cup.entry_fee = f.parse().ok().filter(|f| *f <= MAX_ENTRY_FEE)?;
            } else {
                let o = p.strip_prefix("utc")?;
                let hours: i32 = if o.is_empty() { 0 } else { o.parse().ok()? };
                cup.utc_offset = hours * 60;
            }
        }
        any.then_some(cup)
    }

    /// The next start at or after `t` (Unix seconds).
    pub fn next_start(&self, t: i64) -> i64 {
        let local = t + self.utc_offset as i64 * 60;
        let day = local.div_euclid(86_400);
        // 1970-01-01 was a Thursday (4).
        let weekday = (day + 4).rem_euclid(7);
        let ahead = (self.weekday as i64 - weekday).rem_euclid(7);
        let at = |d: i64| {
            (day + d) * 86_400 + self.hour as i64 * 3600 + self.minute as i64 * 60
                - self.utc_offset as i64 * 60
        };
        let start = at(ahead);
        if start >= t { start } else { at(ahead + 7) }
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct ArenaScore {
    points: u32,
    games: u32,
    streak: u32,
    /// The last opponent (not paired again straight away).
    last: Option<String>,
    /// Free to play since (waiting for an opponent).
    idle_since: Option<Instant>,
}

/// Two players who just met are paired again only after waiting this long
/// (when nobody else is free).
const ARENA_REMATCH_WAIT: Duration = Duration::from_secs(15);

pub(super) struct Tournament {
    pub(super) id: u64,
    name: String,
    /// Empty for the server's own cup.
    host: String,
    size: u32,
    rated: bool,
    password: Option<String>,
    pub(super) state: TournamentState,
    /// Lower-case usernames, in sign-up order.
    pub(super) players: Vec<String>,
    /// Profiles of the players (for those who go offline).
    profiles: HashMap<String, (User, bool)>,
    pub(super) bracket: Option<Bracket>,
    missing_since: HashMap<String, Instant>,
    finished_at: Option<Instant>,
    format: TournamentFormat,
    entry_fee: u32,
    /// Who paid the entry fee (refunded if they leave before the start).
    paid: HashSet<String>,
    /// Full: who is waiting for a seat (VIP members first).
    waitlist: Vec<String>,
    /// Organised by the server: starts by itself at this time.
    starts_at: Option<i64>,
    /// Arena: closes at this time (Unix seconds).
    ends_at: Option<i64>,
    arena: HashMap<String, ArenaScore>,
    /// Arena players who left (they keep their points).
    withdrawn: HashSet<String>,
    champion: Option<String>,
}

impl Tournament {
    fn pot(&self) -> u32 {
        self.entry_fee * self.paid.len() as u32
    }

    fn prizes(&self) -> [u32; 2] {
        let pot = self.pot();
        PRIZE_SHARES.map(|share| pot * share / 100)
    }

    /// Arena standings, best first.
    fn standings(&self, rating: impl Fn(&str) -> i32) -> Vec<(String, ArenaScore)> {
        let mut list: Vec<(String, ArenaScore)> =
            self.arena.iter().map(|(k, s)| (k.clone(), s.clone())).collect();
        list.sort_by(|a, b| {
            b.1.points
                .cmp(&a.1.points)
                .then(a.1.games.cmp(&b.1.games))
                .then(rating(&b.0).cmp(&rating(&a.0)))
                .then(a.0.cmp(&b.0))
        });
        list
    }

    fn is_arena(&self) -> bool {
        self.format == TournamentFormat::Arena
    }

    /// Still playing (not knocked out, not withdrawn).
    pub(super) fn alive(&self, k: &str) -> bool {
        if !self.players.iter().any(|p| p == k) {
            return false;
        }
        !self.withdrawn.contains(k)
            && (self.is_arena() || self.bracket.as_ref().is_some_and(|b| b.alive(k)))
    }
}

/// What happened to a request to join a tournament.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Joined {
    In,
    Waiting,
    No,
}

impl Lounge {
    pub(super) fn tournament_info(&self, t: &Tournament) -> TournamentInfo {
        let profile = |k: &String| -> PlayerInfo {
            self.info(k).unwrap_or_else(|| {
                let (u, guest) = &t.profiles[k];
                player_info(u, *guest, PlayerStatus::Available)
            })
        };
        let display = |k: &str| {
            t.profiles.get(k).map(|(u, _)| u.username.clone()).unwrap_or_else(|| k.to_string())
        };
        let rating = |k: &str| t.profiles.get(k).map(|(u, _)| u.rating).unwrap_or(0);
        TournamentInfo {
            id: t.id,
            name: t.name.clone(),
            host: if t.host.is_empty() {
                snookered_proto::GAME_NAME.into()
            } else {
                display(&t.host)
            },
            size: t.size,
            rated: t.rated,
            private: t.password.is_some(),
            state: t.state,
            players: t.players.iter().map(profile).collect(),
            rounds: t.bracket.as_ref().map(|b| b.to_info(display)).unwrap_or_default(),
            champion: t
                .bracket
                .as_ref()
                .and_then(|b| b.champion())
                .or(t.champion.as_ref())
                .map(|c| display(c)),
            entry_fee: t.entry_fee,
            prizes: t.prizes(),
            starts_at: t.starts_at,
            format: t.format,
            standings: if t.is_arena() {
                t.standings(rating)
                    .into_iter()
                    .map(|(k, s)| ArenaStanding {
                        username: display(&k),
                        points: s.points,
                        games: s.games,
                        streak: s.streak,
                    })
                    .collect()
            } else {
                Vec::new()
            },
            ends_at: t.ends_at,
        }
    }

    pub(super) fn broadcast_tournament(&self, id: u64) {
        if let Some(t) = self.tournaments.get(&id) {
            self.broadcast(&ServerMsg::TournamentUpdated { tournament: self.tournament_info(t) });
        }
    }

    /// The entry fee of a tournament still taking players (0: free or none).
    pub fn tournament_fee(&self, id: u64) -> u32 {
        self.tournaments
            .get(&id)
            .filter(|t| t.state == TournamentState::Registering || t.is_arena())
            .map(|t| t.entry_fee)
            .unwrap_or(0)
    }

    /// Creates a tournament; `paid`: the host already paid the entry fee.
    /// Returns its id.
    #[allow(clippy::too_many_arguments)]
    pub fn create_tournament(
        &mut self,
        k: &str,
        name: &str,
        size: u32,
        rated: bool,
        password: Option<String>,
        entry_fee: u32,
        format: TournamentFormat,
        paid: bool,
    ) -> Option<u64> {
        if self.refuse_if_draining(k) {
            return None;
        }
        let name = clean_text(name, MAX_NAME_LEN);
        if name.chars().count() < 3 {
            self.error(k, "Give the tournament a name (at least 3 characters).");
            return None;
        }
        let arena = format == TournamentFormat::Arena;
        if !arena && ![4, 8, 16].contains(&size) {
            self.error(k, "Tournaments have 4, 8 or 16 players.");
            return None;
        }
        let vip = self.sessions.get(k).is_some_and(|s| s.look.vip);
        if size == 16 && !arena && !vip && !self.rules.lan {
            self.error(k, "16-player tournaments are for VIP members.");
            return None;
        }
        if entry_fee > MAX_ENTRY_FEE || (entry_fee > 0 && !paid) {
            self.error(k, "Entry fees go up to 1000 coins.");
            return None;
        }
        if self.in_tournament(k).is_some() {
            self.error(k, "You are already in a tournament.");
            return None;
        }
        let open =
            self.tournaments.values().filter(|t| t.state != TournamentState::Finished).count();
        if open >= self.rules.max_open_tournaments {
            self.error(k, "There are too many tournaments right now. Join one!");
            return None;
        }
        let id = self.new_id();
        let s = self.sessions.get(k)?;
        if entry_fee > 0 && s.guest {
            self.error(k, "Guests can't play for coins.");
            return None;
        }
        let mut profiles = HashMap::new();
        profiles.insert(k.to_string(), (s.user.clone(), s.guest));
        let t = Tournament {
            id,
            name,
            host: k.to_string(),
            size: if arena { ARENA_MAX as u32 } else { size },
            rated: rated && !s.guest,
            password: clean_password(password),
            state: TournamentState::Registering,
            players: vec![k.to_string()],
            profiles,
            bracket: None,
            missing_since: HashMap::new(),
            finished_at: None,
            format,
            entry_fee,
            paid: if entry_fee > 0 { HashSet::from([k.to_string()]) } else { HashSet::new() },
            waitlist: Vec::new(),
            starts_at: None,
            ends_at: None,
            arena: HashMap::new(),
            withdrawn: HashSet::new(),
            champion: None,
        };
        self.tournaments.insert(id, t);
        self.broadcast_tournament(id);
        Some(id)
    }

    /// Joins a tournament (`paid`: the fee was taken). A full one puts the
    /// player on its waiting list; an arena can be joined while it runs.
    pub fn join_tournament(
        &mut self,
        k: &str,
        id: u64,
        password: Option<String>,
        paid: bool,
    ) -> Joined {
        let Some(t) = self.tournaments.get(&id) else {
            self.error(k, "That tournament no longer exists.");
            return Joined::No;
        };
        let late_arena = t.is_arena()
            && t.state == TournamentState::Running
            && t.ends_at.is_some_and(|e| now() < e - 300);
        if t.state != TournamentState::Registering && !late_arena {
            self.error(k, "That tournament has already started.");
            return Joined::No;
        }
        if t.players.iter().any(|p| p == k) || t.waitlist.iter().any(|p| p == k) {
            if t.withdrawn.contains(k) {
                // Back in the arena.
                let t = self.tournaments.get_mut(&id).expect("checked");
                t.withdrawn.remove(k);
                self.broadcast_tournament(id);
                return Joined::In;
            }
            return Joined::No;
        }
        if t.entry_fee > 0 && !paid {
            self.error(k, "Not enough coins.");
            return Joined::No;
        }
        if let Some(expected) = &t.password
            && !ct_eq(expected, &clean_password(password).unwrap_or_default())
        {
            self.error(k, "Wrong tournament password.");
            return Joined::No;
        }
        if self.in_tournament(k).is_some() {
            self.error(k, "You are already in a tournament.");
            return Joined::No;
        }
        if self.refuse_if_draining(k) {
            return Joined::No;
        }
        let Some(s) = self.sessions.get(k) else { return Joined::No };
        if t.rated && s.guest {
            self.error(k, "Guests can't play rated tournaments.");
            return Joined::No;
        }
        if t.entry_fee > 0 && s.guest {
            self.error(k, "Guests can't play for coins.");
            return Joined::No;
        }
        let profile = (s.user.clone(), s.guest);
        let full = t.players.len() >= t.size as usize;
        let t = self.tournaments.get_mut(&id).expect("checked");
        t.profiles.insert(k.to_string(), profile);
        if paid {
            t.paid.insert(k.to_string());
        }
        if full {
            t.waitlist.push(k.to_string());
            self.notify(
                k,
                ServerMsg::Notice {
                    text: "The tournament is full: you are on the waiting list.".into(),
                },
            );
            self.broadcast_tournament(id);
            return Joined::Waiting;
        }
        t.players.push(k.to_string());
        if t.is_arena() && t.state == TournamentState::Running {
            t.arena.entry(k.to_string()).or_default();
        }
        let now_full = !t.is_arena() && t.players.len() >= t.size as usize;
        let official = t.host.is_empty();
        self.broadcast_tournament(id);
        if now_full && !official {
            self.begin_tournament(id);
        }
        Joined::In
    }

    /// Gives back an entry fee (the player left before the start, or the
    /// tournament was cancelled).
    fn refund(&mut self, id: u64, k: &str) {
        let Some(t) = self.tournaments.get_mut(&id) else { return };
        if !t.paid.remove(k) {
            return;
        }
        let Some((user, _)) = t.profiles.get(k) else { return };
        let pay = Effect::Pay {
            user_id: user.id,
            name: user.username.clone(),
            coins: t.entry_fee as i64,
            reason: "refund".into(),
            reference: format!("refund:tour:{id}:{}", user.id),
        };
        self.effect(pay);
    }

    /// Removes a tournament that never started, giving every fee back.
    pub(super) fn cancel_tournament(&mut self, id: u64) {
        let Some(t) = self.tournaments.get(&id) else { return };
        let everyone: Vec<String> = t.paid.iter().cloned().collect();
        for k in everyone {
            self.refund(id, &k);
        }
        self.tournaments.remove(&id);
        self.broadcast(&ServerMsg::TournamentRemoved { id });
    }

    /// A seat came free before the start: the first VIP member waiting
    /// gets it, else whoever waited longest.
    fn promote_waiting(&mut self, id: u64) {
        let Some(t) = self.tournaments.get(&id) else { return };
        if t.players.len() >= t.size as usize || t.waitlist.is_empty() {
            return;
        }
        let vip = |k: &String| self.sessions.get(k).is_some_and(|s| s.look.vip);
        let pick = t.waitlist.iter().position(vip).unwrap_or(0);
        let t = self.tournaments.get_mut(&id).expect("checked");
        let k = t.waitlist.remove(pick);
        t.players.push(k.clone());
        self.notify(
            &k,
            ServerMsg::Notice { text: "A seat opened: you are in the tournament!".into() },
        );
    }

    pub(super) fn leave_tournament(&mut self, k: &str, id: u64) {
        let Some(t) = self.tournaments.get_mut(&id) else { return };
        if let Some(i) = t.waitlist.iter().position(|p| p == k) {
            t.waitlist.remove(i);
            self.refund(id, k);
            self.broadcast_tournament(id);
            return;
        }
        if !t.players.iter().any(|p| p == k) {
            return;
        }
        match t.state {
            TournamentState::Registering => {
                if t.host == k {
                    self.cancel_tournament(id);
                    return;
                }
                t.players.retain(|p| p != k);
                self.refund(id, k);
                self.promote_waiting(id);
                self.broadcast_tournament(id);
            }
            TournamentState::Running if t.is_arena() => {
                t.withdrawn.insert(k.to_string());
                let in_game =
                    self.room_of(k).is_some_and(|r| r.tournament.is_some_and(|x| x.0 == id));
                if in_game {
                    self.resign(k);
                }
                self.broadcast_tournament(id);
            }
            TournamentState::Running => {
                // Withdrawing: lose the current match (or the next one, when
                // the opponent isn't known yet).
                if !t.alive(k) {
                    return;
                }
                t.withdrawn.insert(k.to_string());
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

    pub(super) fn start_tournament(&mut self, k: &str, id: u64) {
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
        if self.draining.is_some() {
            return;
        }
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
        // Whoever is still waiting for a seat gets the fee back.
        let waiting: Vec<String> =
            self.tournaments.get(&id).map(|t| t.waitlist.clone()).unwrap_or_default();
        for k in &waiting {
            self.refund(id, k);
        }
        let t = self.tournaments.get_mut(&id).expect("checked");
        t.waitlist.clear();
        let mut seeded = t.players.clone();
        seeded.sort_by(|a, b| ratings[b].cmp(&ratings[a]).then(a.cmp(b)));
        if t.is_arena() {
            let length = match (t.starts_at, t.ends_at) {
                (Some(s), Some(e)) => (e - s).max(600),
                _ => ARENA_LENGTH.as_secs() as i64,
            };
            t.ends_at = Some(now() + length);
            for p in &seeded {
                t.arena.entry(p.clone()).or_default();
            }
        } else {
            t.bracket = Some(Bracket::new(&seeded));
        }
        t.state = TournamentState::Running;
        // Open tables and invitations of the players are withdrawn.
        for p in seeded {
            self.close_table_of(&p);
            self.drop_invites_of(&p);
            self.announce(&p);
        }
        self.tournament_progress(id);
    }

    /// A tournament game ended: the bracket or the arena table moves on.
    pub(super) fn tournament_game_over(
        &mut self,
        tid: u64,
        round: usize,
        index: usize,
        keys: &[String; 2],
        winner: usize,
    ) {
        let Some(t) = self.tournaments.get_mut(&tid) else { return };
        if round == ARENA_ROUND {
            let (w, l) = (&keys[winner], &keys[1 - winner]);
            let ws = t.arena.entry(w.clone()).or_default();
            // From the third win in a row a win counts double.
            ws.points += if ws.streak >= 2 { 4 } else { 2 };
            ws.streak += 1;
            ws.games += 1;
            ws.last = Some(l.clone());
            let ls = t.arena.entry(l.clone()).or_default();
            ls.streak = 0;
            ls.games += 1;
            ls.last = Some(w.clone());
            self.broadcast_tournament(tid);
        } else if let Some(b) = t.bracket.as_mut() {
            let slot = b.rounds[round][index].players.clone();
            let w = &keys[winner];
            let seat = if slot[1].as_deref() == Some(w.as_str()) { 1 } else { 0 };
            b.set_winner(round, index, seat);
        }
        self.tournament_progress(tid);
    }

    /// Starts every game that can be played, gives walkovers to players
    /// whose opponent didn't show up, and crowns the champion.
    pub(super) fn tournament_progress(&mut self, id: u64) {
        let Some(t) = self.tournaments.get(&id) else { return };
        if t.state != TournamentState::Running {
            return;
        }
        if t.is_arena() {
            return self.arena_progress(id);
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
            // A player who withdrew loses at once.
            match [t.withdrawn.contains(&players[0]), t.withdrawn.contains(&players[1])] {
                [true, false] => {
                    walkovers.push((r, i, 1));
                    continue;
                }
                [false, true] | [true, true] => {
                    walkovers.push((r, i, 0));
                    continue;
                }
                [false, false] => {}
            }
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
        if self.draining.is_none() {
            for (r, i, players) in starts {
                let room_id = self.new_id();
                if let Some(b) = self.tournaments.get_mut(&id).and_then(|t| t.bracket.as_mut()) {
                    b.rounds[r][i].room = Some(room_id);
                }
                self.tournament_room(room_id, players, rated, (id, r, i));
            }
        }
        let t = self.tournaments.get(&id).expect("checked");
        let champion = t.bracket.as_ref().and_then(|b| b.champion()).cloned();
        if let Some(champion) = champion {
            // The runner-up lost the final.
            let runner_up = t.bracket.as_ref().and_then(|b| {
                let last = b.rounds.last()?.first()?;
                let w = last.winner?;
                last.players[1 - w].clone()
            });
            self.crown(id, champion, runner_up);
        }
        self.broadcast_tournament(id);
    }

    fn tournament_room(
        &mut self,
        room_id: u64,
        players: [String; 2],
        rated: bool,
        at: (u64, usize, usize),
    ) {
        let users =
            [self.sessions[&players[0]].user.clone(), self.sessions[&players[1]].user.clone()];
        let guests = [self.sessions[&players[0]].guest, self.sessions[&players[1]].guest];
        for p in &players {
            self.drop_invites_of(p);
            self.close_table_of(p);
            self.queue_leave(p, false);
        }
        let host = if users[0].rating >= users[1].rating {
            players[0].clone()
        } else {
            players[1].clone()
        };
        let look = self.table_look_of(&host);
        self.create_room(room_id, players, users, guests, rated, 0, Some(at), look);
    }

    /// Arena: pairs the players who are free (closest points, then rating;
    /// not the same opponent twice in a row) until the time is up, then
    /// crowns the best.
    fn arena_progress(&mut self, id: u64) {
        let Some(t) = self.tournaments.get(&id) else { return };
        let open = t.ends_at.is_some_and(|e| now() < e);
        if open && self.draining.is_none() {
            let nowi = Instant::now();
            let is_free = |p: &String| !t.withdrawn.contains(p) && self.available(p);
            let free_now: Vec<String> = t.players.iter().filter(|p| is_free(p)).cloned().collect();
            let t = self.tournaments.get_mut(&id).expect("checked");
            for (k, score) in t.arena.iter_mut() {
                if free_now.contains(k) {
                    score.idle_since.get_or_insert(nowi);
                } else {
                    score.idle_since = None;
                }
            }
            let t = &self.tournaments[&id];
            let waited = |k: &str| {
                t.arena
                    .get(k)
                    .and_then(|s| s.idle_since)
                    .is_some_and(|i| nowi - i >= ARENA_REMATCH_WAIT)
            };
            let mut free: Vec<(String, u32, i32, Option<String>)> = free_now
                .iter()
                .map(|p| {
                    let s = t.arena.get(p).cloned().unwrap_or_default();
                    (p.clone(), s.points, self.sessions[p].user.rating, s.last)
                })
                .collect();
            free.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)));
            let rated = t.rated;
            let mut pairs = Vec::new();
            while free.len() >= 2 {
                let a = free.remove(0);
                // The nearest who isn't the last opponent; the last one only
                // after both waited a while.
                let pick = free.iter().position(|b| {
                    a.3.as_deref() != Some(b.0.as_str()) || (waited(&a.0) && waited(&b.0))
                });
                let Some(pick) = pick else { continue };
                let b = free.remove(pick);
                pairs.push([a.0, b.0]);
            }
            for players in pairs {
                let room_id = self.new_id();
                self.tournament_room(room_id, players, rated, (id, ARENA_ROUND, 0));
            }
            return;
        }
        if open {
            return;
        }
        // Time is up: wait for the games still on, then crown the best.
        let playing =
            self.rooms.values().any(|r| !r.finished && r.tournament.is_some_and(|x| x.0 == id));
        if playing {
            return;
        }
        let t = &self.tournaments[&id];
        let rating = |k: &str| t.profiles.get(k).map(|(u, _)| u.rating).unwrap_or(0);
        let standings = t.standings(rating);
        let champion = standings.first().filter(|(_, s)| s.games > 0).map(|(k, _)| k.clone());
        let runner_up = standings.get(1).filter(|(_, s)| s.games > 0).map(|(k, _)| k.clone());
        match champion {
            Some(c) => self.crown(id, c, runner_up),
            None => {
                // Nobody played: no champion, fees back.
                let everyone: Vec<String> = t.paid.iter().cloned().collect();
                for k in everyone {
                    self.refund(id, &k);
                }
                let t = self.tournaments.get_mut(&id).expect("checked");
                t.state = TournamentState::Finished;
                t.finished_at = Some(Instant::now());
            }
        }
        self.broadcast_tournament(id);
    }

    /// The tournament is over: prizes, the record and the announcement.
    fn crown(&mut self, id: u64, champion: String, runner_up: Option<String>) {
        let Some(t) = self.tournaments.get_mut(&id) else { return };
        if t.state == TournamentState::Finished {
            return;
        }
        t.state = TournamentState::Finished;
        t.finished_at = Some(Instant::now());
        t.champion = Some(champion.clone());
        let prizes = t.prizes();
        let mut paid_out = Vec::new();
        for (k, coins) in [Some(champion.clone()), runner_up].into_iter().zip(prizes) {
            let Some(k) = k else { continue };
            let Some((user, guest)) = t.profiles.get(&k) else { continue };
            if coins == 0 || *guest {
                continue;
            }
            paid_out.push((user.id, user.username.clone(), coins));
        }
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
            entry_fee: t.entry_fee,
            prizes: paid_out
                .iter()
                .map(|(_, n, c)| format!("{n}:{c}"))
                .collect::<Vec<_>>()
                .join(", "),
        };
        let name = t.name.clone();
        let display =
            t.profiles.get(&champion).map(|(u, _)| u.username.clone()).unwrap_or(champion);
        for (user_id, username, coins) in paid_out {
            self.effect(Effect::Pay {
                user_id,
                name: username,
                coins: coins as i64,
                reason: "prize".into(),
                reference: format!("prize:tour:{id}:{user_id}"),
            });
        }
        if !record.players.is_empty() {
            self.effect(Effect::RecordTournament(record));
        }
        self.announce_text(&format!("{display} won the tournament \u{201c}{name}\u{201d}!"));
    }

    /// Called by the ticker: running tournaments move on, tournaments
    /// whose host left are dropped, the weekly cup opens and starts.
    pub(super) fn tournaments_tick(&mut self, nowi: Instant) {
        let running: Vec<u64> = self
            .tournaments
            .values()
            .filter(|t| t.state == TournamentState::Running)
            .map(|t| t.id)
            .collect();
        for id in running {
            self.tournament_progress(id);
        }
        let t_now = now();
        // The server's cup starts by itself.
        let due: Vec<u64> = self
            .tournaments
            .values()
            .filter(|t| t.state == TournamentState::Registering)
            .filter(|t| t.starts_at.is_some_and(|s| t_now >= s))
            .map(|t| t.id)
            .collect();
        for id in due {
            if self.tournaments[&id].players.len() >= 2 {
                self.begin_tournament(id);
            } else {
                self.cancel_tournament(id);
                self.announce_text("The cup was called off: not enough players signed up.");
            }
        }
        // Tournaments still taking names whose host went away and stayed away.
        let mut expired: Vec<u64> = Vec::new();
        for t in self.tournaments.values_mut() {
            if t.state != TournamentState::Registering || t.host.is_empty() {
                continue;
            }
            if self.sessions.contains_key(&t.host) {
                t.missing_since.remove(&t.host);
            } else {
                let since = *t.missing_since.entry(t.host.clone()).or_insert(nowi);
                if nowi.duration_since(since) > HOSTLESS_TOURNAMENT {
                    expired.push(t.id);
                }
            }
        }
        for id in expired {
            self.cancel_tournament(id);
        }
        let old: Vec<u64> = self
            .tournaments
            .values()
            .filter(|t| t.finished_at.is_some_and(|f| f.elapsed() > Duration::from_secs(600)))
            .map(|t| t.id)
            .collect();
        for id in old {
            self.tournaments.remove(&id);
            self.broadcast(&ServerMsg::TournamentRemoved { id });
        }
        self.schedule_cup(t_now);
    }

    /// Opens the weekly cup for sign-up when it is near.
    fn schedule_cup(&mut self, t: i64) {
        let Some(cup) = self.rules.cup.clone() else { return };
        if self.draining.is_some() || self.rules.lan {
            return;
        }
        let start = cup.next_start(t);
        if start - t > CUP_SIGN_UP {
            return;
        }
        if self.tournaments.values().any(|x| x.host.is_empty() && x.starts_at == Some(start)) {
            return;
        }
        let id = self.new_id();
        let end = start + cup.minutes as i64 * 60;
        self.tournaments.insert(
            id,
            Tournament {
                id,
                name: cup.name.clone(),
                host: String::new(),
                size: ARENA_MAX as u32,
                rated: true,
                password: None,
                state: TournamentState::Registering,
                players: Vec::new(),
                profiles: HashMap::new(),
                bracket: None,
                missing_since: HashMap::new(),
                finished_at: None,
                format: TournamentFormat::Arena,
                entry_fee: cup.entry_fee,
                paid: HashSet::new(),
                waitlist: Vec::new(),
                starts_at: Some(start),
                ends_at: Some(end),
                arena: HashMap::new(),
                withdrawn: HashSet::new(),
                champion: None,
            },
        );
        self.announce_text(&format!(
            "The {} is open for sign-up: it starts soon in Tournaments.",
            cup.name
        ));
        self.broadcast_tournament(id);
    }
}

#[cfg(test)]
mod tests {
    use super::Cup;

    #[test]
    fn cup_schedule() {
        let cup = Cup::parse("sat 20:00").unwrap();
        assert_eq!((cup.weekday, cup.hour, cup.minute, cup.utc_offset), (6, 20, 0, -180));
        // Thursday 2026-11-12 12:00 UTC → Saturday 2026-11-14 20:00 in Brasília (23:00 UTC).
        let t = 1_762_948_800 + 365 * 86_400; // 2026-11-12 12:00 UTC
        let start = cup.next_start(t);
        assert_eq!(start - t, 2 * 86_400 + 11 * 3600);
        // Right after it starts, the next one is a week later.
        assert_eq!(cup.next_start(start + 1), start + 7 * 86_400);
        assert_eq!(cup.next_start(start), start);
        let other = Cup::parse("wed 21:30 45m fee=50 utc+1").unwrap();
        assert_eq!(
            (other.weekday, other.minutes, other.entry_fee, other.utc_offset),
            (3, 45, 50, 60)
        );
        assert!(Cup::parse("whenever").is_none());
    }
}
