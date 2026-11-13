# Changelog

## Unreleased

### Game
- Administrators' names are red everywhere (lounge, chat, match, ranking,
  profiles, game history), and theirs alone: no rating tier or VIP name
  style is red (Ember is now orange). Profiles show an "Admin" tag.
- A click counts only when pressed and released on the same button: dragging
  the power bar and letting go over the shoot button no longer shoots (the
  same for every button and the reactions).
- Profile: with the account, report or country dialog open, the profile
  under it no longer reacts to the pointer (achievement cards showed through).
- Deleting the account is on a page of its own, behind a small link in the
  account dialog, and asks for the password and the username typed again.
- Games on this computer start with the signed-in player's username as
  their name.

### Distribution
- The Store package indexes its icons (resources.pri) and has "unplated"
  small icons: Windows drew the emblem on a plate in the accent colour (blue
  corners in the taskbar and Start menu).

### Server
- Chat lines, the ranking and game history carry the player's role.
- The server stays on a paid Render plan: the keep-awake workflow is gone.

## 0.1.2 (protocol 11)

### The game is called Snookered
- New name everywhere: window, menus, lounge, notifications, the installer
  and its shortcuts, the server's messages and pages. Installing 0.1.2 over
  0.1.1 replaces it (now in `Program Files\Snookered`); the settings moved to
  a new folder, so sign in again once.

### Distribution
- The Microsoft Store version: an MSIX package built by the *Microsoft Store*
  workflow from release tags and sent to Partner Center as a draft
  submission (`docs/MSSTORE.md`). It sells nothing through the game's own
  payment pages (coins come from playing), has the C runtime built in, and
  opens the local network ports in the firewall.
- Pictures for the Store pages: screenshots in three languages
  (`store-screenshots/`) and the poster, box art, tile icons and hero art
  (`store-art/`, made by `tools/store_art.py`).

### Server
- The privacy policy, in Portuguese (`/privacidade`) and English
  (`/privacy`); `SNOOKERED_GAME_NAME`, `SNOOKERED_PRIVACY_CONTROLLER` and
  `SNOOKERED_PRIVACY_CONTACT` fill in its names.
- Environment variables are now `SNOOKERED_*`.
- A test keeps migrations already applied in production unchanged (the
  server refuses to start when one is edited).

### Fixes
- Lounge: the featured game's "watching · average" line ran into a long
  label ("EN LA TELE" in Spanish).

## 0.1.1 (protocol 11)

Everything in the 0.1.1 betas, and:

### Fixes
- Tournaments: a player who withdrew from a knockout while waiting for their
  next opponent stayed in it (held from other games, then pulled into the
  match). They now lose that match by walkover and are free at once.
- The ranking left out for good anyone whose temporary ban had ended.
- Staff panel sign-in: spaces around the name got around the per-account
  attempt limit; unknown names now take as long as wrong passwords.
- A free VIP month was recorded with the month's coins, which a reversal
  would have taken back without them ever being given.
- The settings file (it holds the session token) is created private from
  the start.
