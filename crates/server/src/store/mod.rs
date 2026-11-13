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

use snookered_core::stats::ShotStats;
use snookered_proto::replay::Replay;
use snookered_proto::{CareerNumbers, LeaderboardEntry, Role, Stats};

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
    /// Where a forgotten password can be sent (set by the player).
    pub recovery_email: Option<String>,
    /// The account was deleted (and anonymised) at this time.
    pub deleted_at: Option<i64>,
    /// Games lost by leaving or by running out of reconnection time.
    pub abandons: u32,
}

impl User {
    pub fn rating(&self) -> snookered_core::rating::Rating {
        snookered_core::rating::Rating { rating: self.rating, games: self.games }
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

/// A note on the lounge's board.
#[derive(Debug, Clone, PartialEq)]
pub struct BoardNote {
    pub id: i64,
    pub text: String,
    /// Who pinned it (empty if the account is gone).
    pub author: String,
    pub created_at: i64,
    pub expires_at: Option<i64>,
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
    pub reason: snookered_core::GameOverReason,
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
    /// The whole game (seed, breaker, strokes).
    pub replay: Option<Replay>,
    /// Each seat's numbers this game.
    pub stats: [ShotStats; 2],
    /// The seat that left the game unfinished (lost on the reconnection clock).
    pub abandoned: Option<usize>,
}

/// A game in progress, saved after every stroke.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveMatch {
    pub id: i64,
    pub players: [i64; 2],
    pub rated: bool,
    pub replay: Replay,
    pub updated_at: i64,
}

/// A stored game with its replay.
#[derive(Debug, Clone)]
pub struct StoredReplay {
    pub id: i64,
    pub players: [String; 2],
    pub countries: [String; 2],
    pub ratings: [i32; 2],
    pub winner: usize,
    pub rated: bool,
    pub at: i64,
    pub replay: Replay,
}

/// A player's potting in the hard bands, for the aim review queue.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AimRow {
    pub user_id: i64,
    pub username: String,
    pub rating: i32,
    pub attempts: u32,
    pub made: u32,
}

/// Numbers for the staff panel's metrics page.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Metrics {
    /// (day, active players, matches, new accounts), oldest first.
    pub days: Vec<(i64, i64, i64, i64)>,
    /// Players active on day N who came back on day N+1 / N+7 (percent),
    /// over the accounts created in the last 30 days.
    pub retention_d1: f64,
    pub retention_d7: f64,
    /// Accounts created in the last 30 days, and how many played 1 and 5 games.
    pub funnel: [i64; 3],
    /// Payments in the last 30 days by currency: (currency, cents, count).
    pub revenue: Vec<(String, i64, i64)>,
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
    pub match_id: i64,
    pub replay: bool,
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
    /// The e-mail the customer gave at checkout.
    pub email: Option<String>,
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
    pub entry_fee: u32,
    /// Who got which prize ("name:coins, …").
    pub prizes: String,
}

