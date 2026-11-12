//! PostgreSQL storage (production; works with Neon, Supabase, RDS…).

use snookered_proto::{LeaderboardEntry, Role};
use sqlx::postgres::{PgPool, PgPoolOptions, PgRow};
use sqlx::{Row, query};

use snookered_proto::CareerNumbers;
use snookered_proto::replay::Replay;

use super::{
    AimRow, AuditRow, BuyOutcome, FriendOutcome, FriendState, HistoryRow, LedgerRow, LiveMatch,
    MatchRecord, Metrics, NewReport, NewUser, Overview, Payment, ReportRow, SessionKind,
    StoreError, StoreResult, StoredReplay, TournamentRecord, User, Wallet, day_of,
};

fn replay_json(r: &Replay) -> String {
    serde_json::to_string(&r.strokes).unwrap_or_else(|_| "[]".into())
}

fn replay_of(seed: i64, breaker: i16, strokes: &str) -> Option<Replay> {
    Some(Replay {
        seed: seed as u64,
        breaker: breaker as usize % 2,
        strokes: serde_json::from_str(strokes).ok()?,
    })
}

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
    tournaments_played, tournaments_won, created_at, last_seen, banned_until, ban_reason, muted_until, \
    recovery_email, deleted_at, abandons";

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
        recovery_email: r.try_get("recovery_email")?,
        deleted_at: r.try_get("deleted_at")?,
        abandons: n("abandons")?,
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
        let row =
            query(&format!("SELECT {USER_COLUMNS} FROM users WHERE lower(username) = lower($1)"))
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

    pub async fn record_match(&self, m: &MatchRecord) -> StoreResult<([Option<User>; 2], i64)> {
        let mut tx = self.pool.begin().await?;
        let mut out = [None, None];
        for (seat, slot) in out.iter_mut().enumerate() {
            let won = seat == m.winner;
            let (rated, delta) = (m.rated, m.delta[seat]);
            // Feats only count in rated games (casual ones are easy to arrange).
            let bonus = |flag: bool| (rated && won && flag) as i32;
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
                    shutouts = shutouts + $8,
                    abandons = abandons + $9
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
            .bind((m.abandoned == Some(seat)) as i32)
            .fetch_optional(&mut *tx)
            .await?;
            *slot = row.map(|r| user(&r)).transpose()?;
        }
        let id: i64 = query(
            "INSERT INTO matches (player0, player1, winner_seat, reason, rated, delta0, delta1, rating0, rating1, shots, tournament_id, ended_at, seed, breaker, strokes)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15::jsonb) RETURNING id",
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
        .bind(m.replay.as_ref().map(|r| r.seed as i64))
        .bind(m.replay.as_ref().map(|r| r.breaker as i16))
        .bind(m.replay.as_ref().map(replay_json))
        .fetch_one(&mut *tx)
        .await?
        .get("id");
        for seat in 0..2 {
            let st = &m.stats[seat];
            if st.shots == 0 {
                continue;
            }
            query(
                "INSERT INTO match_stats (match_id, user_id, shots, potting_shots, balls, fouls, best_run, in_hand)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT DO NOTHING",
            )
            .bind(id)
            .bind(m.players[seat])
            .bind(st.shots as i32)
            .bind(st.potting_shots as i32)
            .bind(st.balls as i32)
            .bind(st.fouls as i32)
            .bind(st.best_run as i32)
            .bind(st.in_hand as i32)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok((out, id))
    }

    pub async fn career(&self, user_id: i64) -> StoreResult<CareerNumbers> {
        let r = query(
            "SELECT COUNT(*) AS games, COALESCE(SUM(shots), 0) AS shots,
                    COALESCE(SUM(potting_shots), 0) AS potting, COALESCE(SUM(balls), 0) AS balls,
                    COALESCE(SUM(fouls), 0) AS fouls, COALESCE(MAX(best_run), 0) AS best
             FROM match_stats WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        let n = |c: &str| r.get::<i64, _>(c).max(0) as u32;
        Ok(CareerNumbers {
            games: n("games"),
            shots: n("shots"),
            potting_shots: n("potting"),
            balls: n("balls"),
            fouls: n("fouls"),
            best_run: r.get::<i32, _>("best").max(0) as u32,
        })
    }

    pub async fn rating_history(&self, user_id: i64, limit: u32) -> StoreResult<Vec<(i64, i32)>> {
        let rows = query(
            "SELECT ended_at, CASE WHEN player0 = $1 THEN rating0 ELSE rating1 END AS rating
             FROM matches WHERE rated AND (player0 = $1 OR player1 = $1)
             ORDER BY ended_at DESC, id DESC LIMIT $2",
        )
        .bind(user_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        let mut list: Vec<(i64, i32)> =
            rows.iter().map(|r| (r.get("ended_at"), r.get("rating"))).collect();
        list.reverse();
        Ok(list)
    }

    pub async fn replay(&self, match_id: i64) -> StoreResult<Option<StoredReplay>> {
        let row = query(
            "SELECT m.id, m.winner_seat, m.rated, m.ended_at, m.rating0 - m.delta0 AS r0,
                    m.rating1 - m.delta1 AS r1, m.seed, m.breaker, m.strokes::text AS strokes,
                    a.username AS n0, a.country AS c0, b.username AS n1, b.country AS c1
             FROM matches m JOIN users a ON a.id = m.player0 JOIN users b ON b.id = m.player1
             WHERE m.id = $1 AND m.strokes IS NOT NULL",
        )
        .bind(match_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(r) = row else { return Ok(None) };
        let strokes: String = r.get("strokes");
        let Some(replay) = replay_of(r.get("seed"), r.get("breaker"), &strokes) else {
            return Ok(None);
        };
        Ok(Some(StoredReplay {
            id: r.get("id"),
            players: [r.get("n0"), r.get("n1")],
            countries: [r.get("c0"), r.get("c1")],
            ratings: [r.get("r0"), r.get("r1")],
            winner: r.get::<i16, _>("winner_seat") as usize % 2,
            rated: r.get("rated"),
            at: r.get("ended_at"),
            replay,
        }))
    }

    pub async fn save_live(&self, m: &LiveMatch) -> StoreResult<()> {
        query(
            "INSERT INTO live_matches (id, player0, player1, rated, seed, breaker, strokes, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb, $8)
             ON CONFLICT (id) DO UPDATE SET strokes = EXCLUDED.strokes, updated_at = EXCLUDED.updated_at",
        )
        .bind(m.id)
        .bind(m.players[0])
        .bind(m.players[1])
        .bind(m.rated)
        .bind(m.replay.seed as i64)
        .bind(m.replay.breaker as i16)
        .bind(replay_json(&m.replay))
        .bind(m.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_live(&self, id: i64) -> StoreResult<()> {
        query("DELETE FROM live_matches WHERE id = $1").bind(id).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn live_match_of(&self, user_id: i64) -> StoreResult<Option<LiveMatch>> {
        let row = query(
            "SELECT id, player0, player1, rated, seed, breaker, strokes::text AS strokes, updated_at
             FROM live_matches WHERE player0 = $1 OR player1 = $1 ORDER BY updated_at DESC LIMIT 1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.and_then(|r| {
            let strokes: String = r.get("strokes");
            Some(LiveMatch {
                id: r.get("id"),
                players: [r.get("player0"), r.get("player1")],
                rated: r.get("rated"),
                replay: replay_of(r.get("seed"), r.get("breaker"), &strokes)?,
                updated_at: r.get("updated_at"),
            })
        }))
    }

    pub async fn purge_live(&self, before: i64) -> StoreResult<u64> {
        Ok(query("DELETE FROM live_matches WHERE updated_at < $1")
            .bind(before)
            .execute(&self.pool)
            .await?
            .rows_affected())
    }

    pub async fn spend(
        &self,
        user_id: i64,
        amount: i64,
        reason: &str,
        reference: Option<&str>,
        at: i64,
    ) -> StoreResult<bool> {
        let mut tx = self.pool.begin().await?;
        let n = query("UPDATE wallets SET coins = coins - $2 WHERE user_id = $1 AND coins >= $2")
            .bind(user_id)
            .bind(amount)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if n == 0 {
            return Ok(false);
        }
        query(
            "INSERT INTO coin_ledger (user_id, delta, reason, reference, at) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(user_id)
        .bind(-amount)
        .bind(reason)
        .bind(reference)
        .bind(at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    pub async fn add_shot_stats(
        &self,
        user_id: i64,
        bands: &[(usize, u32, u32)],
    ) -> StoreResult<()> {
        if bands.is_empty() {
            return Ok(());
        }
        let b: Vec<i16> = bands.iter().map(|x| x.0 as i16).collect();
        let a: Vec<i32> = bands.iter().map(|x| x.1 as i32).collect();
        let m: Vec<i32> = bands.iter().map(|x| x.2 as i32).collect();
        query(
            "INSERT INTO shot_stats (user_id, band, attempts, made)
             SELECT $1, b, a, m FROM UNNEST($2::smallint[], $3::int[], $4::int[]) AS t(b, a, m)
             ON CONFLICT (user_id, band) DO UPDATE SET
                attempts = shot_stats.attempts + EXCLUDED.attempts,
                made = shot_stats.made + EXCLUDED.made",
        )
        .bind(user_id)
        .bind(&b)
        .bind(&a)
        .bind(&m)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn aim_rows(&self, bands: &[usize], min: u32) -> StoreResult<Vec<AimRow>> {
        let b: Vec<i16> = bands.iter().map(|x| *x as i16).collect();
        let rows = query(
            "SELECT u.id, u.username, u.rating, SUM(s.attempts) AS attempts, SUM(s.made) AS made
             FROM shot_stats s JOIN users u ON u.id = s.user_id
             WHERE s.band = ANY($1) AND u.deleted_at IS NULL
             GROUP BY u.id HAVING SUM(s.attempts) >= $2",
        )
        .bind(&b)
        .bind(min as i64)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|r| AimRow {
                user_id: r.get("id"),
                username: r.get("username"),
                rating: r.get("rating"),
                attempts: r.get::<i64, _>("attempts").max(0) as u32,
                made: r.get::<i64, _>("made").max(0) as u32,
            })
            .collect())
    }

    pub async fn mark_active(&self, user_id: i64, day: i64) -> StoreResult<()> {
        query("INSERT INTO activity (day, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(day)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn metrics(&self, now: i64) -> StoreResult<Metrics> {
        let today = day_of(now);
        let first = today - 29;
        let rows = query(
            "SELECT d.day,
                (SELECT COUNT(*) FROM activity a WHERE a.day = d.day) AS active,
                (SELECT COUNT(*) FROM matches m WHERE m.ended_at >= d.day * 86400 AND m.ended_at < (d.day + 1) * 86400) AS matches,
                (SELECT COUNT(*) FROM users u WHERE u.created_at >= d.day * 86400 AND u.created_at < (d.day + 1) * 86400) AS new
             FROM generate_series($1::bigint, $2::bigint) AS d(day) ORDER BY d.day",
        )
        .bind(first)
        .bind(today)
        .fetch_all(&self.pool)
        .await?;
        let days = rows
            .iter()
            .map(|r| (r.get("day"), r.get("active"), r.get("matches"), r.get("new")))
            .collect();
        let retention = |after: i64| async move {
            let r = query(
                "SELECT COUNT(*) AS eligible,
                    COUNT(*) FILTER (WHERE EXISTS (SELECT 1 FROM activity a
                        WHERE a.user_id = u.id AND a.day = u.created_at / 86400 + $2)) AS came
                 FROM users u WHERE u.created_at / 86400 >= $1 AND u.created_at / 86400 + $2 <= $3",
            )
            .bind(today - 30)
            .bind(after)
            .bind(today)
            .fetch_one(&self.pool)
            .await?;
            let (e, c): (i64, i64) = (r.get("eligible"), r.get("came"));
            Ok::<f64, sqlx::Error>(if e == 0 { 0.0 } else { c as f64 * 100.0 / e as f64 })
        };
        let retention_d1 = retention(1).await?;
        let retention_d7 = retention(7).await?;
        let f = query(
            "SELECT COUNT(*) AS created,
                COUNT(*) FILTER (WHERE games + casual_games >= 1) AS one,
                COUNT(*) FILTER (WHERE games + casual_games >= 5) AS five
             FROM users WHERE created_at / 86400 >= $1",
        )
        .bind(today - 30)
        .fetch_one(&self.pool)
        .await?;
        let revenue = query(
            "SELECT currency, SUM(amount)::bigint AS cents, COUNT(*) AS n FROM payments
             WHERE NOT refunded AND at > $1 GROUP BY currency ORDER BY currency",
        )
        .bind(now - 30 * 86_400)
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(|r| (r.get("currency"), r.get("cents"), r.get("n")))
        .collect();
        Ok(Metrics {
            days,
            retention_d1,
            retention_d7,
            funnel: [f.get("created"), f.get("one"), f.get("five")],
            revenue,
        })
    }

    pub async fn set_recovery_email(&self, user_id: i64, email: Option<&str>) -> StoreResult<()> {
        query("UPDATE users SET recovery_email = $2 WHERE id = $1")
            .bind(user_id)
            .bind(email)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn payment_email(&self, user_id: i64) -> StoreResult<Option<String>> {
        let row = query(
            "SELECT email FROM payments WHERE user_id = $1 AND email IS NOT NULL
             ORDER BY at DESC LIMIT 1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.get("email")))
    }

    pub async fn delete_account(&self, user_id: i64, now: i64) -> StoreResult<()> {
        let mut tx = self.pool.begin().await?;
        query(
            "UPDATE users SET username = $2, password_hash = '', country = '',
                recovery_email = NULL, deleted_at = $3, register_ip = ''
             WHERE id = $1",
        )
        .bind(user_id)
        .bind(super::deleted_name(user_id))
        .bind(now)
        .execute(&mut *tx)
        .await?;
        for sql in [
            "DELETE FROM sessions WHERE user_id = $1",
            "DELETE FROM friendships WHERE user_id = $1 OR friend_id = $1",
            "DELETE FROM blocks WHERE user_id = $1 OR blocked_id = $1",
            "DELETE FROM achievements WHERE user_id = $1",
            "DELETE FROM equipped WHERE user_id = $1",
            "DELETE FROM shot_stats WHERE user_id = $1",
            "DELETE FROM live_matches WHERE player0 = $1 OR player1 = $1",
            "UPDATE wallets SET stripe_customer = NULL WHERE user_id = $1",
            "UPDATE payments SET email = NULL WHERE user_id = $1",
        ] {
            query(sql).bind(user_id).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn claim_daily(&self, user_id: i64, day: i64, coins: i64) -> StoreResult<bool> {
        let mut tx = self.pool.begin().await?;
        query("INSERT INTO wallets (user_id) VALUES ($1) ON CONFLICT DO NOTHING")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        let n = query(
            "UPDATE wallets SET daily_day = $2, coins = coins + $3
             WHERE user_id = $1 AND daily_day < $2",
        )
        .bind(user_id)
        .bind(day)
        .bind(coins)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if n > 0 {
            query(
                "INSERT INTO coin_ledger (user_id, delta, reason, reference, at)
                 VALUES ($1, $2, 'daily bonus', NULL, $3)",
            )
            .bind(user_id)
            .bind(coins)
            .bind(day * 86_400)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(n > 0)
    }

    pub async fn earn(
        &self,
        user_id: i64,
        day: i64,
        amount: i64,
        cap: i64,
        reason: &str,
        at: i64,
    ) -> StoreResult<i64> {
        let mut tx = self.pool.begin().await?;
        query("INSERT INTO wallets (user_id) VALUES ($1) ON CONFLICT DO NOTHING")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        // Lock the row, start a new day if needed, give what fits under the cap.
        let r = query(
            "SELECT CASE WHEN earned_day = $2 THEN earned_today ELSE 0 END AS so_far
             FROM wallets WHERE user_id = $1 FOR UPDATE",
        )
        .bind(user_id)
        .bind(day)
        .fetch_one(&mut *tx)
        .await?;
        let so_far: i32 = r.get("so_far");
        let given = amount.min(cap - so_far as i64).max(0);
        if given > 0 {
            query(
                "UPDATE wallets SET earned_day = $2, earned_today = $3, coins = coins + $4
                 WHERE user_id = $1",
            )
            .bind(user_id)
            .bind(day)
            .bind(so_far + given as i32)
            .bind(given)
            .execute(&mut *tx)
            .await?;
            query(
                "INSERT INTO coin_ledger (user_id, delta, reason, reference, at)
                 VALUES ($1, $2, $3, NULL, $4)",
            )
            .bind(user_id)
            .bind(given)
            .bind(reason)
            .bind(at)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(given)
    }

    pub async fn gift_item(
        &self,
        from: i64,
        to: i64,
        code: &str,
        price: i64,
        at: i64,
    ) -> StoreResult<BuyOutcome> {
        let mut tx = self.pool.begin().await?;
        let added = query(
            "INSERT INTO owned_items (user_id, code, acquired_at, source) VALUES ($1, $2, $3, $4)
             ON CONFLICT DO NOTHING",
        )
        .bind(to)
        .bind(code)
        .bind(at)
        .bind(format!("gift:{from}"))
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if added == 0 {
            return Ok(BuyOutcome::AlreadyOwned);
        }
        let paid =
            query("UPDATE wallets SET coins = coins - $2 WHERE user_id = $1 AND coins >= $2")
                .bind(from)
                .bind(price)
                .execute(&mut *tx)
                .await?
                .rows_affected();
        if paid == 0 {
            return Ok(BuyOutcome::NotEnoughCoins);
        }
        query(
            "INSERT INTO coin_ledger (user_id, delta, reason, reference, at)
             VALUES ($1, $2, $3, NULL, $4)",
        )
        .bind(from)
        .bind(-price)
        .bind(format!("gift {code} to {to}"))
        .bind(at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(BuyOutcome::Bought)
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
                let pick =
                    |a: &'static str, b: &'static str| if seat == 0 { (a, b) } else { (b, a) };
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
                    match_id: r.try_get("id")?,
                    replay: r.try_get::<Option<i64>, _>("seed")?.is_some(),
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(Into::into)
    }

    pub async fn leaderboard(&self, limit: u32, now: i64) -> StoreResult<Vec<LeaderboardEntry>> {
        let rows = query(
            "SELECT u.username, u.rating, u.games, u.wins, u.country,
                    COALESCE(w.vip_until, 0) > $2 AS vip
             FROM users u LEFT JOIN wallets w ON w.user_id = u.id
             WHERE u.games > 0 AND (u.banned_until IS NULL OR u.banned_until <= $2)
               AND u.deleted_at IS NULL
             ORDER BY u.rating DESC, u.wins DESC, u.username ASC LIMIT $1",
        )
        .bind(limit as i64)
        .bind(now)
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
                vip: r.get("vip"),
            })
            .collect())
    }

    pub async fn rank_of(&self, user_id: i64) -> StoreResult<Option<u32>> {
        let row = query(
            "SELECT (SELECT COUNT(*) FROM users o WHERE o.games > 0 AND (o.banned_until IS NULL OR o.banned_until <= $2) AND o.deleted_at IS NULL AND o.rating > u.rating) AS above
             FROM users u WHERE u.id = $1 AND u.games > 0",
        )
        .bind(user_id)
        .bind(crate::now())
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
        out.sort_by_key(|a| a.0.username.to_lowercase());
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
                query(
                    "UPDATE friendships SET accepted = TRUE WHERE user_id = $1 AND friend_id = $2",
                )
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
            "INSERT INTO tournaments (name, host_id, size, rated, players, winner_id, finished_at, entry_fee, prizes)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(&t.name)
        .bind(t.host)
        .bind(t.size as i32)
        .bind(t.rated)
        .bind(t.players.len() as i32)
        .bind(t.winner)
        .bind(t.finished_at)
        .bind(t.entry_fee as i32)
        .bind(&t.prizes)
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
            "SELECT {USER_COLUMNS} FROM users
             WHERE lower(username) LIKE $1 OR lower(recovery_email) = $4
             ORDER BY id DESC OFFSET $2 LIMIT $3"
        ))
        .bind(&pattern)
        .bind(offset as i64)
        .bind(limit as i64)
        .bind(q.trim().to_lowercase())
        .fetch_all(&self.pool)
        .await?;
        let total: i64 = query(
            "SELECT COUNT(*) AS n FROM users WHERE lower(username) LIKE $1 OR lower(recovery_email) = $2",
        )
        .bind(&pattern)
        .bind(q.trim().to_lowercase())
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

    const REPORT_SELECT: &str =
        "SELECT r.id, a.username AS reporter, t.username AS target, r.reason, r.details,
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
    // The shop.

    pub async fn wallet(&self, user_id: i64) -> StoreResult<Wallet> {
        let mut w = Wallet::default();
        if let Some(r) =
            query("SELECT coins, vip_until, stripe_customer FROM wallets WHERE user_id = $1")
                .bind(user_id)
                .fetch_optional(&self.pool)
                .await?
        {
            w.coins = r.get("coins");
            w.vip_until = r.get("vip_until");
            w.stripe_customer = r.get("stripe_customer");
        }
        w.owned = query("SELECT code FROM owned_items WHERE user_id = $1 ORDER BY acquired_at")
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(|r| r.get("code"))
            .collect();
        w.equipped = query("SELECT category, code FROM equipped WHERE user_id = $1")
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(|r| (r.get("category"), r.get("code")))
            .collect();
        Ok(w)
    }

    pub async fn buy_item(
        &self,
        user_id: i64,
        code: &str,
        price: i64,
        at: i64,
    ) -> StoreResult<BuyOutcome> {
        let mut tx = self.pool.begin().await?;
        let added = query(
            "INSERT INTO owned_items (user_id, code, acquired_at, source) VALUES ($1, $2, $3, 'coins')
             ON CONFLICT DO NOTHING",
        )
        .bind(user_id)
        .bind(code)
        .bind(at)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if added == 0 {
            return Ok(BuyOutcome::AlreadyOwned);
        }
        // Takes the coins only if there are enough (never below zero).
        let paid =
            query("UPDATE wallets SET coins = coins - $2 WHERE user_id = $1 AND coins >= $2")
                .bind(user_id)
                .bind(price)
                .execute(&mut *tx)
                .await?
                .rows_affected();
        if paid == 0 {
            return Ok(BuyOutcome::NotEnoughCoins); // Rolled back on drop.
        }
        query("INSERT INTO coin_ledger (user_id, delta, reason, at) VALUES ($1, $2, $3, $4)")
            .bind(user_id)
            .bind(-price)
            .bind(format!("buy {code}"))
            .bind(at)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(BuyOutcome::Bought)
    }

    pub async fn grant_item(
        &self,
        user_id: i64,
        code: &str,
        source: &str,
        at: i64,
    ) -> StoreResult<bool> {
        let added = query(
            "INSERT INTO owned_items (user_id, code, acquired_at, source) VALUES ($1, $2, $3, $4)
             ON CONFLICT DO NOTHING",
        )
        .bind(user_id)
        .bind(code)
        .bind(at)
        .bind(source)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(added > 0)
    }

    pub async fn equip(&self, user_id: i64, category: &str, code: &str) -> StoreResult<()> {
        query(
            "INSERT INTO equipped (user_id, category, code) VALUES ($1, $2, $3)
             ON CONFLICT (user_id, category) DO UPDATE SET code = EXCLUDED.code",
        )
        .bind(user_id)
        .bind(category)
        .bind(code)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn credit(
        &self,
        user_id: i64,
        delta: i64,
        reason: &str,
        reference: Option<&str>,
        at: i64,
    ) -> StoreResult<bool> {
        let mut tx = self.pool.begin().await?;
        let inserted = query(
            "INSERT INTO coin_ledger (user_id, delta, reason, reference, at) VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (reference) DO NOTHING",
        )
        .bind(user_id)
        .bind(delta)
        .bind(reason)
        .bind(reference)
        .bind(at)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if inserted == 0 {
            return Ok(false);
        }
        query(
            "INSERT INTO wallets (user_id, coins) VALUES ($1, $2)
             ON CONFLICT (user_id) DO UPDATE SET coins = wallets.coins + EXCLUDED.coins",
        )
        .bind(user_id)
        .bind(delta)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    pub async fn set_vip(&self, user_id: i64, until: i64, force: bool) -> StoreResult<()> {
        let sql = if force {
            "INSERT INTO wallets (user_id, vip_until) VALUES ($1, $2)
             ON CONFLICT (user_id) DO UPDATE SET vip_until = EXCLUDED.vip_until"
        } else {
            "INSERT INTO wallets (user_id, vip_until) VALUES ($1, $2)
             ON CONFLICT (user_id) DO UPDATE SET vip_until = GREATEST(wallets.vip_until, EXCLUDED.vip_until)"
        };
        query(sql).bind(user_id).bind(until).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn set_stripe_customer(&self, user_id: i64, customer: &str) -> StoreResult<()> {
        query(
            "INSERT INTO wallets (user_id, stripe_customer) VALUES ($1, $2)
             ON CONFLICT (user_id) DO UPDATE SET stripe_customer = EXCLUDED.stripe_customer",
        )
        .bind(user_id)
        .bind(customer)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn user_by_stripe_customer(&self, customer: &str) -> StoreResult<Option<i64>> {
        let row = query("SELECT user_id FROM wallets WHERE stripe_customer = $1")
            .bind(customer)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get("user_id")))
    }

    pub async fn record_payment(&self, p: &Payment) -> StoreResult<bool> {
        let added = query(
            "INSERT INTO payments (id, user_id, kind, coins, amount, currency, at, refunded, email)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) ON CONFLICT DO NOTHING",
        )
        .bind(&p.id)
        .bind(p.user_id)
        .bind(&p.kind)
        .bind(p.coins)
        .bind(p.amount)
        .bind(&p.currency)
        .bind(p.at)
        .bind(p.refunded)
        .bind(&p.email)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(added > 0)
    }

    pub async fn payment(&self, id: &str) -> StoreResult<Option<Payment>> {
        let row = query("SELECT * FROM payments WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| payment_row(&r)))
    }

    pub async fn refund_payment(&self, id: &str) -> StoreResult<bool> {
        let n = query("UPDATE payments SET refunded = TRUE WHERE id = $1 AND NOT refunded")
            .bind(id)
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok(n > 0)
    }

    pub async fn ledger(&self, user_id: i64, limit: u32) -> StoreResult<Vec<LedgerRow>> {
        let rows = query(
            "SELECT delta, reason, reference, at FROM coin_ledger WHERE user_id = $1
             ORDER BY id DESC LIMIT $2",
        )
        .bind(user_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|r| LedgerRow {
                delta: r.get("delta"),
                reason: r.get("reason"),
                reference: r.get("reference"),
                at: r.get("at"),
            })
            .collect())
    }

    pub async fn payments_of(&self, user_id: i64, limit: u32) -> StoreResult<Vec<Payment>> {
        let rows = query("SELECT * FROM payments WHERE user_id = $1 ORDER BY at DESC LIMIT $2")
            .bind(user_id)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(payment_row).collect())
    }
}

fn payment_row(r: &PgRow) -> Payment {
    Payment {
        id: r.get("id"),
        user_id: r.get("user_id"),
        kind: r.get("kind"),
        coins: r.get("coins"),
        amount: r.get("amount"),
        currency: r.get("currency"),
        at: r.get("at"),
        refunded: r.get("refunded"),
        email: r.get("email"),
    }
}
