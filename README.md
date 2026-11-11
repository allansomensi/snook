# Snook

A lightweight 3D eight-ball pool game for the desktop, written in Rust.

Snook is built around what made the old online pool rooms special: a realistic
cue (spin, draw, follow, jump and massé shots), a lounge where you meet people and
chat, invitations, open tables and tournaments, friends, achievements, and a
world ranking that keeps everyone coming back.

| Menu | 3D view |
|---|---|
| ![Menu](docs/screenshots/menu.png) | ![3D view](docs/screenshots/3d-view.png) |

| Lounge | Profile |
|---|---|
| ![Lounge](docs/screenshots/lounge.png) | ![Profile](docs/screenshots/profile.png) |

## Features

- **Real ball physics**: sliding → rolling transition, side spin (English) that
  changes cushion rebounds, draw and follow, squirt, throw, ball-to-ball friction,
  and balls that can leave the slate for **jump shots**.
- **Full cue control**: aim, power, the exact point where the tip hits the cue ball,
  and cue elevation.
- **Eight-ball rules** (WPA-based): break, open table, groups, fouls, ball in hand,
  called pocket for the 8.
- **Computer opponent** with three levels. It plays with the same physics and rules:
  it simulates candidate shots and scores the outcome. Harder levels plan cue-ball
  position, play safeties and aim more accurately.
