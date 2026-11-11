//! Persistent storage: accounts, sessions, match history, achievements,
//! friends, reports and the staff audit log.
//!
//! Two back ends with the same behaviour: PostgreSQL for production
//! (feature `postgres`) and an in-memory store for tests and LAN games.

mod memory;
#[cfg(feature = "postgres")]
mod postgres;

pub use memory::MemoryStore;
#[cfg(feature = "postgres")]
pub use postgres::PgStore;

use std::collections::BTreeMap;

use snook_proto::{LeaderboardEntry, Role, Stats};

#[derive(Debug, Clone)]
pub struct StoreError(pub String);

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StoreError {}

pub type StoreResult<T> = Result<T, StoreError>;

/// An account.
#[derive(Debug, Clone, PartialEq)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub country: String,
    pub role: Role,
    pub rating: i32,
    pub peak_rating: i32,
    pub games: u32,
    pub wins: u32,
    pub eight_wins: u32,
    pub casual_games: u32,
    pub streak: u32,
    pub best_streak: u32,
    pub runouts: u32,
    pub shutouts: u32,
    pub tournaments_played: u32,
    pub tournaments_won: u32,
    /// Unix seconds.
    pub created_at: i64,
    pub last_seen: i64,
    /// Banned until this time (Unix seconds); `i64::MAX` is permanent.
    pub banned_until: Option<i64>,
    pub ban_reason: Option<String>,
    pub muted_until: Option<i64>,
}

impl User {
    pub fn rating(&self) -> snook_core::rating::Rating {
        snook_core::rating::Rating { rating: self.rating, games: self.games }
    }

    pub fn is_banned(&self, now: i64) -> bool {
        self.banned_until.is_some_and(|t| t > now)
    }

    pub fn is_muted(&self, now: i64) -> bool {
        self.muted_until.is_some_and(|t| t > now)
    }

    /// Lifetime numbers (`friends` must be filled in by the caller).
    pub fn stats(&self, now: i64) -> Stats {
        Stats {
            games: self.games,
            wins: self.wins,
            eight_wins: self.eight_wins,
            casual_games: self.casual_games,
            peak_rating: self.peak_rating,
            streak: self.streak,
            best_streak: self.best_streak,
            runouts: self.runouts,
            shutouts: self.shutouts,
            friends: 0,
            tournaments_played: self.tournaments_played,
            tournaments_won: self.tournaments_won,
            account_days: ((now - self.created_at).max(0) / 86_400) as u32,
            today: 0,
        }
    }
}

pub struct NewUser<'a> {
    pub username: &'a str,
    pub password_hash: &'a str,
    pub country: &'a str,
    pub rating: i32,
    pub role: Role,
    pub ip: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    Game = 0,
    Admin = 1,
}

/// A finished match, as stored.
#[derive(Debug, Clone)]
pub struct MatchRecord {
    pub players: [i64; 2],
    pub winner: usize,
    pub reason: snook_core::GameOverReason,
    pub rated: bool,
    pub delta: [i32; 2],
    pub ratings_after: [i32; 2],
    pub shots: u32,
    pub tournament: Option<i64>,
    /// The winner legally pocketed the 8.
    pub eight_win: bool,
    /// The winner ran the table from the break.
    pub runout: bool,
    /// The loser never pocketed a ball of their group.
    pub shutout: bool,
    pub ended_at: i64,
}

