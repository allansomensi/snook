//! In-memory storage (tests and LAN games): nothing survives a restart.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;

use snookered_proto::{LeaderboardEntry, Role};

use snookered_proto::CareerNumbers;

use super::{
    AimRow, AuditRow, BuyOutcome, FriendOutcome, FriendState, HistoryRow, LedgerRow, LiveMatch,
    MatchRecord, Metrics, NewReport, NewUser, Overview, Payment, ReportRow, SessionKind,
    StoreResult, StoredReplay, TournamentRecord, User, Wallet, day_of,
};

#[derive(Default)]
pub struct MemoryStore {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    users: BTreeMap<i64, User>,
    next_user: i64,
    /// token hash -> (user, kind, created, expires)
    sessions: HashMap<String, (i64, SessionKind, i64, i64)>,
    matches: Vec<MatchRecord>,
    achievements: HashMap<i64, Vec<(String, i64)>>,
    /// (from, to) -> accepted
    friendships: HashMap<(i64, i64), bool>,
    blocks: HashSet<(i64, i64)>,
    reports: Vec<ReportRow>,
    report_ids: Vec<(i64, i64)>,
    audit: Vec<AuditRow>,
    tournaments: Vec<TournamentRecord>,
    wallets: HashMap<i64, Wallet>,
    /// (user, delta, reason, reference, at)
    ledger: Vec<(i64, i64, String, Option<String>, i64)>,
    payments: Vec<Payment>,
    live: BTreeMap<i64, LiveMatch>,
    /// (user, band) -> (attempts, made)
    shot_stats: HashMap<(i64, usize), (u32, u32)>,
    /// (day, user)
    activity: HashSet<(i64, i64)>,
    /// user -> (daily bonus day, earned day, earned that day)
    earnings: HashMap<i64, (i64, i64, i64)>,
}

impl Inner {
    fn by_name(&self, name: &str) -> Option<&User> {
        self.users.values().find(|u| u.username.eq_ignore_ascii_case(name))
    }

    fn name(&self, id: i64) -> String {
        self.users.get(&id).map(|u| u.username.clone()).unwrap_or_default()
    }
}

impl MemoryStore {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn create_user(&self, new: &NewUser<'_>) -> StoreResult<Option<User>> {
        let mut s = self.lock();
        if s.by_name(new.username).is_some() {
            return Ok(None);
        }
        s.next_user += 1;
        let now = crate::now();
        let user = User {
            id: s.next_user,
            username: new.username.to_string(),
            password_hash: new.password_hash.to_string(),
            country: new.country.to_string(),
            role: new.role,
            rating: new.rating,
            peak_rating: new.rating,
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
            created_at: now,
            last_seen: now,
            banned_until: None,
            ban_reason: None,
            muted_until: None,
            recovery_email: None,
            deleted_at: None,
            abandons: 0,
        };
        s.users.insert(user.id, user.clone());
        Ok(Some(user))
    }

    pub fn user_by_name(&self, name: &str) -> StoreResult<Option<User>> {
        Ok(self.lock().by_name(name).cloned())
    }

    pub fn user_by_id(&self, id: i64) -> StoreResult<Option<User>> {
        Ok(self.lock().users.get(&id).cloned())
    }

    pub fn create_session(
        &self,
        hash: &str,
        user_id: i64,
        kind: SessionKind,
        created: i64,
        expires: i64,
    ) -> StoreResult<()> {
        self.lock().sessions.insert(hash.to_string(), (user_id, kind, created, expires));
        Ok(())
    }

    pub fn session_user(
        &self,
        hash: &str,
        kind: SessionKind,
        now: i64,
    ) -> StoreResult<Option<(User, i64)>> {
        let s = self.lock();
        Ok(s.sessions
            .get(hash)
            .filter(|(_, k, _, exp)| *k == kind && *exp > now)
            .and_then(|(id, _, created, _)| s.users.get(id).map(|u| (u.clone(), *created))))
    }

