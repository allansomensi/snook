# Changelog

## Unreleased

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

### Fixes
- Hover hints (tooltips, achievement cards, "Shoot!", the power hint) are
  drawn after everything else, so panels, buttons, chat and toasts drawn
  later no longer cover them ("Find an opponent…" used to hide under the
  rated/casual switch, achievement cards under the match history).
- A release built with an empty `SNOOK_SERVER` (the CI variable not set) no
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
