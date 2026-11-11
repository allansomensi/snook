-- The shop: coins, owned and equipped cosmetics, VIP and payments.

CREATE TABLE wallets (
    user_id         BIGINT PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    coins           BIGINT NOT NULL DEFAULT 0,
    -- VIP until this time (Unix seconds); 0 = never was.
    vip_until       BIGINT NOT NULL DEFAULT 0,
    stripe_customer TEXT UNIQUE
);

CREATE TABLE owned_items (
    user_id     BIGINT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    code        TEXT   NOT NULL,
    acquired_at BIGINT NOT NULL,
    source      TEXT   NOT NULL,
    PRIMARY KEY (user_id, code)
);

CREATE TABLE equipped (
    user_id  BIGINT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    category TEXT   NOT NULL,
    code     TEXT   NOT NULL,
    PRIMARY KEY (user_id, category)
);

-- Every change to a balance. `reference` makes credits from payments
-- idempotent (a provider may deliver the same event twice).
CREATE TABLE coin_ledger (
    id        BIGSERIAL PRIMARY KEY,
    user_id   BIGINT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    delta     BIGINT NOT NULL,
    reason    TEXT   NOT NULL,
    reference TEXT UNIQUE,
    at        BIGINT NOT NULL
);
CREATE INDEX coin_ledger_user ON coin_ledger (user_id, at DESC);

-- Payments received (the payment intent or invoice id from the provider).
CREATE TABLE payments (
    id       TEXT PRIMARY KEY,
    user_id  BIGINT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind     TEXT   NOT NULL,
    coins    BIGINT NOT NULL,
    amount   BIGINT NOT NULL,
    currency TEXT   NOT NULL,
    at       BIGINT NOT NULL,
    refunded BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE INDEX payments_user ON payments (user_id, at DESC);
