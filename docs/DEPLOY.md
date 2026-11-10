# Running the Snook server

The server is a single executable (`snook-server`, about 5 MB) with an embedded
SQLite database. A small VPS (1 vCPU, 512 MB RAM) is plenty for hundreds of
players.

## 1. Build

On the server (or any Linux x86_64 machine):

```bash
cargo build --release -p snook_server
# result: target/release/snook-server
```

## 2. Configure

Everything is set with environment variables:

| Variable | Default | Meaning |
|---|---|---|
| `SNOOK_BIND` | `0.0.0.0:7878` | address and port to listen on |
| `SNOOK_DB` | `snook.db` | SQLite database file (accounts, ratings, match history) |
| `SNOOK_MOTD` | a welcome line | message shown in the lounge |
| `SNOOK_SHOT_CLOCK` | `30` | seconds per shot (then the shot is played at full power where the player was aiming) |
| `RUST_LOG` | `info` | log level (`warn`, `info`, `debug`) |

Endpoints: `GET /ws` (the game, WebSocket) and `GET /health` (answers `ok`).

## 3. Run it as a service (systemd)

```bash
sudo useradd --system --home /var/lib/snook --create-home snook
sudo cp target/release/snook-server /usr/local/bin/
```

`/etc/systemd/system/snook.service`:

```ini
[Unit]
Description=Snook game server
After=network.target

[Service]
User=snook
WorkingDirectory=/var/lib/snook
Environment=SNOOK_BIND=127.0.0.1:7878
Environment=SNOOK_DB=/var/lib/snook/snook.db
Environment="SNOOK_MOTD=Welcome to Snook! Be kind, play fair."
ExecStart=/usr/local/bin/snook-server
Restart=always
RestartSec=2
NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths=/var/lib/snook

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now snook
journalctl -u snook -f        # logs
```

## 4. HTTPS / WSS with Caddy

Passwords travel inside the WebSocket, so put the server behind TLS. With a
domain pointing at the machine (e.g. `snook.example.com`), Caddy gets a
certificate automatically. `/etc/caddy/Caddyfile`:

```
snook.example.com {
    reverse_proxy 127.0.0.1:7878
}
```

```bash
sudo systemctl reload caddy
curl https://snook.example.com/health   # → ok
```

The game's server address is then `wss://snook.example.com/ws`.

## 5. Point the game at your server

Build the game with your address baked in, so players don't need to type it:

```bash
SNOOK_SERVER=wss://snook.example.com/ws cargo build --release -p snook
```

Players can also change it under **Settings → Game server**.

## 6. Backups

The database is a single file. SQLite's online backup is safe while the server
runs:

```bash
sqlite3 /var/lib/snook/snook.db ".backup '/var/backups/snook-$(date +%F).db'"
```

Add that line to a daily cron job and copy the file off the machine.

## 7. Updating

```bash
sudo cp target/release/snook-server /usr/local/bin/ && sudo systemctl restart snook
```

Players in the lounge reconnect automatically. Games in progress are lost on a
restart, so update when it's quiet. When the message format changes,
`PROTOCOL_VERSION` in `crates/proto` is bumped and older games are asked to
update.

## Rules enforced by the server

- Usernames: 3–16 letters, digits or `_` (case-insensitive, unique). Passwords:
  6–72 characters, stored with Argon2.
- Sessions last 30 days (the game signs in automatically).
- Five wrong passwords close the connection.
- Chat: up to 200 characters, at most one message every 0.7 s; the last 50
  lounge messages are shown to newcomers.
- Rated games: Elo with K = 40 for the first 30 games, 20 after that, 10 above
  2400. At most 5 rated games per pair of players per day (more are casual), so
  ratings can't be farmed.
- Shot clock (default 30 s): when it runs out, the shot is played at full power in the direction the player was aiming.
- A player who disconnects has 60 s to come back; after that the game is lost.