    pub fn delete_session(&self, hash: &str) -> StoreResult<()> {
        self.lock().sessions.remove(hash);
        Ok(())
    }

    pub fn delete_user_sessions(&self, user_id: i64) -> StoreResult<()> {
        self.lock().sessions.retain(|_, (id, ..)| *id != user_id);
        Ok(())
    }

    pub fn purge_sessions(&self, now: i64) -> StoreResult<u64> {
        let mut s = self.lock();
        let before = s.sessions.len();
        s.sessions.retain(|_, (.., exp)| *exp > now);
        Ok((before - s.sessions.len()) as u64)
    }

    pub fn set_country(&self, user_id: i64, country: &str) -> StoreResult<()> {
        if let Some(u) = self.lock().users.get_mut(&user_id) {
            u.country = country.to_string();
        }
        Ok(())
    }

    pub fn touch(&self, user_id: i64, now: i64) -> StoreResult<()> {
        if let Some(u) = self.lock().users.get_mut(&user_id) {
            u.last_seen = now;
        }
        Ok(())
    }

    pub fn record_match(&self, m: &MatchRecord) -> StoreResult<([Option<User>; 2], i64)> {
        let mut s = self.lock();
        for seat in 0..2 {
            if let Some(u) = s.users.get_mut(&m.players[seat]) {
                super::apply_match_numbers(u, m, seat);
            }
        }
        s.matches.push(m.clone());
        let id = s.matches.len() as i64;
        Ok(([s.users.get(&m.players[0]).cloned(), s.users.get(&m.players[1]).cloned()], id))
    }

    pub fn career(&self, user_id: i64) -> StoreResult<CareerNumbers> {
        let s = self.lock();
        let mut c = CareerNumbers::default();
        for m in &s.matches {
            let Some(seat) = m.players.iter().position(|p| *p == user_id) else { continue };
            let st = &m.stats[seat];
            if m.replay.is_none() && st.shots == 0 {
                continue;
            }
            c.games += 1;
            c.shots += st.shots;
            c.potting_shots += st.potting_shots;
            c.balls += st.balls;
            c.fouls += st.fouls;
            c.best_run = c.best_run.max(st.best_run);
        }
        Ok(c)
    }

    pub fn rating_history(&self, user_id: i64, limit: u32) -> StoreResult<Vec<(i64, i32)>> {
        let s = self.lock();
        let mut list: Vec<(i64, i32)> = s
            .matches
            .iter()
            .rev()
            .filter(|m| m.rated)
            .filter_map(|m| {
                let seat = m.players.iter().position(|p| *p == user_id)?;
                Some((m.ended_at, m.ratings_after[seat]))
            })
            .take(limit as usize)
            .collect();
        list.reverse();
        Ok(list)
    }

    pub fn replay(&self, match_id: i64) -> StoreResult<Option<StoredReplay>> {
        let s = self.lock();
        let Some(m) = usize::try_from(match_id - 1).ok().and_then(|i| s.matches.get(i)) else {
            return Ok(None);
        };
        let Some(replay) = m.replay.clone() else { return Ok(None) };
        let user = |seat: usize| s.users.get(&m.players[seat]);
        Ok(Some(StoredReplay {
            id: match_id,
            players: [0, 1].map(|k| user(k).map(|u| u.username.clone()).unwrap_or_default()),
            countries: [0, 1].map(|k| user(k).map(|u| u.country.clone()).unwrap_or_default()),
            ratings: [0, 1].map(|k| m.ratings_after[k] - m.delta[k]),
            winner: m.winner,
            rated: m.rated,
            at: m.ended_at,
            replay,
        }))
    }

    pub fn save_live(&self, m: &LiveMatch) -> StoreResult<()> {
        self.lock().live.insert(m.id, m.clone());
        Ok(())
    }

