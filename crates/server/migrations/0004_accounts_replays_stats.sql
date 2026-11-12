-- Accounts: a recovery e-mail, deletion by anonymising, abandoned games.
ALTER TABLE users
    ADD COLUMN recovery_email TEXT,
    ADD COLUMN deleted_at     BIGINT,
    ADD COLUMN abandons       INTEGER NOT NULL DEFAULT 0;
CREATE INDEX users_recovery_email ON users (lower(recovery_email))
    WHERE recovery_email IS NOT NULL;

-- The e-mail the payment provider collected (offered as a recovery e-mail).
ALTER TABLE payments ADD COLUMN email TEXT;

-- Coins earned by playing: the daily bonus and a daily cap.
ALTER TABLE wallets
    ADD COLUMN daily_day    BIGINT  NOT NULL DEFAULT 0,
    ADD COLUMN earned_day   BIGINT  NOT NULL DEFAULT 0,
    ADD COLUMN earned_today INTEGER NOT NULL DEFAULT 0;

-- A deleted account is anonymised, never removed: deleting a user must not
-- take their opponents' history with it.
ALTER TABLE matches
    DROP CONSTRAINT matches_player0_fkey,
    DROP CONSTRAINT matches_player1_fkey;
ALTER TABLE matches
    ADD CONSTRAINT matches_player0_fkey FOREIGN KEY (player0) REFERENCES users (id) ON DELETE RESTRICT,
    ADD CONSTRAINT matches_player1_fkey FOREIGN KEY (player1) REFERENCES users (id) ON DELETE RESTRICT;

-- The whole game: the physics is deterministic, so a seed, who broke and the
-- strokes rebuild every shot (replays, reports, restarts).
ALTER TABLE matches
    ADD COLUMN seed    BIGINT,
    ADD COLUMN breaker SMALLINT,
    ADD COLUMN strokes JSONB;
CREATE INDEX matches_rated_ended ON matches (ended_at) WHERE rated;

-- End-of-game numbers, per player.
CREATE TABLE match_stats (
    match_id      BIGINT   NOT NULL REFERENCES matches (id) ON DELETE CASCADE,
    user_id       BIGINT   NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    shots         INTEGER  NOT NULL,
    potting_shots INTEGER  NOT NULL,
    balls         INTEGER  NOT NULL,
    fouls         INTEGER  NOT NULL,
    best_run      INTEGER  NOT NULL,
    in_hand       INTEGER  NOT NULL,
    PRIMARY KEY (match_id, user_id)
);
CREATE INDEX match_stats_user ON match_stats (user_id);

-- Games in progress, saved after every stroke: a restarted server rebuilds
-- them and the players reconnect.
CREATE TABLE live_matches (
    id         BIGINT   PRIMARY KEY,
    player0    BIGINT   NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    player1    BIGINT   NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    rated      BOOLEAN  NOT NULL,
    seed       BIGINT   NOT NULL,
    breaker    SMALLINT NOT NULL,
    strokes    JSONB    NOT NULL,
    updated_at BIGINT   NOT NULL
);

-- Potting by difficulty band (cut angle × distance), for the aim-assistance
-- review queue in the staff panel.
CREATE TABLE shot_stats (
    user_id  BIGINT   NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    band     SMALLINT NOT NULL,
    attempts INTEGER  NOT NULL DEFAULT 0,
    made     INTEGER  NOT NULL DEFAULT 0,
    PRIMARY KEY (user_id, band)
);

-- Tournament entry fees and prizes (coins), and who received what.
ALTER TABLE tournaments
    ADD COLUMN entry_fee INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN prizes    TEXT    NOT NULL DEFAULT '';

-- Who played on which day (Unix day number): active players and retention
-- for the staff panel.
CREATE TABLE activity (
    day     BIGINT NOT NULL,
    user_id BIGINT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    PRIMARY KEY (day, user_id)
);
CREATE INDEX activity_user ON activity (user_id, day);
