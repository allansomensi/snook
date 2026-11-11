//! PostgreSQL storage (production; works with Neon, Supabase, RDS…).

use snook_proto::{LeaderboardEntry, Role};
use sqlx::postgres::{PgPool, PgPoolOptions, PgRow};
use sqlx::{Row, query};

use super::{
    AuditRow, FriendOutcome, FriendState, HistoryRow, MatchRecord, NewReport, NewUser, Overview,
    ReportRow, SessionKind, StoreError, StoreResult, TournamentRecord, User,
};

pub struct PgStore {
    pool: PgPool,
}

impl From<sqlx::Error> for StoreError {
    fn from(e: sqlx::Error) -> Self {
        StoreError(e.to_string())
    }
}

impl From<sqlx::migrate::MigrateError> for StoreError {
    fn from(e: sqlx::migrate::MigrateError) -> Self {
        StoreError(e.to_string())
    }
}

const USER_COLUMNS: &str = "id, username, password_hash, country, role, rating, peak_rating, \
    games, wins, eight_wins, casual_games, streak, best_streak, runouts, shutouts, \
    tournaments_played, tournaments_won, created_at, last_seen, banned_until, ban_reason, muted_until";

fn user(r: &PgRow) -> Result<User, sqlx::Error> {
    let n = |c: &str| -> Result<u32, sqlx::Error> { Ok(r.try_get::<i32, _>(c)?.max(0) as u32) };
    Ok(User {
        id: r.try_get("id")?,
        username: r.try_get("username")?,
        password_hash: r.try_get("password_hash")?,
        country: r.try_get("country")?,
        role: Role::from_i16(r.try_get("role")?),
        rating: r.try_get("rating")?,
        peak_rating: r.try_get("peak_rating")?,
        games: n("games")?,
        wins: n("wins")?,
        eight_wins: n("eight_wins")?,
        casual_games: n("casual_games")?,
        streak: n("streak")?,
        best_streak: n("best_streak")?,
        runouts: n("runouts")?,
        shutouts: n("shutouts")?,
        tournaments_played: n("tournaments_played")?,
        tournaments_won: n("tournaments_won")?,
        created_at: r.try_get("created_at")?,
        last_seen: r.try_get("last_seen")?,
        banned_until: r.try_get("banned_until")?,
        ban_reason: r.try_get("ban_reason")?,
        muted_until: r.try_get("muted_until")?,
    })
}