#[derive(Debug, Clone)]
pub struct HistoryRow {
    pub opponent: String,
    pub opponent_country: String,
    pub opponent_rating: i32,
    pub won: bool,
    pub reason: String,
    pub rated: bool,
    pub delta: i32,
    pub rating_after: i32,
    pub tournament: bool,
    pub at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FriendState {
    Friends,
    /// The user asked the other.
    Outgoing,
    /// The other asked the user.
    Incoming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FriendOutcome {
    Sent,
    /// The other had already asked: now they are friends.
    Accepted,
    AlreadyFriends,
    AlreadySent,
}

#[derive(Debug, Clone)]
pub struct NewReport {
    pub reporter: i64,
    pub target: i64,
    pub reason: String,
    pub details: String,
    pub context: String,
    pub at: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ReportRow {
    pub id: i64,
    pub reporter: String,
    pub target: String,
    pub reason: String,
    pub details: String,
    pub context: String,
    /// 0 open, 1 action taken, 2 dismissed.
    pub status: i16,
    pub created_at: i64,
    pub resolved_by: Option<String>,
    pub resolution: Option<String>,
}

/// A player's coins, VIP time, cosmetics and payment customer.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct Wallet {
    pub coins: i64,
    /// VIP until this time (Unix seconds; 0 = never).
    pub vip_until: i64,
    pub owned: Vec<String>,
    /// Category code -> equipped item code.
    pub equipped: BTreeMap<String, String>,
    pub stripe_customer: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BuyOutcome {
    Bought,
    AlreadyOwned,
    NotEnoughCoins,
}

/// A payment received (coins or a VIP month).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Payment {
    /// The provider's id (payment intent or invoice).
    pub id: String,
    pub user_id: i64,
    /// "coins" or "vip".
    pub kind: String,
    pub coins: i64,
    /// In the currency's smallest unit (cents).
    pub amount: i64,
    pub currency: String,
    pub at: i64,
    pub refunded: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct LedgerRow {
    pub delta: i64,
    pub reason: String,
    pub reference: Option<String>,
    pub at: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AuditRow {
    pub id: i64,
    pub staff: String,
    pub action: String,
    pub target: String,
    pub details: String,
    pub at: i64,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Overview {
    pub users: i64,
    pub new_users_7d: i64,
    pub active_users_7d: i64,
    pub matches_total: i64,
    pub matches_24h: i64,
    pub open_reports: i64,
    pub banned: i64,
}

#[derive(Debug, Clone)]
pub struct TournamentRecord {
    pub name: String,
    pub host: i64,
    pub size: u32,
    pub rated: bool,
    pub players: Vec<i64>,
    pub winner: Option<i64>,
    pub finished_at: i64,
}

/// The storage back end (one per server, so the size difference doesn't matter).
#[allow(clippy::large_enum_variant)]
pub enum Store {
    Memory(MemoryStore),
    #[cfg(feature = "postgres")]
    Postgres(PgStore),
}

macro_rules! dispatch {
    ($self:ident, $s:ident => $e:expr) => {
        match $self {
            Store::Memory($s) => $e,
            #[cfg(feature = "postgres")]
            Store::Postgres($s) => $e.await,
        }
    };
}

impl Store {
    pub fn memory() -> Store {
        Store::Memory(MemoryStore::default())
    }

    #[cfg(feature = "postgres")]
    pub async fn postgres(url: &str, max_connections: u32) -> StoreResult<Store> {
        Ok(Store::Postgres(PgStore::connect(url, max_connections).await?))
    }

    pub fn is_persistent(&self) -> bool {
        !matches!(self, Store::Memory(_))
    }

    pub async fn create_user(&self, new: &NewUser<'_>) -> StoreResult<Option<User>> {
        dispatch!(self, s => s.create_user(new))
    }
    pub async fn user_by_name(&self, name: &str) -> StoreResult<Option<User>> {
        dispatch!(self, s => s.user_by_name(name))
    }
    pub async fn user_by_id(&self, id: i64) -> StoreResult<Option<User>> {
        dispatch!(self, s => s.user_by_id(id))
    }
    pub async fn create_session(
        &self,
        token_hash: &str,
        user_id: i64,
        kind: SessionKind,
        created_at: i64,
        expires_at: i64,
    ) -> StoreResult<()> {
        dispatch!(self, s => s.create_session(token_hash, user_id, kind, created_at, expires_at))
    }
    /// The user of a valid session, with the session's creation time.
    pub async fn session_user(
        &self,
        token_hash: &str,
        kind: SessionKind,
        now: i64,
    ) -> StoreResult<Option<(User, i64)>> {
        dispatch!(self, s => s.session_user(token_hash, kind, now))
    }
    pub async fn delete_session(&self, token_hash: &str) -> StoreResult<()> {
        dispatch!(self, s => s.delete_session(token_hash))
    }
    pub async fn delete_user_sessions(&self, user_id: i64) -> StoreResult<()> {
        dispatch!(self, s => s.delete_user_sessions(user_id))
    }
    pub async fn purge_sessions(&self, now: i64) -> StoreResult<u64> {
        dispatch!(self, s => s.purge_sessions(now))
    }
    pub async fn set_country(&self, user_id: i64, country: &str) -> StoreResult<()> {
        dispatch!(self, s => s.set_country(user_id, country))
    }
    pub async fn touch(&self, user_id: i64, now: i64) -> StoreResult<()> {
        dispatch!(self, s => s.touch(user_id, now))
    }
    /// Stores a match and updates both players' numbers; returns them updated.
    pub async fn record_match(&self, m: &MatchRecord) -> StoreResult<[Option<User>; 2]> {
        dispatch!(self, s => s.record_match(m))
    }
    /// Unlocks achievements; returns the codes that were new.
    pub async fn add_achievements(
        &self,
        user_id: i64,
        codes: &[&str],
        at: i64,
    ) -> StoreResult<Vec<String>> {
        dispatch!(self, s => s.add_achievements(user_id, codes, at))
    }
    pub async fn achievements(&self, user_id: i64) -> StoreResult<Vec<(String, i64)>> {
        dispatch!(self, s => s.achievements(user_id))
    }
    pub async fn history(&self, user_id: i64, limit: u32) -> StoreResult<Vec<HistoryRow>> {
        dispatch!(self, s => s.history(user_id, limit))
    }
    pub async fn leaderboard(&self, limit: u32) -> StoreResult<Vec<LeaderboardEntry>> {
        dispatch!(self, s => s.leaderboard(limit))
    }
    pub async fn rank_of(&self, user_id: i64) -> StoreResult<Option<u32>> {
        dispatch!(self, s => s.rank_of(user_id))
    }
    pub async fn friends(&self, user_id: i64) -> StoreResult<Vec<(User, FriendState)>> {
        dispatch!(self, s => s.friends(user_id))
    }
    pub async fn friend_request(&self, from: i64, to: i64, at: i64) -> StoreResult<FriendOutcome> {
        dispatch!(self, s => s.friend_request(from, to, at))
    }
    /// `me` accepts `from`'s request; false if there was none.
    pub async fn friend_accept(&self, me: i64, from: i64) -> StoreResult<bool> {
        dispatch!(self, s => s.friend_accept(me, from))
    }
    /// Removes a friendship or a request in either direction.
    pub async fn friend_remove(&self, a: i64, b: i64) -> StoreResult<()> {
        dispatch!(self, s => s.friend_remove(a, b))
    }
    pub async fn friend_count(&self, user_id: i64) -> StoreResult<u32> {
        dispatch!(self, s => s.friend_count(user_id))
    }
    pub async fn block(&self, user_id: i64, blocked: i64) -> StoreResult<()> {
        dispatch!(self, s => s.block(user_id, blocked))
    }
    pub async fn unblock(&self, user_id: i64, blocked: i64) -> StoreResult<()> {
        dispatch!(self, s => s.unblock(user_id, blocked))
    }
    pub async fn blocked(&self, user_id: i64) -> StoreResult<Vec<User>> {
        dispatch!(self, s => s.blocked(user_id))
    }
    pub async fn create_report(&self, r: &NewReport) -> StoreResult<()> {
        dispatch!(self, s => s.create_report(r))
    }
    pub async fn record_tournament(&self, t: &TournamentRecord) -> StoreResult<()> {
        dispatch!(self, s => s.record_tournament(t))
    }

    // Staff.

    pub async fn search_users(
        &self,
        query: &str,
        offset: u32,
        limit: u32,
    ) -> StoreResult<(Vec<User>, i64)> {
        dispatch!(self, s => s.search_users(query, offset, limit))
    }
    pub async fn overview(&self, now: i64) -> StoreResult<Overview> {
        dispatch!(self, s => s.overview(now))
    }
    pub async fn set_ban(
        &self,
        user_id: i64,
        until: Option<i64>,
        reason: Option<&str>,
    ) -> StoreResult<()> {
        dispatch!(self, s => s.set_ban(user_id, until, reason))
    }
    pub async fn set_mute(&self, user_id: i64, until: Option<i64>) -> StoreResult<()> {
        dispatch!(self, s => s.set_mute(user_id, until))
    }
    pub async fn set_role(&self, user_id: i64, role: Role) -> StoreResult<()> {
        dispatch!(self, s => s.set_role(user_id, role))
    }
    pub async fn set_password(&self, user_id: i64, hash: &str) -> StoreResult<()> {
        dispatch!(self, s => s.set_password(user_id, hash))
    }
    pub async fn reports(&self, status: Option<i16>, limit: u32) -> StoreResult<Vec<ReportRow>> {
        dispatch!(self, s => s.reports(status, limit))
    }
    pub async fn reports_about(&self, user_id: i64, limit: u32) -> StoreResult<Vec<ReportRow>> {
        dispatch!(self, s => s.reports_about(user_id, limit))
    }
    pub async fn resolve_report(
        &self,
        id: i64,
        staff: i64,
        status: i16,
        note: &str,
        at: i64,
    ) -> StoreResult<bool> {
        dispatch!(self, s => s.resolve_report(id, staff, status, note, at))
    }
    pub async fn audit(
        &self,
        staff: i64,
        action: &str,
        target: &str,
        details: &str,
        at: i64,
    ) -> StoreResult<()> {
        dispatch!(self, s => s.audit(staff, action, target, details, at))
    }
    pub async fn audit_log(&self, limit: u32) -> StoreResult<Vec<AuditRow>> {
        dispatch!(self, s => s.audit_log(limit))
    }

    // The shop.

    pub async fn wallet(&self, user_id: i64) -> StoreResult<Wallet> {
        dispatch!(self, s => s.wallet(user_id))
    }
    /// Takes `price` coins and gives the item, atomically.
    pub async fn buy_item(
        &self,
        user_id: i64,
        code: &str,
        price: i64,
        at: i64,
    ) -> StoreResult<BuyOutcome> {
        dispatch!(self, s => s.buy_item(user_id, code, price, at))
    }
    /// Gives an item for free (staff, promotions); false if already owned.
    pub async fn grant_item(
        &self,
        user_id: i64,
        code: &str,
        source: &str,
        at: i64,
    ) -> StoreResult<bool> {
        dispatch!(self, s => s.grant_item(user_id, code, source, at))
    }
    pub async fn equip(&self, user_id: i64, category: &str, code: &str) -> StoreResult<()> {
        dispatch!(self, s => s.equip(user_id, category, code))
    }
    /// Adds (or removes) coins. With a `reference`, the same change is only
    /// ever applied once; returns whether it was applied now.
    pub async fn credit(
        &self,
        user_id: i64,
        delta: i64,
        reason: &str,
        reference: Option<&str>,
        at: i64,
    ) -> StoreResult<bool> {
        dispatch!(self, s => s.credit(user_id, delta, reason, reference, at))
    }
    /// VIP until `until` (never shortens it unless `force`).
    pub async fn set_vip(&self, user_id: i64, until: i64, force: bool) -> StoreResult<()> {
        dispatch!(self, s => s.set_vip(user_id, until, force))
    }
    pub async fn set_stripe_customer(&self, user_id: i64, customer: &str) -> StoreResult<()> {
        dispatch!(self, s => s.set_stripe_customer(user_id, customer))
    }
    pub async fn user_by_stripe_customer(&self, customer: &str) -> StoreResult<Option<i64>> {
        dispatch!(self, s => s.user_by_stripe_customer(customer))
    }
    /// Records a payment; false if it was already recorded.
    pub async fn record_payment(&self, p: &Payment) -> StoreResult<bool> {
        dispatch!(self, s => s.record_payment(p))
    }
    pub async fn payment(&self, id: &str) -> StoreResult<Option<Payment>> {
        dispatch!(self, s => s.payment(id))
    }
    /// Marks a payment refunded; false if it already was (or is unknown).
    pub async fn refund_payment(&self, id: &str) -> StoreResult<bool> {
        dispatch!(self, s => s.refund_payment(id))
    }
    pub async fn ledger(&self, user_id: i64, limit: u32) -> StoreResult<Vec<LedgerRow>> {
        dispatch!(self, s => s.ledger(user_id, limit))
    }
    pub async fn payments_of(&self, user_id: i64, limit: u32) -> StoreResult<Vec<Payment>> {
        dispatch!(self, s => s.payments_of(user_id, limit))
    }
}

/// Updates a player's numbers after a match (the same rules as the
/// database update in both stores).
pub fn apply_match_numbers(u: &mut User, m: &MatchRecord, seat: usize) {
    let won = seat == m.winner;
    if m.rated {
        u.rating += m.delta[seat];
        u.peak_rating = u.peak_rating.max(u.rating);
        u.games += 1;
        if won {
            u.wins += 1;
            u.streak += 1;
            u.best_streak = u.best_streak.max(u.streak);
        } else {
            u.streak = 0;
        }
    } else {
        u.casual_games += 1;
    }
    if won && m.rated {
        u.eight_wins += m.eight_win as u32;
        u.runouts += m.runout as u32;
        u.shutouts += m.shutout as u32;
    }
}