    pub fn delete_live(&self, id: i64) -> StoreResult<()> {
        self.lock().live.remove(&id);
        Ok(())
    }

    pub fn live_match_of(&self, user_id: i64) -> StoreResult<Option<LiveMatch>> {
        Ok(self.lock().live.values().rev().find(|m| m.players.contains(&user_id)).cloned())
    }

    pub fn purge_live(&self, before: i64) -> StoreResult<u64> {
        let mut s = self.lock();
        let n = s.live.len();
        s.live.retain(|_, m| m.updated_at >= before);
        Ok((n - s.live.len()) as u64)
    }

    pub fn spend(
        &self,
        user_id: i64,
        amount: i64,
        reason: &str,
        reference: Option<&str>,
        at: i64,
    ) -> StoreResult<bool> {
        let mut s = self.lock();
        let w = s.wallets.entry(user_id).or_default();
        if w.coins < amount {
            return Ok(false);
        }
        w.coins -= amount;
        s.ledger.push((user_id, -amount, reason.to_string(), reference.map(str::to_string), at));
        Ok(true)
    }

    pub fn add_shot_stats(&self, user_id: i64, bands: &[(usize, u32, u32)]) -> StoreResult<()> {
        let mut s = self.lock();
        for &(band, attempts, made) in bands {
            let e = s.shot_stats.entry((user_id, band)).or_default();
            e.0 += attempts;
            e.1 += made;
        }
        Ok(())
    }

    pub fn aim_rows(&self, bands: &[usize], min: u32) -> StoreResult<Vec<AimRow>> {
        let s = self.lock();
        let mut rows: BTreeMap<i64, (u32, u32)> = BTreeMap::new();
        for (&(user, band), &(a, m)) in &s.shot_stats {
            if bands.contains(&band) {
                let e = rows.entry(user).or_default();
                e.0 += a;
                e.1 += m;
            }
        }
        Ok(rows
            .into_iter()
            .filter(|(_, (a, _))| *a >= min)
            .filter_map(|(id, (attempts, made))| {
                let u = s.users.get(&id).filter(|u| u.deleted_at.is_none())?;
                Some(AimRow {
                    user_id: id,
                    username: u.username.clone(),
                    rating: u.rating,
                    attempts,
                    made,
                })
            })
            .collect())
    }

    pub fn mark_active(&self, user_id: i64, day: i64) -> StoreResult<()> {
        self.lock().activity.insert((day, user_id));
        Ok(())
    }

    pub fn metrics(&self, now: i64) -> StoreResult<Metrics> {
        let s = self.lock();
        let today = day_of(now);
        let days = (today - 29..=today)
            .map(|d| {
                let active = s.activity.iter().filter(|(day, _)| *day == d).count() as i64;
                let matches = s.matches.iter().filter(|m| day_of(m.ended_at) == d).count() as i64;
                let new = s.users.values().filter(|u| day_of(u.created_at) == d).count() as i64;
                (d, active, matches, new)
            })
            .collect();
        let recent: Vec<&User> =
            s.users.values().filter(|u| day_of(u.created_at) >= today - 30).collect();
        let back = |after: i64| {
            let eligible: Vec<&&User> =
                recent.iter().filter(|u| day_of(u.created_at) + after <= today).collect();
            let came = eligible
                .iter()
                .filter(|u| s.activity.contains(&(day_of(u.created_at) + after, u.id)))
                .count();
            if eligible.is_empty() { 0.0 } else { came as f64 * 100.0 / eligible.len() as f64 }
        };
        let games = |u: &User| u.games + u.casual_games;
        let mut revenue: BTreeMap<String, (i64, i64)> = BTreeMap::new();
        for p in s.payments.iter().filter(|p| !p.refunded && p.at > now - 30 * 86_400) {
            let e = revenue.entry(p.currency.clone()).or_default();
            e.0 += p.amount;
            e.1 += 1;
        }
        Ok(Metrics {
            days,
            retention_d1: back(1),
            retention_d7: back(7),
            funnel: [
                recent.len() as i64,
                recent.iter().filter(|u| games(u) >= 1).count() as i64,
                recent.iter().filter(|u| games(u) >= 5).count() as i64,
            ],
            revenue: revenue.into_iter().map(|(c, (a, n))| (c, a, n)).collect(),
        })
    }