/// Escapes `%`, `_` and `\` for a LIKE pattern.
fn like_pattern(q: &str) -> String {
    let mut out = String::from("%");
    for c in q.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

impl PgStore {
    pub async fn connect(url: &str, max_connections: u32) -> StoreResult<PgStore> {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections.max(1))
            .min_connections(0)
            .acquire_timeout(std::time::Duration::from_secs(15))
            // Serverless databases (Neon) close idle connections themselves.
            .idle_timeout(std::time::Duration::from_secs(240))
            .test_before_acquire(true)
            .connect(url)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(PgStore { pool })
    }

    pub async fn create_user(&self, new: &NewUser<'_>) -> StoreResult<Option<User>> {
        let now = crate::now();
        let row = query(&format!(
            "INSERT INTO users (username, password_hash, country, role, rating, peak_rating, created_at, last_seen, register_ip)
             VALUES ($1, $2, $3, $4, $5, $5, $6, $6, $7)
             ON CONFLICT DO NOTHING RETURNING {USER_COLUMNS}"
        ))
        .bind(new.username)
        .bind(new.password_hash)
        .bind(new.country)
        .bind(new.role.as_i16())
        .bind(new.rating)
        .bind(now)
        .bind(new.ip)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| user(&r)).transpose()?)
    }

    pub async fn user_by_name(&self, name: &str) -> StoreResult<Option<User>> {
        let row = query(&format!("SELECT {USER_COLUMNS} FROM users WHERE lower(username) = lower($1)"))
            .bind(name)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| user(&r)).transpose()?)
    }

    pub async fn user_by_id(&self, id: i64) -> StoreResult<Option<User>> {
        let row = query(&format!("SELECT {USER_COLUMNS} FROM users WHERE id = $1"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| user(&r)).transpose()?)
    }

    pub async fn create_session(
        &self,
        hash: &str,
        user_id: i64,
        kind: SessionKind,
        created: i64,
        expires: i64,
    ) -> StoreResult<()> {
        query("INSERT INTO sessions (token_hash, user_id, kind, created_at, expires_at) VALUES ($1, $2, $3, $4, $5)")
            .bind(hash)
            .bind(user_id)
            .bind(kind as i16)
            .bind(created)
            .bind(expires)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn session_user(
        &self,
        hash: &str,
        kind: SessionKind,
        now: i64,
    ) -> StoreResult<Option<(User, i64)>> {
        let cols = USER_COLUMNS
            .split(", ")
            .map(|c| format!("u.{}", c.trim()))
            .collect::<Vec<_>>()
            .join(", ");
        let row = query(&format!(
            "SELECT {cols}, s.created_at AS session_created FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = $1 AND s.kind = $2 AND s.expires_at > $3"
        ))
        .bind(hash)
        .bind(kind as i16)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;
        match row {
            Some(r) => Ok(Some((user(&r)?, r.try_get("session_created")?))),
            None => Ok(None),
        }
    }

    pub async fn delete_session(&self, hash: &str) -> StoreResult<()> {
        query("DELETE FROM sessions WHERE token_hash = $1").bind(hash).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn delete_user_sessions(&self, user_id: i64) -> StoreResult<()> {
        query("DELETE FROM sessions WHERE user_id = $1").bind(user_id).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn purge_sessions(&self, now: i64) -> StoreResult<u64> {
        let r = query("DELETE FROM sessions WHERE expires_at <= $1")
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }

    pub async fn set_country(&self, user_id: i64, country: &str) -> StoreResult<()> {
        query("UPDATE users SET country = $1 WHERE id = $2")
            .bind(country)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn touch(&self, user_id: i64, now: i64) -> StoreResult<()> {
        query("UPDATE users SET last_seen = $1 WHERE id = $2")
            .bind(now)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn record_match(&self, m: &MatchRecord) -> StoreResult<[Option<User>; 2]> {
        let mut tx = self.pool.begin().await?;
        let mut out = [None, None];
        for (seat, slot) in out.iter_mut().enumerate() {
            let won = seat == m.winner;
            let (rated, delta) = (m.rated, m.delta[seat]);
            let bonus = |flag: bool| (won && flag) as i32;
            let row = query(&format!(
                "UPDATE users SET
                    rating = rating + $2,
                    peak_rating = GREATEST(peak_rating, rating + $2),
                    games = games + $3,
                    wins = wins + $4,
                    streak = CASE WHEN $3 = 0 THEN streak WHEN $4 = 1 THEN streak + 1 ELSE 0 END,
                    best_streak = GREATEST(best_streak, CASE WHEN $3 = 1 AND $4 = 1 THEN streak + 1 ELSE 0 END),
                    casual_games = casual_games + $5,
                    eight_wins = eight_wins + $6,
                    runouts = runouts + $7,
                    shutouts = shutouts + $8
                 WHERE id = $1 RETURNING {USER_COLUMNS}"
            ))
            .bind(m.players[seat])
            .bind(if rated { delta } else { 0 })
            .bind(rated as i32)
            .bind((rated && won) as i32)
            .bind((!rated) as i32)
            .bind(bonus(m.eight_win))
            .bind(bonus(m.runout))
            .bind(bonus(m.shutout))
            .fetch_optional(&mut *tx)
            .await?;
            *slot = row.map(|r| user(&r)).transpose()?;
        }
        query(
            "INSERT INTO matches (player0, player1, winner_seat, reason, rated, delta0, delta1, rating0, rating1, shots, tournament_id, ended_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(m.players[0])
        .bind(m.players[1])
        .bind(m.winner as i16)
        .bind(crate::reason_code(m.reason))
        .bind(m.rated)
        .bind(m.delta[0])
        .bind(m.delta[1])
        .bind(out[0].as_ref().map(|u| u.rating).unwrap_or(m.ratings_after[0]))
        .bind(out[1].as_ref().map(|u| u.rating).unwrap_or(m.ratings_after[1]))
        .bind(m.shots as i32)
        .bind(m.tournament)
        .bind(m.ended_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(out)
    }

    pub async fn add_achievements(
        &self,
        user_id: i64,
        codes: &[&str],
        at: i64,
    ) -> StoreResult<Vec<String>> {
        if codes.is_empty() {
            return Ok(Vec::new());
        }
        let codes: Vec<String> = codes.iter().map(|c| c.to_string()).collect();
        let rows = query(
            "INSERT INTO achievements (user_id, code, unlocked_at)
             SELECT $1, c, $3 FROM UNNEST($2::text[]) AS c
             ON CONFLICT DO NOTHING RETURNING code",
        )
        .bind(user_id)
        .bind(&codes)
        .bind(at)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(|r| r.get::<String, _>("code")).collect())
    }

    pub async fn achievements(&self, user_id: i64) -> StoreResult<Vec<(String, i64)>> {
        let rows = query(
            "SELECT code, unlocked_at FROM achievements WHERE user_id = $1 ORDER BY unlocked_at",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(|r| (r.get("code"), r.get("unlocked_at"))).collect())
    }

    pub async fn history(&self, user_id: i64, limit: u32) -> StoreResult<Vec<HistoryRow>> {
        let rows = query(
            "SELECT m.*, CASE WHEN m.player0 = $1 THEN 0 ELSE 1 END AS seat,
                    o.username AS opp_name, o.country AS opp_country
             FROM matches m
             JOIN users o ON o.id = CASE WHEN m.player0 = $1 THEN m.player1 ELSE m.player0 END
             WHERE m.player0 = $1 OR m.player1 = $1
             ORDER BY m.ended_at DESC, m.id DESC LIMIT $2",
        )
        .bind(user_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                let seat: i32 = r.try_get("seat")?;
                let pick = |a: &'static str, b: &'static str| if seat == 0 { (a, b) } else { (b, a) };
                let (my_delta, _) = pick("delta0", "delta1");
                let (my_rating, opp_rating) = pick("rating0", "rating1");
                let winner: i16 = r.try_get("winner_seat")?;
                Ok(HistoryRow {
                    opponent: r.try_get("opp_name")?,
                    opponent_country: r.try_get("opp_country")?,
                    opponent_rating: r.try_get(opp_rating)?,
                    won: winner as i32 == seat,
                    reason: r.try_get("reason")?,
                    rated: r.try_get("rated")?,
                    delta: r.try_get(my_delta)?,
                    rating_after: r.try_get(my_rating)?,
                    tournament: r.try_get::<Option<i64>, _>("tournament_id")?.is_some(),
                    at: r.try_get("ended_at")?,
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(Into::into)
    }

    pub async fn leaderboard(&self, limit: u32) -> StoreResult<Vec<LeaderboardEntry>> {
        let rows = query(
            "SELECT username, rating, games, wins, country FROM users
             WHERE games > 0 AND banned_until IS NULL
             ORDER BY rating DESC, wins DESC, username ASC LIMIT $1",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .enumerate()
            .map(|(i, r)| LeaderboardEntry {
                rank: i as u32 + 1,
                username: r.get("username"),
                rating: r.get("rating"),
                games: r.get::<i32, _>("games") as u32,
                wins: r.get::<i32, _>("wins") as u32,
                country: r.get("country"),
            })
            .collect())
    }

    pub async fn rank_of(&self, user_id: i64) -> StoreResult<Option<u32>> {
        let row = query(
            "SELECT (SELECT COUNT(*) FROM users o WHERE o.games > 0 AND o.banned_until IS NULL AND o.rating > u.rating) AS above
             FROM users u WHERE u.id = $1 AND u.games > 0",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.get::<i64, _>("above") as u32 + 1))
    }

    pub async fn friends(&self, user_id: i64) -> StoreResult<Vec<(User, FriendState)>> {
        let cols = USER_COLUMNS
            .split(", ")
            .map(|c| format!("u.{}", c.trim()))
            .collect::<Vec<_>>()
            .join(", ");
        let rows = query(&format!(
            "SELECT {cols}, 0 AS dir, f.accepted FROM friendships f JOIN users u ON u.id = f.friend_id WHERE f.user_id = $1
             UNION ALL
             SELECT {cols}, 1 AS dir, f.accepted FROM friendships f JOIN users u ON u.id = f.user_id
             WHERE f.friend_id = $1 AND NOT f.accepted"
        ))
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::new();
        for r in &rows {
            let accepted: bool = r.try_get("accepted")?;
            let dir: i32 = r.try_get("dir")?;
            let state = match (accepted, dir) {
                (true, _) => FriendState::Friends,
                (false, 0) => FriendState::Outgoing,
                _ => FriendState::Incoming,
            };
            out.push((user(r)?, state));
        }
        out.sort_by(|a, b| a.0.username.to_lowercase().cmp(&b.0.username.to_lowercase()));
        Ok(out)
    }

    pub async fn friend_request(&self, from: i64, to: i64, at: i64) -> StoreResult<FriendOutcome> {
        let mut tx = self.pool.begin().await?;
        let mine: Option<bool> =
            query("SELECT accepted FROM friendships WHERE user_id = $1 AND friend_id = $2")
                .bind(from)
                .bind(to)
                .fetch_optional(&mut *tx)
                .await?
                .map(|r| r.get("accepted"));
        let theirs: Option<bool> =
            query("SELECT accepted FROM friendships WHERE user_id = $1 AND friend_id = $2")
                .bind(to)
                .bind(from)
                .fetch_optional(&mut *tx)
                .await?
                .map(|r| r.get("accepted"));
        let outcome = match (mine, theirs) {
            (Some(true), _) => FriendOutcome::AlreadyFriends,
            (_, Some(false)) => {
                query("UPDATE friendships SET accepted = TRUE WHERE user_id = $1 AND friend_id = $2")
                    .bind(to)
                    .bind(from)
                    .execute(&mut *tx)
                    .await?;
                query(
                    "INSERT INTO friendships (user_id, friend_id, accepted, created_at) VALUES ($1, $2, TRUE, $3)
                     ON CONFLICT (user_id, friend_id) DO UPDATE SET accepted = TRUE",
                )
                .bind(from)
                .bind(to)
                .bind(at)
                .execute(&mut *tx)
                .await?;
                FriendOutcome::Accepted
            }
            (Some(false), _) => FriendOutcome::AlreadySent,
            (None, _) => {
                query("INSERT INTO friendships (user_id, friend_id, accepted, created_at) VALUES ($1, $2, FALSE, $3)")
                    .bind(from)
                    .bind(to)
                    .bind(at)
                    .execute(&mut *tx)
                    .await?;
                FriendOutcome::Sent
            }
        };
        tx.commit().await?;
        Ok(outcome)
    }

    pub async fn friend_accept(&self, me: i64, from: i64) -> StoreResult<bool> {
        let mut tx = self.pool.begin().await?;
        let updated = query(
            "UPDATE friendships SET accepted = TRUE WHERE user_id = $1 AND friend_id = $2 AND NOT accepted",
        )
        .bind(from)
        .bind(me)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if updated == 0 {
            return Ok(false);
        }
        query(
            "INSERT INTO friendships (user_id, friend_id, accepted, created_at) VALUES ($1, $2, TRUE, $3)
             ON CONFLICT (user_id, friend_id) DO UPDATE SET accepted = TRUE",
        )
        .bind(me)
        .bind(from)
        .bind(crate::now())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    pub async fn friend_remove(&self, a: i64, b: i64) -> StoreResult<()> {
        query(
            "DELETE FROM friendships WHERE (user_id = $1 AND friend_id = $2) OR (user_id = $2 AND friend_id = $1)",
        )
        .bind(a)
        .bind(b)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn friend_count(&self, user_id: i64) -> StoreResult<u32> {
        let r = query("SELECT COUNT(*) AS n FROM friendships WHERE user_id = $1 AND accepted")
            .bind(user_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(r.get::<i64, _>("n") as u32)
    }

    pub async fn block(&self, user_id: i64, blocked: i64) -> StoreResult<()> {
        query("INSERT INTO blocks (user_id, blocked_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(user_id)
            .bind(blocked)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn unblock(&self, user_id: i64, blocked: i64) -> StoreResult<()> {
        query("DELETE FROM blocks WHERE user_id = $1 AND blocked_id = $2")
            .bind(user_id)
            .bind(blocked)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn blocked(&self, user_id: i64) -> StoreResult<Vec<User>> {
        let cols = USER_COLUMNS
            .split(", ")
            .map(|c| format!("u.{}", c.trim()))
            .collect::<Vec<_>>()
            .join(", ");
        let rows = query(&format!(
            "SELECT {cols} FROM blocks b JOIN users u ON u.id = b.blocked_id WHERE b.user_id = $1"
        ))
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(user).collect::<Result<_, _>>()?)
    }

    pub async fn create_report(&self, r: &NewReport) -> StoreResult<()> {
        query(
            "INSERT INTO reports (reporter_id, target_id, reason, details, context, created_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(r.reporter)
        .bind(r.target)
        .bind(&r.reason)
        .bind(&r.details)
        .bind(&r.context)
        .bind(r.at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn record_tournament(&self, t: &TournamentRecord) -> StoreResult<()> {
        let mut tx = self.pool.begin().await?;
        query(
            "INSERT INTO tournaments (name, host_id, size, rated, players, winner_id, finished_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&t.name)
        .bind(t.host)
        .bind(t.size as i32)
        .bind(t.rated)
        .bind(t.players.len() as i32)
        .bind(t.winner)
        .bind(t.finished_at)
        .execute(&mut *tx)
        .await?;
        query(
            "UPDATE users SET tournaments_played = tournaments_played + 1,
                tournaments_won = tournaments_won + CASE WHEN id = $2 THEN 1 ELSE 0 END
             WHERE id = ANY($1)",
        )
        .bind(&t.players)
        .bind(t.winner.unwrap_or(-1))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn search_users(
        &self,
        q: &str,
        offset: u32,
        limit: u32,
    ) -> StoreResult<(Vec<User>, i64)> {
        let pattern = like_pattern(&q.to_lowercase());
        let rows = query(&format!(
            "SELECT {USER_COLUMNS} FROM users WHERE lower(username) LIKE $1
             ORDER BY id DESC OFFSET $2 LIMIT $3"
        ))
        .bind(&pattern)
        .bind(offset as i64)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        let total: i64 = query("SELECT COUNT(*) AS n FROM users WHERE lower(username) LIKE $1")
            .bind(&pattern)
            .fetch_one(&self.pool)
            .await?
            .get("n");
        Ok((rows.iter().map(user).collect::<Result<_, _>>()?, total))
    }

    pub async fn overview(&self, now: i64) -> StoreResult<Overview> {
        let week = now - 7 * 86_400;
        let r = query(
            "SELECT
                (SELECT COUNT(*) FROM users) AS users,
                (SELECT COUNT(*) FROM users WHERE created_at > $1) AS new_users,
                (SELECT COUNT(*) FROM users WHERE last_seen > $1) AS active_users,
                (SELECT COUNT(*) FROM matches) AS matches_total,
                (SELECT COUNT(*) FROM matches WHERE ended_at > $2) AS matches_day,
                (SELECT COUNT(*) FROM reports WHERE status = 0) AS open_reports,
                (SELECT COUNT(*) FROM users WHERE banned_until > $3) AS banned",
        )
        .bind(week)
        .bind(now - 86_400)
        .bind(now)
        .fetch_one(&self.pool)
        .await?;
        Ok(Overview {
            users: r.get("users"),
            new_users_7d: r.get("new_users"),
            active_users_7d: r.get("active_users"),
            matches_total: r.get("matches_total"),
            matches_24h: r.get("matches_day"),
            open_reports: r.get("open_reports"),
            banned: r.get("banned"),
        })
    }

    pub async fn set_ban(
        &self,
        user_id: i64,
        until: Option<i64>,
        reason: Option<&str>,
    ) -> StoreResult<()> {
        query("UPDATE users SET banned_until = $1, ban_reason = $2 WHERE id = $3")
            .bind(until)
            .bind(reason)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_mute(&self, user_id: i64, until: Option<i64>) -> StoreResult<()> {
        query("UPDATE users SET muted_until = $1 WHERE id = $2")
            .bind(until)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_role(&self, user_id: i64, role: Role) -> StoreResult<()> {
        query("UPDATE users SET role = $1 WHERE id = $2")
            .bind(role.as_i16())
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_password(&self, user_id: i64, hash: &str) -> StoreResult<()> {
        query("UPDATE users SET password_hash = $1 WHERE id = $2")
            .bind(hash)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    fn report(r: &PgRow) -> Result<ReportRow, sqlx::Error> {
        Ok(ReportRow {
            id: r.try_get("id")?,
            reporter: r.try_get("reporter")?,
            target: r.try_get("target")?,
            reason: r.try_get("reason")?,
            details: r.try_get("details")?,
            context: r.try_get("context")?,
            status: r.try_get("status")?,
            created_at: r.try_get("created_at")?,
            resolved_by: r.try_get("resolver")?,
            resolution: r.try_get("resolution")?,
        })
    }

    const REPORT_SELECT: &str = "SELECT r.id, a.username AS reporter, t.username AS target, r.reason, r.details,
            r.context, r.status, r.created_at, s.username AS resolver, r.resolution
         FROM reports r JOIN users a ON a.id = r.reporter_id JOIN users t ON t.id = r.target_id
         LEFT JOIN users s ON s.id = r.resolved_by";

    pub async fn reports(&self, status: Option<i16>, limit: u32) -> StoreResult<Vec<ReportRow>> {
        let rows = query(&format!(
            "{} WHERE ($1::smallint IS NULL OR r.status = $1) ORDER BY r.created_at DESC LIMIT $2",
            Self::REPORT_SELECT
        ))
        .bind(status)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(Self::report).collect::<Result<_, _>>()?)
    }

    pub async fn reports_about(&self, user_id: i64, limit: u32) -> StoreResult<Vec<ReportRow>> {
        let rows = query(&format!(
            "{} WHERE r.target_id = $1 ORDER BY r.created_at DESC LIMIT $2",
            Self::REPORT_SELECT
        ))
        .bind(user_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(Self::report).collect::<Result<_, _>>()?)
    }

    pub async fn resolve_report(
        &self,
        id: i64,
        staff: i64,
        status: i16,
        note: &str,
        at: i64,
    ) -> StoreResult<bool> {
        let r = query(
            "UPDATE reports SET status = $1, resolved_by = $2, resolution = $3, resolved_at = $4 WHERE id = $5",
        )
        .bind(status)
        .bind(staff)
        .bind(note)
        .bind(at)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn audit(
        &self,
        staff: i64,
        action: &str,
        target: &str,
        details: &str,
        at: i64,
    ) -> StoreResult<()> {
        query("INSERT INTO audit_log (staff_id, action, target, details, at) VALUES ($1, $2, $3, $4, $5)")
            .bind(staff)
            .bind(action)
            .bind(target)
            .bind(details)
            .bind(at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn audit_log(&self, limit: u32) -> StoreResult<Vec<AuditRow>> {
        let rows = query(
            "SELECT l.id, COALESCE(u.username, '?') AS staff, l.action, l.target, l.details, l.at
             FROM audit_log l LEFT JOIN users u ON u.id = l.staff_id ORDER BY l.id DESC LIMIT $1",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok(AuditRow {
                    id: r.try_get("id")?,
                    staff: r.try_get("staff")?,
                    action: r.try_get("action")?,
                    target: r.try_get("target")?,
                    details: r.try_get("details")?,
                    at: r.try_get("at")?,
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(Into::into)
    }
}
