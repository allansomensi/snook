# Running the Snookered server

The server is a single executable (`snookered-server`) that keeps the lounge and
running matches in memory and stores accounts, ratings, history,
achievements, friends, reports and the staff audit log in PostgreSQL. It also
serves the staff panel at `/admin`.

A step-by-step guide for **Render + Neon (free plans)**, in Portuguese, is in
[DEPLOY.pt-BR.md](DEPLOY.pt-BR.md). This page is the reference.

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `PORT` / `SNOOKERED_BIND` | `0.0.0.0:7878` | port to listen on (`PORT`, as set by hosts like Render) or a full address |
| `DATABASE_URL` | — | PostgreSQL connection string (required unless `SNOOKERED_IN_MEMORY=1`); migrations run at start |
| `SNOOKERED_DB_POOL` | `5` | database connections |
| `SNOOKERED_IN_MEMORY` | — | `1`: keep everything in memory (development only) |
| `SNOOKERED_ADMINS` | — | comma-separated usernames that become administrators when they sign in |
| `SNOOKERED_CLIENT_IP_HEADER` | — | headers with the client's real address behind a proxy, tried in order, e.g. `cf-connecting-ip,true-client-ip,x-forwarded-for` |
| `SNOOKERED_PROXY_HOPS` | `1` | with `x-forwarded-for`: proxies that append to it (the address is taken that many entries from the right) |
| `SNOOKERED_MOTD` | a welcome line | message shown in the lounge |
| `SNOOKERED_SHOT_CLOCK` | `30` | seconds per shot (10–120) |
| `RUST_LOG` | `info` | log filter |

Endpoints: `GET /ws` (the game), `GET /health` (`ok`), `GET /api/status` and
`GET /api/leaderboard` (public JSON with CORS, for a website), `/admin`.

## Docker

```bash
docker build -t snookered-server .
```

```bash
docker run -p 7878:7878 -e DATABASE_URL=postgres://user:pass@host/db snookered-server
```

The image is distroless (no shell, unprivileged user, ~50 MB). The Server
image workflow publishes it as `ghcr.io/<owner>/snookered-server:<tag>` when a
release tag is pushed (see [RELEASING.md](RELEASING.md)).

## Without Docker (a VPS with systemd)

```bash
cargo build --release -p snookered_server
```

```bash
sudo cp target/release/snookered-server /usr/local/bin/
```

`/etc/systemd/system/snookered.service`:

```ini
[Unit]
Description=Snookered game server
After=network.target postgresql.service

[Service]
DynamicUser=yes
Environment=SNOOKERED_BIND=127.0.0.1:7878
Environment=DATABASE_URL=postgres://snookered:secret@localhost/snookered
Environment=SNOOKERED_CLIENT_IP_HEADER=x-forwarded-for
ExecStart=/usr/local/bin/snookered-server
Restart=always
RestartSec=2
NoNewPrivileges=true
ProtectSystem=strict

[Install]
WantedBy=multi-user.target
```

Put it behind TLS (Caddy: `snookered.example.com { reverse_proxy 127.0.0.1:7878 }`);
the game's address is then `wss://snookered.example.com/ws`.

## Point the game at the server

```bash
SNOOKERED_SERVER=wss://snookered.example.com/ws cargo build --release -p snookered
```

Players can also change it under **Settings → Game → Game server**.

## Updating

A restart keeps every account and result; players reconnect by themselves,
but games in progress are lost, so update when it's quiet. When messages
change incompatibly, `PROTOCOL_VERSION` in `crates/proto` is bumped and older
games are told to update.

## Rules enforced by the server

- Usernames: 3–16 letters, digits or `_` (case-insensitive, unique, a few
  reserved). Passwords: 8–72 characters, not only digits; Argon2id.
- Sessions: random 256-bit tokens, stored hashed (SHA-256); valid 30 days,
  renewed when used after a day; staff panel sessions last 8 hours.
- Sign-in throttling: 8 failures per account or 20 per address in 15 minutes;
  5 new accounts per address per hour; 5 bad sign-ins close the connection.
- Connections: 12 per address; 40 messages a second per connection (bursts of
  120) or the connection is closed; 16 KiB per message; outgoing queues are
  bounded (a client that stops reading is disconnected).
- Chat: 200 characters, control and invisible characters removed, about one
  message a second with bursts of 5, no repeating the same line; blocked
  players' messages aren't delivered; muted players can't chat.
- Rated games: Elo with K = 24 for the first 10 games, 14 after that, 8 from
  1900.
- Shot clock (default 30 s): when it runs out the shot is played at full
  power where the player was aiming.
- A player who disconnects has 60 s to come back; after that the game is lost.
- Every shot is validated and simulated by the server; clients only send the
  stroke.
