-- Snookered schema, version 1.

CREATE TABLE users (
    id                 BIGSERIAL PRIMARY KEY,
    username           TEXT     NOT NULL,
    password_hash      TEXT     NOT NULL,
    country            TEXT     NOT NULL DEFAULT '',
    role               SMALLINT NOT NULL DEFAULT 0,
    rating             INTEGER  NOT NULL,
    peak_rating        INTEGER  NOT NULL,
    games              INTEGER  NOT NULL DEFAULT 0,
    wins               INTEGER  NOT NULL DEFAULT 0,
    eight_wins         INTEGER  NOT NULL DEFAULT 0,
    casual_games       INTEGER  NOT NULL DEFAULT 0,
    streak             INTEGER  NOT NULL DEFAULT 0,
    best_streak        INTEGER  NOT NULL DEFAULT 0,
    runouts            INTEGER  NOT NULL DEFAULT 0,
    shutouts           INTEGER  NOT NULL DEFAULT 0,
    tournaments_played INTEGER  NOT NULL DEFAULT 0,
    tournaments_won    INTEGER  NOT NULL DEFAULT 0,
    created_at         BIGINT   NOT NULL,
    last_seen          BIGINT   NOT NULL,
    banned_until       BIGINT,
    ban_reason         TEXT,
    muted_until        BIGINT,
    register_ip        TEXT     NOT NULL DEFAULT ''
);
CREATE UNIQUE INDEX users_username_key ON users (lower(username));
CREATE INDEX users_leaderboard ON users (rating DESC, wins DESC) WHERE games > 0;

CREATE TABLE sessions (
    token_hash TEXT     PRIMARY KEY,
    user_id    BIGINT   NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind       SMALLINT NOT NULL,
    created_at BIGINT   NOT NULL,
    expires_at BIGINT   NOT NULL
);
CREATE INDEX sessions_user ON sessions (user_id);
CREATE INDEX sessions_expiry ON sessions (expires_at);

CREATE TABLE matches (
    id            BIGSERIAL PRIMARY KEY,
    player0       BIGINT   NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    player1       BIGINT   NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    winner_seat   SMALLINT NOT NULL,
    reason        TEXT     NOT NULL,
    rated         BOOLEAN  NOT NULL,
    delta0        INTEGER  NOT NULL,
    delta1        INTEGER  NOT NULL,
    rating0       INTEGER  NOT NULL,
    rating1       INTEGER  NOT NULL,
    shots         INTEGER  NOT NULL,
    tournament_id BIGINT,
    ended_at      BIGINT   NOT NULL
);
CREATE INDEX matches_player0 ON matches (player0, ended_at DESC);
CREATE INDEX matches_player1 ON matches (player1, ended_at DESC);
CREATE INDEX matches_ended ON matches (ended_at);

CREATE TABLE achievements (
    user_id     BIGINT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    code        TEXT   NOT NULL,
    unlocked_at BIGINT NOT NULL,
    PRIMARY KEY (user_id, code)
);

-- One row per direction; `accepted` rows exist in both directions.
CREATE TABLE friendships (
    user_id    BIGINT  NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    friend_id  BIGINT  NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    accepted   BOOLEAN NOT NULL,
    created_at BIGINT  NOT NULL,
    PRIMARY KEY (user_id, friend_id)
);
CREATE INDEX friendships_friend ON friendships (friend_id);

CREATE TABLE blocks (
    user_id    BIGINT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    blocked_id BIGINT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, blocked_id)
);

CREATE TABLE reports (
    id          BIGSERIAL PRIMARY KEY,
    reporter_id BIGINT   NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    target_id   BIGINT   NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    reason      TEXT     NOT NULL,
    details     TEXT     NOT NULL,
    context     TEXT     NOT NULL,
    status      SMALLINT NOT NULL DEFAULT 0,
    created_at  BIGINT   NOT NULL,
    resolved_by BIGINT REFERENCES users (id) ON DELETE SET NULL,
    resolved_at BIGINT,
    resolution  TEXT
);
CREATE INDEX reports_status ON reports (status, created_at DESC);
CREATE INDEX reports_target ON reports (target_id, created_at DESC);

CREATE TABLE audit_log (
    id       BIGSERIAL PRIMARY KEY,
    staff_id BIGINT REFERENCES users (id) ON DELETE SET NULL,
    action   TEXT   NOT NULL,
    target   TEXT   NOT NULL,
    details  TEXT   NOT NULL,
    at       BIGINT NOT NULL
);

CREATE TABLE tournaments (
    id          BIGSERIAL PRIMARY KEY,
    name        TEXT     NOT NULL,
    host_id     BIGINT REFERENCES users (id) ON DELETE SET NULL,
    size        INTEGER  NOT NULL,
    rated       BOOLEAN  NOT NULL,
    players     INTEGER  NOT NULL,
    winner_id   BIGINT REFERENCES users (id) ON DELETE SET NULL,
    finished_at BIGINT   NOT NULL
);