    pub fn set_recovery_email(&self, user_id: i64, email: Option<&str>) -> StoreResult<()> {
        if let Some(u) = self.lock().users.get_mut(&user_id) {
            u.recovery_email = email.map(str::to_string);
        }
        Ok(())
    }

    pub fn payment_email(&self, user_id: i64) -> StoreResult<Option<String>> {
        let s = self.lock();
        Ok(s.payments.iter().rev().filter(|p| p.user_id == user_id).find_map(|p| p.email.clone()))
    }

    pub fn delete_account(&self, user_id: i64, now: i64) -> StoreResult<()> {
        let mut s = self.lock();
        if let Some(u) = s.users.get_mut(&user_id) {
            u.username = super::deleted_name(user_id);
            u.password_hash = String::new();
            u.country = String::new();
            u.recovery_email = None;
            u.deleted_at = Some(now);
        }
        s.sessions.retain(|_, v| v.0 != user_id);
        s.friendships.retain(|(a, b), _| *a != user_id && *b != user_id);
        s.blocks.retain(|(a, b)| *a != user_id && *b != user_id);
        s.achievements.remove(&user_id);
        s.shot_stats.retain(|(u, _), _| *u != user_id);
        s.live.retain(|_, m| !m.players.contains(&user_id));
        if let Some(w) = s.wallets.get_mut(&user_id) {
            w.stripe_customer = None;
            w.equipped.clear();
        }
        for p in s.payments.iter_mut().filter(|p| p.user_id == user_id) {
            p.email = None;
        }
        Ok(())
    }

    pub fn add_achievements(
        &self,
        user_id: i64,
        codes: &[&str],
        at: i64,
    ) -> StoreResult<Vec<String>> {
        let mut s = self.lock();
        let list = s.achievements.entry(user_id).or_default();
        let mut new = Vec::new();
        for code in codes {
            if !list.iter().any(|(c, _)| c == code) {
                list.push((code.to_string(), at));
                new.push(code.to_string());
            }
        }
        Ok(new)
    }

    pub fn achievements(&self, user_id: i64) -> StoreResult<Vec<(String, i64)>> {
        Ok(self.lock().achievements.get(&user_id).cloned().unwrap_or_default())
    }

    pub fn history(&self, user_id: i64, limit: u32) -> StoreResult<Vec<HistoryRow>> {
        let s = self.lock();
        Ok(s.matches
            .iter()
            .enumerate()
            .rev()
            .filter_map(|(index, m)| {
                let seat = m.players.iter().position(|p| *p == user_id)?;
                let other = s.users.get(&m.players[1 - seat]);
                Some(HistoryRow {
                    opponent: other.map(|u| u.username.clone()).unwrap_or_default(),
                    opponent_country: other.map(|u| u.country.clone()).unwrap_or_default(),
                    opponent_rating: m.ratings_after[1 - seat],
                    won: m.winner == seat,
                    reason: crate::reason_code(m.reason).to_string(),
                    rated: m.rated,
                    delta: m.delta[seat],
                    rating_after: m.ratings_after[seat],
                    tournament: m.tournament.is_some(),
                    at: m.ended_at,
                    match_id: index as i64 + 1,
                    replay: m.replay.is_some(),
                    opponent_role: other.map(|u| u.role).unwrap_or_default(),
                })
            })
            .take(limit as usize)
            .collect())
    }

