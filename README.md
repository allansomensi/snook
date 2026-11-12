<p align="center"><img src="crates/client/assets/icon/snook-256.png" width="128" alt="Snookered"></p>

# Snookered

A lightweight 3D eight-ball pool game for the desktop, written in Rust.

Snookered is built around what made the old online pool rooms special: a realistic
cue (spin, draw, follow, jump and massé shots), a lounge where you meet people and
chat, invitations, open tables and tournaments, friends, achievements, and a
world ranking that keeps everyone coming back.

| Menu | 3D view |
|---|---|
| ![Menu](docs/screenshots/menu.png) | ![3D view](docs/screenshots/3d-view.png) |

| Lounge | Profile |
|---|---|
| ![Lounge](docs/screenshots/lounge.png) | ![Profile](docs/screenshots/profile.png) |

## Download

Get the latest version from the
[Releases page](https://github.com/allansomensi/snook/releases/latest):

- **Windows**: `snook-x86_64-pc-windows-msvc.msi` installs the game with a
  Start menu (and optional desktop) shortcut; the `.zip` is a portable copy.
- **macOS** (Apple Silicon or Intel) and **Linux** (x64 or ARM64): the
  `.tar.xz` for your computer, or install from a terminal:

  ```bash
  curl --proto '=https' --tlsv1.2 -LsSf https://github.com/allansomensi/snook/releases/latest/download/snook-installer.sh | sh
  ```

- **Windows from PowerShell**:

  ```powershell
  powershell -ExecutionPolicy Bypass -c "irm https://github.com/allansomensi/snook/releases/latest/download/snook-installer.ps1 | iex"
  ```

On Linux the game needs the usual desktop libraries (ALSA, X11, OpenGL),
which every desktop installation has. How releases are made is described in
[docs/RELEASING.md](docs/RELEASING.md).

## Features

- **Real ball physics**: sliding → rolling transition, side spin (English) that
  changes cushion rebounds, draw and follow, squirt, throw, ball-to-ball friction,
  and balls that can leave the slate for **jump shots**. Contacts are found
  continuously (a 0.02 mm cut at full power still counts), and the simulation
  gives the **same bits on Windows, macOS and Linux**, so the server and the
  players always agree (pinned by a fingerprint test in CI).
- **Full cue control**: aim, power, the exact point where the tip hits the cue ball,
  cue elevation, and fine aim right on the bottom bar.
- **Eight-ball rules** (WPA): break, open table (the break never decides the
  groups), groups, fouls (including "no cushion after contact"), ball in
  hand, a called pocket for the 8 that is binding once made. Every rule has
  its own test, and 60 random games are checked after every shot.
- **Computer opponent** with three levels. It plays with the same physics and rules:
  it simulates candidate shots and scores the outcome. Harder levels plan cue-ball
  position, play safeties and aim more accurately. Games against it never
  touch ratings, achievements or the shop.
- **Online lounge**: accounts with a country flag, players online with names
  coloured by rating tier, lounge chat, **Play now** (a queue that pairs
  players by rating, widening with time), invitations to **rated** or
  **casual** games, **open tables** (public or **private with a password**),
  **tournaments** (4, 8 or 16 players, public or private, with a live
  bracket), **friends** (with "last seen"), blocking and reporting, chat with
  the opponent (with the game's events in the same chat), rematches, a shot
  clock and reconnection after a dropped connection.
- **Watching friends**: an eye next to a friend who is playing opens their
  game; players see how many are watching. Spectators follow into the
  rematch and can send small, faint reactions.
- **Reactions** during a match: 4 for everyone, 16 more unlocked by hard
  achievements and events (Halloween, the holidays, New Year).
- **Instant shots online**: your shot plays the moment you take it (local
  prediction of the deterministic physics), the opponent's as soon as the
  server announces it; the server's result is authoritative.
- **Profiles and progress**: rating, tier and how far the next one is, world
  rank, numbers, match history with rating changes, **37 achievements**, the
  reactions collected and the **world top 100**. Ratings move slowly (Elo,
  K 24 for the first 10 games, then 14, 8 from 1900).
- **End-of-game numbers**: potting success, balls made, best run, fouls and
  how often each player had ball in hand.
- **The shop**: cloth colours, cues, chalk and ball sets, shown on the table
  (opponents see your cue and chalk). Some are free, some come with
  achievements, some cost coins, some are for **VIP** members. Coins and the
  monthly VIP are paid through Stripe (card, Pix, boleto) on its hosted page;
  the server keeps the only wallet and credits every payment exactly once.
- **Local network**: host a game from the game itself (no server needed);
  others find it automatically or join by address.
- **Replay** of the last shot, a discreet **zoom bar**, a read-only game
  history offline, and a staff **admin panel** on the server (moderation,
  wallets, payments, audit log).
- **Aiming guides**: the full guide (where both balls go) is for offline
  practice; against people, the standard guide (ghost ball) or a short line.
- **A cosy pool hall** around the table (cue racks, scoreboard, posters, wall
  lights), or a dark stage with only the table.
- **Graphics presets** (Low / Medium / High / Ultra; on the first run the game
  measures the frame rate and lowers the preset if needed). High and Ultra add
  soft shadows from every lamp and contact shading plus anti-aliasing (off, 2×,
  4×, 8×) and full screen, so it runs on old laptops and looks sharp on gaming PCs.
- **Small and fast**: a ~5 MB executable that runs on OpenGL 2.1 / GLES 2, so it works
  on practically any PC, including old integrated GPUs. Textures and sounds
  (modal synthesis) are generated at start-up; the only bundled files are the
  fonts, the flag atlas and the reaction pictures. The interface scales down
  to small windows without anything spilling out.

## Controls

Everything can be played with the mouse alone; the keyboard is optional.
The interface is available in English and Brazilian Portuguese (Settings → Game).

| Action | Mouse | Keyboard |
|---|---|---|
| Aim | drag on the table (the cue points at the mouse) or drag the cue stick; click a ball or spot | A / D, ← / → (speeds up while held) |
| Fine aim | the arrows on the bottom bar (one small step per click) or drag the dial between them; Shift + drag on the table | Shift + A / D |
| Quick aim | click a ball or any spot on the cloth | |
| Shoot | set the power on the bar, then press the green button with the cue and ball (bottom right) | hold **Space**, release |
| Spin | click/drag on the spin ball | ↑ / ↓ (follow/draw), Q / E (English) |
| Cue elevation | elevation slider | W / S |
| Ball in hand | drag the cue ball (any time while it is in hand; aiming and power work too) | B toggles, Enter confirms |
| Call the 8-ball pocket | click a pocket on the table (they glow when a call is needed; binding once made) | |
| Camera | drag with the right button to turn the table (goes 3D; the left button never moves the camera); wheel or the zoom bar to zoom; the camera icon goes back to the top view with no zoom | Tab 3D/top, C reset |
| Replay of the last shot | the replay icon (top left) | R |
| Menu / list of controls | **Menu** button (the controls are listed there, never on the table) | Esc / H |
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
(step by step for Render + Neon, in Portuguese: [docs/DEPLOY.pt-BR.md](docs/DEPLOY.pt-BR.md),
including the Stripe setup for the shop in section 9).
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
cargo run --release -p snook -- --screen shop    # or lan, setup, ranking, profile:<name>, …
```

Development builds also read a few switches from the environment to drive
automated matches and screenshots (`SNOOK_AUTOPLAY`, `SNOOK_AUTOINVITE`,
`SNOOK_PROFILE`…, see `crates/client/src/dev.rs`); release builds ignore them.

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
            countries, rating tiers, achievements, reactions and the shop
            catalogue
  server/   snook_server: accounts, lounge, Play now, tables, tournaments,
            friends, spectators, authoritative matches, ratings,
            achievements, wallets and Stripe payments, the staff panel
            (Axum; PostgreSQL, or in memory for LAN games and tests)
  client/   snook: the desktop client built on macroquad (3D scene with a
            custom lighting shader, input, HUD, sound, online and LAN screens);
            assets/icon/ holds the emblem's files (made by src/icon.rs) and
            wix/ the Windows installer
vendor/
  miniquad/ miniquad with two platform fixes (see vendor/README.md)
tools/      gen_countries.py, gen_reactions.py: regenerate the country list,
            the flag atlas and the reaction atlas
Dockerfile, render.yaml: the server image and the Render blueprint
dist-workspace.toml: packaging and releases with dist (docs/RELEASING.md)
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
6. ~~Spectating friends, Play now, reactions, the shop and VIP~~
7. ~~Installers and packages for every platform (MSI, shell, PowerShell; dist)~~
8. Signed installers and store releases (Microsoft Store, Steam, itch.io)
9. Replays of whole matches

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
