-- Progress on the daily and weekly missions. `period` is `d<day>` or
-- `w<week>`; `ends` (Unix seconds) is when the period ends, so old rows can
-- be removed.
CREATE TABLE mission_progress (
    user_id  BIGINT  NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    period   TEXT    NOT NULL,
    code     TEXT    NOT NULL,
    progress INTEGER NOT NULL,
    ends     BIGINT  NOT NULL,
    PRIMARY KEY (user_id, period, code)
);
CREATE INDEX mission_progress_ends ON mission_progress (ends);

-- The lounge's notice board: notes the staff pin for everyone.
CREATE TABLE board_notes (
    id         BIGSERIAL PRIMARY KEY,
    text       TEXT   NOT NULL,
    author_id  BIGINT REFERENCES users (id) ON DELETE SET NULL,
    created_at BIGINT NOT NULL,
    -- Taken down at this time (Unix seconds); NULL: until removed.
    expires_at BIGINT,
    -- 'notice', 'event' or 'rule' (the note's colour).
    kind       TEXT    NOT NULL DEFAULT 'notice',
    -- Shown above the others.
    pinned     BOOLEAN NOT NULL DEFAULT FALSE,
    -- A button under the note: 'tournaments', 'shop', 'vip', 'missions'
    -- or NULL.
    action     TEXT
);