    pub fn leaderboard(&self, limit: u32, now: i64) -> StoreResult<Vec<LeaderboardEntry>> {
        let s = self.lock();
        // Banned players are left out while the ban lasts.
        let mut users: Vec<&User> = s
            .users
            .values()
            .filter(|u| u.games > 0 && !u.is_banned(now) && u.deleted_at.is_none())
            .collect();
        users.sort_by(|a, b| {
            b.rating.cmp(&a.rating).then(b.wins.cmp(&a.wins)).then(a.username.cmp(&b.username))
        });
        Ok(users
            .into_iter()
            .take(limit as usize)
            .enumerate()
            .map(|(i, u)| LeaderboardEntry {
                rank: i as u32 + 1,
                username: u.username.clone(),
                rating: u.rating,
                games: u.games,
                wins: u.wins,
                country: u.country.clone(),
                vip: s.wallets.get(&u.id).is_some_and(|w| w.vip_until > now),
                role: u.role,
            })
            .collect())
    }

    pub fn rank_of(&self, user_id: i64) -> StoreResult<Option<u32>> {
        let s = self.lock();
        let Some(me) = s.users.get(&user_id).filter(|u| u.games > 0) else { return Ok(None) };
        let now = crate::now();
        let above = s
            .users
            .values()
            .filter(|u| {
                u.games > 0 && !u.is_banned(now) && u.deleted_at.is_none() && u.rating > me.rating
            })
            .count();
        Ok(Some(above as u32 + 1))
    }

    pub fn friends(&self, user_id: i64) -> StoreResult<Vec<(User, FriendState)>> {
        let s = self.lock();
        let mut out = Vec::new();
        for ((a, b), accepted) in &s.friendships {
            let (other, state) = if *a == user_id {
                (*b, if *accepted { FriendState::Friends } else { FriendState::Outgoing })
            } else if *b == user_id && !*accepted {
                (*a, FriendState::Incoming)
            } else {
                continue;
            };
            if let Some(u) = s.users.get(&other) {
                out.push((u.clone(), state));
            }
        }
        out.sort_by_key(|x| x.0.username.to_lowercase());
        Ok(out)
    }

    pub fn friend_request(&self, from: i64, to: i64, _at: i64) -> StoreResult<FriendOutcome> {
        let mut s = self.lock();
        if s.friendships.get(&(from, to)) == Some(&true) {
            return Ok(FriendOutcome::AlreadyFriends);
        }
        if s.friendships.get(&(to, from)) == Some(&false) {
            s.friendships.insert((to, from), true);
            s.friendships.insert((from, to), true);
            return Ok(FriendOutcome::Accepted);
        }
        if s.friendships.contains_key(&(from, to)) {
            return Ok(FriendOutcome::AlreadySent);
        }
        s.friendships.insert((from, to), false);
        Ok(FriendOutcome::Sent)
    }

    pub fn friend_accept(&self, me: i64, from: i64) -> StoreResult<bool> {
        let mut s = self.lock();
        if s.friendships.get(&(from, me)) != Some(&false) {
            return Ok(false);
        }
        s.friendships.insert((from, me), true);
        s.friendships.insert((me, from), true);
        Ok(true)
    }

    pub fn friend_remove(&self, a: i64, b: i64) -> StoreResult<()> {
        let mut s = self.lock();
        s.friendships.remove(&(a, b));
        s.friendships.remove(&(b, a));
        Ok(())
    }

    pub fn friend_count(&self, user_id: i64) -> StoreResult<u32> {
        let s = self.lock();
        Ok(s.friendships.iter().filter(|((a, _), ok)| *a == user_id && **ok).count() as u32)
    }

    pub fn block(&self, user_id: i64, blocked: i64) -> StoreResult<()> {
        self.lock().blocks.insert((user_id, blocked));
        Ok(())
    }

    pub fn unblock(&self, user_id: i64, blocked: i64) -> StoreResult<()> {
        self.lock().blocks.remove(&(user_id, blocked));
        Ok(())
    }