- **Online lounge**: accounts with a country flag, players online with names
  coloured by rating tier, lounge chat, invitations to **rated** or **casual**
  games, **open tables** (public or **private with a password**),
  **tournaments** (4, 8 or 16 players, public or private, with a live
  bracket), **friends**, blocking and reporting, chat with the opponent
  (with the game's events in the same chat), rematches, a shot clock and
  reconnection after a dropped connection.
- **Instant shots online**: your shot plays the moment you take it (local
  prediction of the deterministic physics), the opponent's as soon as the
  server announces it; the server's result is authoritative.
- **Profiles and progress**: rating and tier, world rank, numbers, match
  history with rating changes, **34 achievements**, and the **world top 100**.
- **Local network**: host a game from the game itself (no server needed);
  others find it automatically or join by address.
- **Replay** of the last shot, a discreet **zoom bar**, and a staff **admin
  panel** on the server.
- **Elo rating** (chess-style) with a higher K-factor for new players. Rated games
  show only a short aiming line; the full ghost-ball guide is for practice and
  casual games.
- **Graphics presets** (Low / Medium / High / Ultra; on the first run the game
  measures the frame rate and lowers the preset if needed). High and Ultra add
  soft shadows from every lamp and contact shading plus anti-aliasing (off, 2×,
  4×, 8×) and full screen, so it runs on old laptops and looks sharp on gaming PCs.
- **Small and fast**: a ~5 MB executable that runs on OpenGL 2.1 / GLES 2, so it works
  on practically any PC, including old integrated GPUs. Textures and sounds are
  generated at start-up; the only bundled files are the DejaVu fonts.

## Controls

Everything can be played with the mouse alone; the keyboard is optional.
The interface is available in English and Brazilian Portuguese (Settings → Game).

| Action | Mouse | Keyboard |
|---|---|---|
| Aim | drag on the table (the cue points at the mouse) or drag the cue stick; click a ball or spot | A / D, ← / → (speeds up while held) |
| Fine aim | Shift + drag | Shift + A / D |
| Quick aim | click a ball or any spot on the cloth | |
| Shoot | set the power on the bar, then press the green button with the cue and ball (bottom right) | hold **Space**, release |
| Fine aim (mouse) | drag the dial in the cue panel, or click ◀ ▶ (one small step per click) | |
| Spin | click/drag on the spin ball | ↑ / ↓ (follow/draw), Q / E (English) |
| Cue elevation | elevation slider | W / S |
| Ball in hand | drag the cue ball | B toggles, Enter confirms |
| Call the 8-ball pocket | click a pocket on the table or on the little map (mandatory: the 8 in any other pocket loses) | |
| Camera | drag with the right button to turn the table (goes 3D; the left button never moves the camera); wheel or the zoom bar to zoom; the camera icon goes back to the top view with no zoom | Tab 3D/top, C reset |
| Replay of the last shot | the replay icon (top left) | R |
| Menu / controls help | **Menu** button | Esc / H |
| Full screen | | F11 |

Settings are saved in `%APPDATA%\Snook\settings.ini` (Windows),
`~/Library/Application Support/Snook` (macOS) or `~/.config/snook` (Linux).

## Building

Requires a recent stable Rust toolchain (see `rust-version` in `Cargo.toml`).

On Linux, install the system libraries for windowing and sound:

```bash
sudo apt-get install libasound2-dev libx11-dev libxi-dev libgl1-mesa-dev
```

Run the game:

```bash
cargo run --release -p snook
```

Run a local server (in memory) and play online against yourself (start the
game twice, with different `HOME` folders so the sessions don't collide):

```bash
SNOOK_IN_MEMORY=1 cargo run --release -p snook_server
```

With PostgreSQL instead (what production uses):

```bash
DATABASE_URL=postgres://user:pass@localhost/snook cargo run --release -p snook_server
```

The game connects to the public server (`wss://snook-server.onrender.com/ws`) by
default; for a local server, enter `ws://127.0.0.1:7878/ws` under Settings → Game
server. To build a release that points at another server, set `SNOOK_SERVER` at
build time:

```bash
SNOOK_SERVER=wss://snook.example.com/ws cargo build --release -p snook
```

Putting the server on the internet is described in [docs/DEPLOY.md](docs/DEPLOY.md)
(step by step for Render + Neon, in Portuguese: [docs/DEPLOY.pt-BR.md](docs/DEPLOY.pt-BR.md)).
The staff panel is described in [docs/ADMIN.md](docs/ADMIN.md), the protocol in
[docs/ONLINE.md](docs/ONLINE.md), and the reviews behind this release in
[docs/SECURITY.md](docs/SECURITY.md) and [docs/UX.md](docs/UX.md).

Useful flags (for development):

```bash
cargo run --release -p snook -- --vs hard        # straight into a game
cargo run --release -p snook -- --demo           # computer vs computer
cargo run --release -p snook -- --window 900x600 # start with this window size
cargo run --release -p snook -- --screenshot shot.png --frames 200
cargo run --release -p snook -- --demo --fps     # print frames per second
cargo run --release -p snook -- --online         # open the lounge directly
```

Run the tests (the PostgreSQL store test runs when
`SNOOK_TEST_DATABASE_URL` points at an empty database):

```bash
cargo test --workspace --release
```

## Project layout

```
crates/
  core/     snook_core: physics, table geometry, 8-ball rules, AI, Elo rating
            (no engine dependencies; shared by the client and the server)
  proto/    snook_proto: the client/server messages (JSON over WebSocket),
            countries, rating tiers and the achievement catalogue
  server/   snook_server: accounts, lounge, tables, tournaments, friends,
            authoritative matches, ratings, achievements, the staff panel
            (Axum; PostgreSQL, or in memory for LAN games and tests)
  client/   snook: the desktop client built on macroquad (3D scene with a
            custom lighting shader, input, HUD, sound, online and LAN screens)
vendor/
  miniquad/ miniquad with two platform fixes (see vendor/README.md)
tools/      gen_countries.py: regenerates the country list and flag atlas
Dockerfile, render.yaml: the server image and the Render blueprint
```

Online games are **authoritative**: the client only sends the stroke (aim, power,
spin, elevation); the server simulates the shot with the same deterministic physics
and sends both players the resulting table, so nobody can cheat by editing their
client.

## Roadmap

1. ~~Physics, rules, AI, rating~~
2. ~~3D client with offline modes~~
3. ~~Game server: accounts, lounge with chat, invitations, authoritative matches~~
4. ~~Rated games and the global leaderboard~~
5. ~~Friends, private tables, tournaments, achievements, profiles, LAN, staff panel~~
6. Signed installers and store releases (Microsoft Store, Steam, itch.io)
7. Spectating games and replays of whole matches

## License

[MIT](LICENSE). The bundled DejaVu fonts keep their own license (below).

## Credits

Fonts: DejaVu Sans (Bitstream Vera license, see https://dejavu-fonts.github.io/License.html).

Flags: [flag-icons](https://github.com/lipis/flag-icons) (MIT, see
`crates/client/assets/flag-icons-LICENSE.txt`). Country names: ISO 3166 data
and translations from the Debian [iso-codes](https://salsa.debian.org/iso-codes-team/iso-codes)
project (via pycountry, LGPL-2.1).

Reactions: [Twemoji](https://github.com/jdecked/twemoji) graphics (CC-BY 4.0, see
`crates/client/assets/twemoji-LICENSE.txt`).