/// The storage back end (one per server, so the size difference doesn't matter).
#[allow(clippy::large_enum_variant)]
pub enum Store {
    Memory(std::sync::Arc<MemoryStore>),
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
        Store::Memory(Default::default())
    }

    /// Another handle on the same in-memory store (tests: two servers, one
    /// database). `None` for a database (connect again instead).
    pub fn share(&self) -> Option<Store> {
        match self {
            Store::Memory(m) => Some(Store::Memory(m.clone())),
            #[cfg(feature = "postgres")]
            Store::Postgres(_) => None,
        }
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
    /// Stores a match and updates both players' numbers; returns them
    /// updated and the match id.
    pub async fn record_match(&self, m: &MatchRecord) -> StoreResult<([Option<User>; 2], i64)> {
        dispatch!(self, s => s.record_match(m))
    }
    /// Lifetime numbers from the games with stored statistics.
    pub async fn career(&self, user_id: i64) -> StoreResult<CareerNumbers> {
        dispatch!(self, s => s.career(user_id))
    }
    /// (time, rating after) of the last `limit` rated games, oldest first.
    pub async fn rating_history(&self, user_id: i64, limit: u32) -> StoreResult<Vec<(i64, i32)>> {
        dispatch!(self, s => s.rating_history(user_id, limit))
    }
    pub async fn replay(&self, match_id: i64) -> StoreResult<Option<StoredReplay>> {
        dispatch!(self, s => s.replay(match_id))
    }
    /// Saves (or replaces) a game in progress.
    pub async fn save_live(&self, m: &LiveMatch) -> StoreResult<()> {
        dispatch!(self, s => s.save_live(m))
    }
    pub async fn delete_live(&self, id: i64) -> StoreResult<()> {
        dispatch!(self, s => s.delete_live(id))
    }
    /// The saved game in progress of this player, if any.
    pub async fn live_match_of(&self, user_id: i64) -> StoreResult<Option<LiveMatch>> {
        dispatch!(self, s => s.live_match_of(user_id))
    }
    /// Forgets saved games nobody came back to.
    pub async fn purge_live(&self, before: i64) -> StoreResult<u64> {
        dispatch!(self, s => s.purge_live(before))
    }
    /// Takes `amount` coins if the player has them; false otherwise.
    pub async fn spend(
        &self,
        user_id: i64,
        amount: i64,
        reason: &str,
        reference: Option<&str>,
        at: i64,
    ) -> StoreResult<bool> {
        dispatch!(self, s => s.spend(user_id, amount, reason, reference, at))
    }
    /// Adds attempts and pots per difficulty band.
    pub async fn add_shot_stats(
        &self,
        user_id: i64,
        bands: &[(usize, u32, u32)],
    ) -> StoreResult<()> {
        dispatch!(self, s => s.add_shot_stats(user_id, bands))
    }
    /// Potting in the given bands, for every player with at least `min` attempts.
    pub async fn aim_rows(&self, bands: &[usize], min: u32) -> StoreResult<Vec<AimRow>> {
        dispatch!(self, s => s.aim_rows(bands, min))
    }
    /// Marks the player active on `day` (Unix day number).
    pub async fn mark_active(&self, user_id: i64, day: i64) -> StoreResult<()> {
        dispatch!(self, s => s.mark_active(user_id, day))
    }
    pub async fn metrics(&self, now: i64) -> StoreResult<Metrics> {
        dispatch!(self, s => s.metrics(now))
    }

    // Accounts.

    pub async fn set_recovery_email(&self, user_id: i64, email: Option<&str>) -> StoreResult<()> {
        dispatch!(self, s => s.set_recovery_email(user_id, email))
    }
    /// The e-mail given at this player's latest payment.
    pub async fn payment_email(&self, user_id: i64) -> StoreResult<Option<String>> {
        dispatch!(self, s => s.payment_email(user_id))
    }
    /// Deletes an account by anonymising it: the name becomes `deleted-<id>`,
    /// the password, e-mail, country, friends, blocks and sessions go; the
    /// games stay (the opponents keep their history) and so do payment
    /// records (accounting), without the e-mail.
    pub async fn delete_account(&self, user_id: i64, now: i64) -> StoreResult<()> {
        dispatch!(self, s => s.delete_account(user_id, now))
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
    pub async fn leaderboard(&self, limit: u32, now: i64) -> StoreResult<Vec<LeaderboardEntry>> {
        dispatch!(self, s => s.leaderboard(limit, now))
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
    /// This player's progress on missions: (period, code, progress) for
    /// the given periods.
    pub async fn mission_progress(
        &self,
        user_id: i64,
        periods: &[String],
    ) -> StoreResult<Vec<(String, String, u32)>> {
        dispatch!(self, s => s.mission_progress(user_id, periods))
    }
    /// Adds `amount` to a mission's progress (never past `target`); returns
    /// the new progress. `ends`: when the period ends (for clean-up).
    pub async fn advance_mission(
        &self,
        user_id: i64,
        period: &str,
        code: &str,
        amount: u32,
        target: u32,
        ends: i64,
    ) -> StoreResult<u32> {
        dispatch!(self, s => s.advance_mission(user_id, period, code, amount, target, ends))
    }
    /// Removes progress on missions that ended before `now`.
    pub async fn purge_missions(&self, now: i64) -> StoreResult<u64> {
        dispatch!(self, s => s.purge_missions(now))
    }
    /// The notes on the lounge's board that are up at `now`, newest first.
    pub async fn board_notes(&self, now: i64) -> StoreResult<Vec<BoardNote>> {
        dispatch!(self, s => s.board_notes(now))
    }
    pub async fn add_board_note(
        &self,
        text: &str,
        author_id: i64,
        now: i64,
        expires_at: Option<i64>,
    ) -> StoreResult<i64> {
        dispatch!(self, s => s.add_board_note(text, author_id, now, expires_at))
    }
    /// Takes a note down; false if there was none.
    pub async fn remove_board_note(&self, id: i64) -> StoreResult<bool> {
        dispatch!(self, s => s.remove_board_note(id))
    }
    /// The daily bonus: `coins` once per `day`; false if already claimed.
    pub async fn claim_daily(&self, user_id: i64, day: i64, coins: i64) -> StoreResult<bool> {
        dispatch!(self, s => s.claim_daily(user_id, day, coins))
    }
    /// Coins for playing, up to `cap` a day; returns what was given.
    pub async fn earn(
        &self,
        user_id: i64,
        day: i64,
        amount: i64,
        cap: i64,
        reason: &str,
        at: i64,
    ) -> StoreResult<i64> {
        dispatch!(self, s => s.earn(user_id, day, amount, cap, reason, at))
    }
    /// `from` pays `price` and `to` gets the item, atomically.
    pub async fn gift_item(
        &self,
        from: i64,
        to: i64,
        code: &str,
        price: i64,
        at: i64,
    ) -> StoreResult<BuyOutcome> {
        dispatch!(self, s => s.gift_item(from, to, code, price, at))
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
    if m.abandoned == Some(seat) {
        u.abandons += 1;
    }
}

/// The username a deleted account is left with.
pub fn deleted_name(id: i64) -> String {
    format!("deleted-{id}")
}

/// Unix day number.
pub fn day_of(t: i64) -> i64 {
    t.div_euclid(86_400)
}