    pub fn blocked(&self, user_id: i64) -> StoreResult<Vec<User>> {
        let s = self.lock();
        Ok(s.blocks
            .iter()
            .filter(|(a, _)| *a == user_id)
            .filter_map(|(_, b)| s.users.get(b).cloned())
            .collect())
    }

    pub fn create_report(&self, r: &NewReport) -> StoreResult<()> {
        let mut s = self.lock();
        let id = s.reports.len() as i64 + 1;
        let row = ReportRow {
            id,
            reporter: s.name(r.reporter),
            target: s.name(r.target),
            reason: r.reason.clone(),
            details: r.details.clone(),
            context: r.context.clone(),
            status: 0,
            created_at: r.at,
            resolved_by: None,
            resolution: None,
        };
        s.reports.push(row);
        s.report_ids.push((r.reporter, r.target));
        Ok(())
    }

    pub fn record_tournament(&self, t: &TournamentRecord) -> StoreResult<()> {
        let mut s = self.lock();
        for id in &t.players {
            if let Some(u) = s.users.get_mut(id) {
                u.tournaments_played += 1;
                if t.winner == Some(*id) {
                    u.tournaments_won += 1;
                }
            }
        }
        s.tournaments.push(t.clone());
        Ok(())
    }

    pub fn search_users(
        &self,
        query: &str,
        offset: u32,
        limit: u32,
    ) -> StoreResult<(Vec<User>, i64)> {
        let s = self.lock();
        let q = query.to_lowercase();
        let all: Vec<User> = s
            .users
            .values()
            .rev()
            .filter(|u| {
                q.is_empty()
                    || u.username.to_lowercase().contains(&q)
                    || u.recovery_email.as_deref().is_some_and(|e| e.to_lowercase() == q)
            })
            .cloned()
            .collect();
        let total = all.len() as i64;
        Ok((all.into_iter().skip(offset as usize).take(limit as usize).collect(), total))
    }

    pub fn overview(&self, now: i64) -> StoreResult<Overview> {
        let s = self.lock();
        let week = now - 7 * 86_400;
        Ok(Overview {
            users: s.users.len() as i64,
            new_users_7d: s.users.values().filter(|u| u.created_at > week).count() as i64,
            active_users_7d: s.users.values().filter(|u| u.last_seen > week).count() as i64,
            matches_total: s.matches.len() as i64,
            matches_24h: s.matches.iter().filter(|m| m.ended_at > now - 86_400).count() as i64,
            open_reports: s.reports.iter().filter(|r| r.status == 0).count() as i64,
            banned: s.users.values().filter(|u| u.is_banned(now)).count() as i64,
        })
    }

    pub fn set_ban(
        &self,
        user_id: i64,
        until: Option<i64>,
        reason: Option<&str>,
    ) -> StoreResult<()> {
        if let Some(u) = self.lock().users.get_mut(&user_id) {
            u.banned_until = until;
            u.ban_reason = reason.map(str::to_string);
        }
        Ok(())
    }

    pub fn set_mute(&self, user_id: i64, until: Option<i64>) -> StoreResult<()> {
        if let Some(u) = self.lock().users.get_mut(&user_id) {
            u.muted_until = until;
        }
        Ok(())
    }

    pub fn set_role(&self, user_id: i64, role: Role) -> StoreResult<()> {
        if let Some(u) = self.lock().users.get_mut(&user_id) {
            u.role = role;
        }
        Ok(())
    }

    pub fn set_password(&self, user_id: i64, hash: &str) -> StoreResult<()> {
        if let Some(u) = self.lock().users.get_mut(&user_id) {
            u.password_hash = hash.to_string();
        }
        Ok(())
    }

    pub fn reports(&self, status: Option<i16>, limit: u32) -> StoreResult<Vec<ReportRow>> {
        let s = self.lock();
        Ok(s.reports
            .iter()
            .rev()
            .filter(|r| status.is_none_or(|st| r.status == st))
            .take(limit as usize)
            .cloned()
            .collect())
    }

