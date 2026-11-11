# How online play works

## Pieces

- **`snook_proto`**: every message between the game and the server (JSON
  objects with a `type` field, one per WebSocket text frame), plus what both
  sides must agree on: name rules, countries, rating tiers, achievements.
- **`snook_server`**: the lounge, open tables, match rooms and tournaments
  live in memory behind one mutex (short, synchronous methods); everything
  slow runs outside it: the database (PostgreSQL, or memory for LAN games and
  tests) and shot simulation (a blocking task per shot).
- **The game** (`crates/client/src/net.rs`) keeps one WebSocket on a
  background thread (4 ms read slices, `TCP_NODELAY`), pings every 15 s and
  reconnects by itself. With a saved session it signs in as soon as the game
  starts, so the menu shows the player and the lounge opens instantly.

## A session

1. `hello { version }`: older games are told to update.
2. `register { username, password, country }`, `login`, `resume { token }`
   or, on a LAN server, `guest { name }` → `auth_ok { player, token }`.
   `leaderboard` and `profile` also work before signing in.
3. The server sends `welcome`, `lounge`, `chat_history`, `tables` and
   `tournaments`, then keeps them current (`player_updated`, `chat`,
   `table_opened`, `tournament_updated`…).
4. Games start from an invitation (`invite` → `answer_invite`), by sitting at
   an open table (`open_table { rated, password }` → `join_table`), or by a
   tournament pairing, all ending in `match_started`.

## A shot, without waiting for the network

```
shooter                     server                         opponent
  │ shoot{turn, stroke}        │                               │
  │──────────────────────────▶ │ validate (turn, cue ball…)    │
  │ (animates at once:         │── shot_taken ────────────────▶│ animates at once
  │  local prediction)         │ simulate (blocking task)      │
  │◀──────── shot_result{end} ─│── shot_result{end} ──────────▶│
  │ adopts `end` when the      │                               │ adopts `end`
  │ balls stop                 │                               │
```

- The physics is deterministic, so the prediction normally matches the
  server exactly (the end-to-end tests check it); the server's table is
  adopted at the end either way.
- A refused shot (`shot_rejected { turn, reason, game }`) rolls the table
  back. A `shot_taken` that isn't the predicted one (the clock ran out at the
  same moment) replaces the prediction.
- `match_ended` that arrives while a shot is still on screen is applied when
  the balls stop (the deciding shot is always shown, and a win on the 8-ball
  is never mistaken for a resignation).
- While aiming, `aim` is streamed ~20 times a second (relayed at most every
  33 ms); the opponent's cue glides towards the latest aim.

## Matches

- The shot clock runs on the server (and keeps running if the shooter
  disconnects); when it runs out the server plays a full-power shot where the
  player was aiming.
- `resign`, a disconnection longer than the grace period or the last ball end
  the match with `match_ended { result }`, ratings included. Then `rematch`
  (both players; not in tournaments) or `leave_match`.
- Reconnecting within the grace period resumes the match with a fresh
  `match_started`.
- Ratings: Elo with K 24 for the first 10 rated games, 14 after, 8 from
  1900 (`crates/core/src/rating.rs`). Feats (8-ball wins, runouts,
  shutouts) count in rated games only.
- `spectate { username }` watches a friend's games (anyone's on a local
  network): `match_started { spectating: true, seat }` with `seat` the
  friend's, then every shot, aim and result of that table, and the next game
  too (rematch or a new opponent) until `stop_spectating` or
  `spectate_ended` (the friend went back to the lounge). The players' chat
  is not sent to spectators. Everyone at the table gets
  `spectators { count }`. A spectator's `react` arrives as
  `reaction { seat: 2 }`, through a limit shared by the whole crowd.
- The shop: `shop` answers `shop_info` (what is sold for money) and
  `wallet` (coins, VIP time, owned and equipped items; also pushed after
  signing in and on every change). `buy_item { code }` and `equip { code }`
  are checked against `crates/proto/src/shop.rs`; `buy_coins { pack }`,
  `subscribe_vip` and `manage_vip` answer `payment_page { url }` (Stripe).
  Payments arrive through `POST /stripe/webhook` (see
  `crates/server/src/shop.rs`).
- `queue_join { rated }` / `queue_leave`: "Play now"; the server sends
  `queue { searching, waited_secs, range }` and then `match_started`.
- `react { code }` sends a reaction to the table (`reaction { seat, code }`
  to both players). Codes and what unlocks each are in
  `crates/proto/src/reactions.rs`; the server drops locked ones and allows a
  few a second. After signing in (and whenever one is unlocked) the server
  sends `my_achievements { codes }`, which the game uses to offer them.

## Tournaments

Single elimination for 4, 8 or 16 players, seeded by rating with byes for empty
seats. It starts when full (or when the host starts it). Matches start by
themselves when both players are free; a player who is offline for two
minutes when their match is ready loses it. Players still in a running
tournament can't start other games.

Rated games use Elo (`snook_core::rating`). See `docs/DEPLOY.md` for limits.