- Translations: texts starting with `#` were read as comments ("#3 in the
  world" stayed in English on the profile).
- Shop: a VIP perk that wrapped onto a third line overlapped the next one;
  long category names ran into their item count.
- Profile: the replay button covered the date of each game.

## 0.1.1-beta.4 (protocol 11)

### Game
- A detailed pool hall (panelling, damask paper, moldings, plank floor, rug,
  tin ceiling, bar with bottles and stools, racks, chalkboard, dartboard,
  posters, jukebox, neon) with baked lighting; turned table legs, rounded
  rail edge, apron lip; finer cloth on Ultra.
- No more freeze when cloth or balls change: textures are made on another
  thread and uploaded a few per frame.
- Shop: 98 items in 7 categories (new: frames, VIP name colours, winning
  effects), a clear confirmation (price, coins before/after), a success
  card, a rotating 3D preview on the table, gifts to friends.
- VIP crown in the ranking and profiles; profile rating chart, career
  numbers, abandon rate, replays, account dialog (recovery e-mail,
  password, deletion).
- Lounge: featured game ("on TV"), restart notice, invitation codes,
  tournaments with entry fees and the Arena format; system notifications.
- Translations moved to `lang/*.txt`; Spanish added.

### Server
- Accounts: change password, recovery e-mail (offered from checkout),
  deletion by anonymising (LGPD); matches no longer cascade on delete.
- Graceful drain on SIGTERM (`SNOOKERED_DRAIN_SECS`, `maxShutdownDelaySeconds`);
  games are saved after every stroke and continue on the next server.
- Replays (seed + strokes), per-match statistics, rating history.
- Coins for rated games (daily cap), daily bonus, anti-farm for repeated
  pairs; tournament fees and prizes (70/20, 10% house); weekly cup
  (`SNOOKERED_CUP`); VIP 16-player brackets and waiting-list priority.
- Staff panel: metrics (DAU, games, retention, funnel, revenue, queue
  wait), aim-assistance review queue, replays attached to reports.

## 0.1.1-beta.3

### Distribution
- Releases are built and published by dist: a Windows installer (`.msi`,
  with Start menu and desktop shortcuts and an uninstaller), portable
  `.zip`/`.tar.xz` packages for Windows, macOS (Apple Silicon and Intel) and
  Linux (x64 and ARM64), and one-line installers for the terminal (shell and
  PowerShell). The server image has its own workflow.
- A logo: a pool table seen from above with an "S" ball. It is the window
  icon, the executable's icon (with version details on Windows), the
  installer's icon and pictures, and it appears on the menu, in the lounge
  and while the game loads.

### Graphics
- The balls' numbers are drawn with the game's bold font, anti-aliased (a
  5x7 bitmap font looked blocky up close); 6 and 9 are underlined, as on
  real balls; the edges of the number circle and the stripes are smooth.
- The cue casts a soft shadow on the cloth and the balls from every lamp
  (High and Ultra).
- The leather around the pockets was lit from below (its faces pointed
  down) and looked dark; the pocket walls were lit from the outside.
- A development view (`SNOOKERED_DEBUG_NORMALS=1`, debug builds) paints red any
  face lit from behind; tests check the shapes the table is made of.

### Fixes
- Buying coins or VIP on Windows opened Stripe's "This link is incomplete"
  page: the game opened the address with Explorer, which drops everything
  after `#`, and Stripe needs that part. It is now opened with
  ShellExecute, which passes the whole address.
- Text never runs past the edge of the panel it is in: it is made a little
  smaller and, if that is not enough, cut with "…" ("Computador (Difícil)
  vence!" used to overflow the game-over panel).
- Installing a newer beta over an older one with the .msi kept the old
  game: Windows Installer ignores the fourth part of a version (the beta
  number), so it saw the same version and left the old snookered.exe, whose
  file version was also the same. Newer builds now replace older ones of the
  same version, and snookered.exe carries the beta number in its version.
- The bottom and right cushions were darker than the others: their faces
  were wound the other way, so the light hit them from behind.
- The side pockets showed a black trapezoid between the jaws: the cloth
  was cut away there, opening onto nothing. It now runs on to the hole and
  rolls over its edge, as in the corner pockets.
- Hover hints (tooltips, achievement cards, "Shoot!", the power hint) are
  drawn after everything else, so panels, buttons, chat and toasts drawn
  later no longer cover them ("Find an opponent…" used to hide under the
  rated/casual switch, achievement cards under the match history).
- A release built with an empty `SNOOKERED_SERVER` (the CI variable not set) no
  longer ships without a game server: it keeps the public one.

## 0.1.0

First public version.

### Play
- Eight-ball with real ball physics, a computer opponent (three levels), two
  players on one computer, and games on the local network (host from the
  game; others find it automatically or join by address).
- Offline setup screens: names, the computer's level, the aiming guide and an
  optional shot clock.
- Replay of the last shot, a slim zoom bar, the top view as one icon, and
  right-dragging to turn the view.
- Text fields: arrow keys, Home/End, Delete, Ctrl+arrows, Ctrl+A, click to
  place the caret, keys repeat when held.

### Online
- The shop: cloth colours, cues, chalk and ball sets. Some are free, some
  come with achievements, some are bought with coins, some are VIP. Coins
  and the monthly VIP are paid through Stripe (card, Pix, boleto) on its
  hosted page; the server keeps the only wallet, credits each payment once,
  takes refunds back, and logs every staff change. Players see each other's
  cue and chalk, and a crown by VIP names.
- Play now: a queue that pairs players by rating, widening with time.
- End-of-game numbers: potting success, balls made, best run, fouls, ball
  in hand.
- Friends see when the other was last online.
- Watch a friend's game: an eye next to them in Friends (or in the players
  list) opens it; the players see an eye with how many are watching.
  Spectators follow the friend into the rematch, can't read the players'
  chat and can send reactions, which reach the table small and faint.
- Reactions during a match: 4 for everyone; the other 16 come from hard
  rated-game achievements (100 games, 50 wins, 10 in a row, rating 2100…)
  and from winning a rated game during an event (Halloween, the holidays,
  New Year). They pop up discreetly by the sender's card.
- Progress is earned slowly: smaller rating changes (K 24 for the first 10
  games, then 14, 8 from 1900), everyone starts as a Rookie, feats (8-ball
  wins, runouts, shutouts) count only in rated games. Games against the
  computer never
  touch ratings, numbers, achievements or reactions.
- The 8-ball pocket is called by clicking a pocket on the table (no panel),
  and the call is binding for the rest of the game.
- With the chat folded, new messages show a pulsing dot and a short preview.
- No full aiming guide against people: the standard one (ghost ball) or the
  short line, in rated and casual games alike.
- The game points at the public server out of the box.
- Shots play instantly for both players (the shooter's is predicted locally;
  the server's result is authoritative).
- Fixed: a game won on the 8-ball was shown to the loser as a resignation.
- Accounts with a country flag; names coloured by rating tier; profiles with
  numbers, achievements (37, three of them events), the reactions unlocked
  and progress towards the next tier and match history; the world top 100.
- Open tables (public or private with a password), invitations, tournaments
  (4, 8 or 16 players, public or private), friends, blocking and reporting.
- The game's events appear in the match chat, quieter than what people write.
- The game signs in in the background; the menu shows who is signed in.

- After a foul on the break the cue ball always goes back behind the head
  string (it used to stay where it stopped, and no shot was allowed until
  the player moved it).
- Esc while charging a shot only cancels the shot (it used to open the menu
  as well).

### Server
- Before launch: limits on lounge actions and database requests, no ghost
  tables or queue places after signing in elsewhere, tournaments abandoned
  by their host are removed, VIP refunds and first payments handled in any
  order of events. See `docs/SECURITY.md` (14–20).
- PostgreSQL storage (Neon-ready) with migrations; in memory for LAN and tests.
- Hardened: hashed and renewed sessions, sign-in and sign-up throttling,
  flood and connection limits, bounded queues, simulation off the main lock,
  bans and mutes. See `docs/SECURITY.md`.
- Staff panel at `/admin` with an audit log. See `docs/ADMIN.md`.
- Docker image, Render blueprint, CI (PostgreSQL, image, dependency audit)
  and a release workflow.
