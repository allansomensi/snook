//! In-memory storage (tests and LAN games): nothing survives a restart.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;

use snook_proto::{LeaderboardEntry, Role};

use super::{
    AuditRow, FriendOutcome, FriendState, HistoryRow, MatchRecord, NewReport, NewUser, Overview,
    ReportRow, SessionKind, StoreResult, TournamentRecord, User,
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

    pub fn record_match(&self, m: &MatchRecord) -> StoreResult<[Option<User>; 2]> {
        let mut s = self.lock();
        for seat in 0..2 {
            if let Some(u) = s.users.get_mut(&m.players[seat]) {
                super::apply_match_numbers(u, m, seat);
            }
        }
        s.matches.push(m.clone());
        Ok([s.users.get(&m.players[0]).cloned(), s.users.get(&m.players[1]).cloned()])
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
            .rev()
            .filter_map(|m| {
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
                })
            })
            .take(limit as usize)
            .collect())
    }

    pub fn leaderboard(&self, limit: u32) -> StoreResult<Vec<LeaderboardEntry>> {
        let s = self.lock();
        let mut users: Vec<&User> =
            s.users.values().filter(|u| u.games > 0 && u.banned_until.is_none()).collect();
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
            })
            .collect())
    }

    pub fn rank_of(&self, user_id: i64) -> StoreResult<Option<u32>> {
        let s = self.lock();
        let Some(me) = s.users.get(&user_id).filter(|u| u.games > 0) else { return Ok(None) };
        let above = s
            .users
            .values()
            .filter(|u| u.games > 0 && u.banned_until.is_none() && u.rating > me.rating)
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
        out.sort_by(|x, y| x.0.username.to_lowercase().cmp(&y.0.username.to_lowercase()));
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
            .filter(|u| q.is_empty() || u.username.to_lowercase().contains(&q))
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

    pub fn set_ban(&self, user_id: i64, until: Option<i64>, reason: Option<&str>) -> StoreResult<()> {
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
}

