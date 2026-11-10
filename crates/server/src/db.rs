//! SQLite persistence: accounts, login tokens, ratings and match history.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use snook_core::rating::Rating;
use snook_proto::LeaderboardEntry;

pub struct Db {
    conn: Connection,
}

#[derive(Debug, Clone)]
pub struct UserRow {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub rating: i32,
    pub games: u32,
    pub wins: u32,
}

impl UserRow {
    pub fn rating(&self) -> Rating {
        Rating { rating: self.rating, games: self.games }
    }
}

/// How long a login token stays valid.
const TOKEN_LIFETIME_SECS: i64 = 60 * 60 * 24 * 30;

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Db {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS users (
                 id INTEGER PRIMARY KEY,
                 username TEXT NOT NULL UNIQUE COLLATE NOCASE,
                 password_hash TEXT NOT NULL,
                 rating INTEGER NOT NULL,
                 games INTEGER NOT NULL DEFAULT 0,
                 wins INTEGER NOT NULL DEFAULT 0,
                 created_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS tokens (
                 token TEXT PRIMARY KEY,
                 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                 created_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS matches (
                 id INTEGER PRIMARY KEY,
                 player0 INTEGER NOT NULL REFERENCES users(id),
                 player1 INTEGER NOT NULL REFERENCES users(id),
                 winner_seat INTEGER NOT NULL,
                 reason TEXT NOT NULL,
                 rated INTEGER NOT NULL,
                 delta0 INTEGER NOT NULL,
                 delta1 INTEGER NOT NULL,
                 shots INTEGER NOT NULL,
                 ended_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS matches_pair ON matches(player0, player1, ended_at);
             CREATE INDEX IF NOT EXISTS users_rating ON users(rating DESC);",
        )?;
        Ok(Db { conn })
    }

    pub fn create_user(
        &self,
        username: &str,
        password_hash: &str,
        rating: i32,
    ) -> rusqlite::Result<Option<UserRow>> {
        let inserted = self.conn.execute(
            "INSERT OR IGNORE INTO users (username, password_hash, rating, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![username, password_hash, rating, now()],
        )?;
        if inserted == 0 {
            return Ok(None);
        }
        self.user_by_name(username)
    }

    pub fn user_by_name(&self, username: &str) -> rusqlite::Result<Option<UserRow>> {
        self.conn
            .query_row(
                "SELECT id, username, password_hash, rating, games, wins FROM users WHERE username = ?1",
                params![username],
                row_to_user,
            )
            .optional()
    }

    pub fn user_by_id(&self, id: i64) -> rusqlite::Result<Option<UserRow>> {
        self.conn
            .query_row(
                "SELECT id, username, password_hash, rating, games, wins FROM users WHERE id = ?1",
                params![id],
                row_to_user,
            )
            .optional()
    }

    pub fn store_token(&self, token: &str, user_id: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO tokens (token, user_id, created_at) VALUES (?1, ?2, ?3)",
            params![token, user_id, now()],
        )?;
        Ok(())
    }

    pub fn user_by_token(&self, token: &str) -> rusqlite::Result<Option<UserRow>> {
        let id: Option<i64> = self
            .conn
            .query_row(
                "SELECT user_id FROM tokens WHERE token = ?1 AND created_at > ?2",
                params![token, now() - TOKEN_LIFETIME_SECS],
                |r| r.get(0),
            )
            .optional()?;
        match id {
            Some(id) => self.user_by_id(id),
            None => Ok(None),
        }
    }

    pub fn delete_token(&self, token: &str) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM tokens WHERE token = ?1", params![token])?;
        Ok(())
    }

    pub fn purge_expired_tokens(&self) -> rusqlite::Result<usize> {
        self.conn.execute(
            "DELETE FROM tokens WHERE created_at <= ?1",
            params![now() - TOKEN_LIFETIME_SECS],
        )
    }

    pub fn update_rating(&self, user_id: i64, rating: Rating, won: bool) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE users SET rating = ?1, games = ?2, wins = wins + ?3 WHERE id = ?4",
            params![rating.rating, rating.games, won as i32, user_id],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_match(
        &self,
        players: [i64; 2],
        winner_seat: usize,
        reason: &str,
        rated: bool,
        delta: [i32; 2],
        shots: u32,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO matches (player0, player1, winner_seat, reason, rated, delta0, delta1, shots, ended_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![players[0], players[1], winner_seat as i64, reason, rated, delta[0], delta[1], shots, now()],
        )?;
        Ok(())
    }

    /// Rated games between the two users in the last `window_secs` seconds.
    pub fn recent_rated_between(&self, a: i64, b: i64, window_secs: i64) -> rusqlite::Result<u32> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM matches WHERE rated = 1 AND ended_at > ?3
             AND ((player0 = ?1 AND player1 = ?2) OR (player0 = ?2 AND player1 = ?1))",
            params![a, b, now() - window_secs],
            |r| r.get(0),
        )
    }

    pub fn leaderboard(&self, limit: u32) -> rusqlite::Result<Vec<LeaderboardEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT username, rating, games, wins FROM users WHERE games > 0
             ORDER BY rating DESC, wins DESC, username ASC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i32>(1)?,
                r.get::<_, u32>(2)?,
                r.get::<_, u32>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for (i, row) in rows.enumerate() {
            let (username, rating, games, wins) = row?;
            out.push(LeaderboardEntry { rank: i as u32 + 1, username, rating, games, wins });
        }
        Ok(out)
    }
}

fn row_to_user(r: &rusqlite::Row<'_>) -> rusqlite::Result<UserRow> {
    Ok(UserRow {
        id: r.get(0)?,
        username: r.get(1)?,
        password_hash: r.get(2)?,
        rating: r.get(3)?,
        games: r.get(4)?,
        wins: r.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn users_tokens_and_leaderboard() {
        let db = Db::open_in_memory().unwrap();
        let a = db.create_user("Alice", "h", 1200).unwrap().unwrap();
        assert!(
            db.create_user("alice", "h", 1200).unwrap().is_none(),
            "names are case-insensitive"
        );
        let b = db.create_user("bob", "h", 1200).unwrap().unwrap();
        db.store_token("t1", a.id).unwrap();
        assert_eq!(db.user_by_token("t1").unwrap().unwrap().username, "Alice");
        db.delete_token("t1").unwrap();
        assert!(db.user_by_token("t1").unwrap().is_none());

        db.update_rating(a.id, Rating { rating: 1220, games: 1 }, true).unwrap();
        db.update_rating(b.id, Rating { rating: 1180, games: 1 }, false).unwrap();
        db.record_match([a.id, b.id], 0, "pocketed_eight", true, [20, -20], 30).unwrap();
        assert_eq!(db.recent_rated_between(b.id, a.id, 3600).unwrap(), 1);
        let lb = db.leaderboard(10).unwrap();
        assert_eq!(lb.len(), 2);
        assert_eq!(lb[0].username, "Alice");
        assert_eq!(lb[0].wins, 1);
        assert_eq!(lb[1].rank, 2);
    }
}