    pub fn reports_about(&self, user_id: i64, limit: u32) -> StoreResult<Vec<ReportRow>> {
        let s = self.lock();
        Ok(s.reports
            .iter()
            .zip(&s.report_ids)
            .rev()
            .filter(|(_, (_, t))| *t == user_id)
            .map(|(r, _)| r.clone())
            .take(limit as usize)
            .collect())
    }

    pub fn resolve_report(
        &self,
        id: i64,
        staff: i64,
        status: i16,
        note: &str,
        _at: i64,
    ) -> StoreResult<bool> {
        let mut s = self.lock();
        let name = s.name(staff);
        match s.reports.iter_mut().find(|r| r.id == id) {
            Some(r) => {
                r.status = status;
                r.resolved_by = Some(name);
                r.resolution = Some(note.to_string());
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub fn audit(
        &self,
        staff: i64,
        action: &str,
        target: &str,
        details: &str,
        at: i64,
    ) -> StoreResult<()> {
        let mut s = self.lock();
        let id = s.audit.len() as i64 + 1;
        let row = AuditRow {
            id,
            staff: s.name(staff),
            action: action.to_string(),
            target: target.to_string(),
            details: details.to_string(),
            at,
        };
        s.audit.push(row);
        Ok(())
    }

    pub fn audit_log(&self, limit: u32) -> StoreResult<Vec<AuditRow>> {
        Ok(self.lock().audit.iter().rev().take(limit as usize).cloned().collect())
    }

    // The shop.

    pub fn wallet(&self, user_id: i64) -> StoreResult<Wallet> {
        Ok(self.lock().wallets.get(&user_id).cloned().unwrap_or_default())
    }

    pub fn buy_item(
        &self,
        user_id: i64,
        code: &str,
        price: i64,
        at: i64,
    ) -> StoreResult<BuyOutcome> {
        let mut s = self.lock();
        let w = s.wallets.entry(user_id).or_default();
        if w.owned.iter().any(|o| o == code) {
            return Ok(BuyOutcome::AlreadyOwned);
        }
        if w.coins < price {
            return Ok(BuyOutcome::NotEnoughCoins);
        }
        w.coins -= price;
        w.owned.push(code.to_string());
        s.ledger.push((user_id, -price, format!("buy {code}"), None, at));
        Ok(BuyOutcome::Bought)
    }

    pub fn grant_item(
        &self,
        user_id: i64,
        code: &str,
        _source: &str,
        _at: i64,
    ) -> StoreResult<bool> {
        let mut s = self.lock();
        let w = s.wallets.entry(user_id).or_default();
        if w.owned.iter().any(|o| o == code) {
            return Ok(false);
        }
        w.owned.push(code.to_string());
        Ok(true)
    }

    pub fn equip(&self, user_id: i64, category: &str, code: &str) -> StoreResult<()> {
        let mut s = self.lock();
        s.wallets
            .entry(user_id)
            .or_default()
            .equipped
            .insert(category.to_string(), code.to_string());
        Ok(())
    }

    pub fn credit(
        &self,
        user_id: i64,
        delta: i64,
        reason: &str,
        reference: Option<&str>,
        at: i64,
    ) -> StoreResult<bool> {
        let mut s = self.lock();
        if let Some(r) = reference
            && s.ledger.iter().any(|l| l.3.as_deref() == Some(r))
        {
            return Ok(false);
        }
        s.wallets.entry(user_id).or_default().coins += delta;
        s.ledger.push((user_id, delta, reason.to_string(), reference.map(str::to_string), at));
        Ok(true)
    }

    pub fn set_vip(&self, user_id: i64, until: i64, force: bool) -> StoreResult<()> {
        let mut s = self.lock();
        let w = s.wallets.entry(user_id).or_default();
        w.vip_until = if force { until } else { w.vip_until.max(until) };
        Ok(())
    }

    pub fn set_stripe_customer(&self, user_id: i64, customer: &str) -> StoreResult<()> {
        let mut s = self.lock();
        s.wallets.entry(user_id).or_default().stripe_customer = Some(customer.to_string());
        Ok(())
    }

    pub fn user_by_stripe_customer(&self, customer: &str) -> StoreResult<Option<i64>> {
        let s = self.lock();
        Ok(s.wallets
            .iter()
            .find(|(_, w)| w.stripe_customer.as_deref() == Some(customer))
            .map(|(id, _)| *id))
    }

    pub fn record_payment(&self, p: &Payment) -> StoreResult<bool> {
        let mut s = self.lock();
        if s.payments.iter().any(|x| x.id == p.id) {
            return Ok(false);
        }
        s.payments.push(p.clone());
        Ok(true)
    }

    pub fn payment(&self, id: &str) -> StoreResult<Option<Payment>> {
        Ok(self.lock().payments.iter().find(|p| p.id == id).cloned())
    }

    pub fn refund_payment(&self, id: &str) -> StoreResult<bool> {
        let mut s = self.lock();
        match s.payments.iter_mut().find(|p| p.id == id) {
            Some(p) if !p.refunded => {
                p.refunded = true;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    pub fn ledger(&self, user_id: i64, limit: u32) -> StoreResult<Vec<LedgerRow>> {
        let s = self.lock();
        Ok(s.ledger
            .iter()
            .rev()
            .filter(|l| l.0 == user_id)
            .take(limit as usize)
            .map(|l| LedgerRow { delta: l.1, reason: l.2.clone(), reference: l.3.clone(), at: l.4 })
            .collect())
    }

    pub fn payments_of(&self, user_id: i64, limit: u32) -> StoreResult<Vec<Payment>> {
        let s = self.lock();
        Ok(s.payments
            .iter()
            .rev()
            .filter(|p| p.user_id == user_id)
            .take(limit as usize)
            .cloned()
            .collect())
    }

    pub fn claim_daily(&self, user_id: i64, day: i64, coins: i64) -> StoreResult<bool> {
        let mut s = self.lock();
        let e = s.earnings.entry(user_id).or_default();
        if e.0 >= day {
            return Ok(false);
        }
        e.0 = day;
        s.wallets.entry(user_id).or_default().coins += coins;
        s.ledger.push((user_id, coins, "daily bonus".into(), None, day * 86_400));
        Ok(true)
    }

    pub fn earn(
        &self,
        user_id: i64,
        day: i64,
        amount: i64,
        cap: i64,
        reason: &str,
        at: i64,
    ) -> StoreResult<i64> {
        let mut s = self.lock();
        let e = s.earnings.entry(user_id).or_default();
        if e.1 != day {
            e.1 = day;
            e.2 = 0;
        }
        let given = amount.min(cap - e.2).max(0);
        if given == 0 {
            return Ok(0);
        }
        e.2 += given;
        s.wallets.entry(user_id).or_default().coins += given;
        s.ledger.push((user_id, given, reason.to_string(), None, at));
        Ok(given)
    }

    pub fn gift_item(
        &self,
        from: i64,
        to: i64,
        code: &str,
        price: i64,
        at: i64,
    ) -> StoreResult<BuyOutcome> {
        let mut s = self.lock();
        if s.wallets.get(&to).is_some_and(|w| w.owned.iter().any(|o| o == code)) {
            return Ok(BuyOutcome::AlreadyOwned);
        }
        let payer = s.wallets.entry(from).or_default();
        if payer.coins < price {
            return Ok(BuyOutcome::NotEnoughCoins);
        }
        payer.coins -= price;
        s.wallets.entry(to).or_default().owned.push(code.to_string());
        s.ledger.push((from, -price, format!("gift {code} to {to}"), None, at));
        Ok(BuyOutcome::Bought)
    }
}
