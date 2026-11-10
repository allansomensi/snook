# How online play works

## Pieces

- **`snook_proto`**: every message between the game and the server, as JSON
  objects with a `type` field, one per WebSocket text frame.
- **`snook_server`**: one process holds the lounge in memory (who's online, chat
  history, invitations, running matches) and stores accounts, ratings and match
  history in SQLite.
- **The game** (`crates/client/src/net.rs`) keeps one WebSocket open on a
  background thread, pings every 20 s and reconnects by itself after a drop.

## A session

1. `hello { version }` — the server refuses outdated games.
2. `register`, `login` or `resume { token }` → `auth_ok { player, token }`. The
   game saves the token so the next start signs in automatically.
3. The server sends `welcome`, `lounge` (players online) and `chat_history`, then
   keeps the lounge up to date with `player_updated` / `player_left` / `chat`.
4. `invite { to, rated }` → the other player gets `invited`; `answer_invite`
   starts the match with `match_started` for both.

## A match

- The player at the table streams `aim` (≈10 per second) and `place_cue_ball` so
  the opponent sees the cue move.
- `shoot { shot, called_pocket, cue_ball }` sends only the stroke. The server
  checks it, simulates it with `snook_core` and sends both players
  `shot_played { start, end, report }`. Clients animate from `start` and then
  adopt `end`, so both tables are always identical to the server's.
- The shot clock runs on the server (`time_foul` gives ball in hand).
- `resign`, an abandoned game or the last ball ends the match with
  `match_ended { result }`, including rating changes. Then `rematch` (both
  players) or `leave_match`.
- If a player drops, the opponent gets `opponent_connection`; reconnecting
  within the grace period resumes the match with a fresh `match_started`.

Rated games use Elo (`snook_core::rating`). See `docs/DEPLOY.md` for limits and
how to run the server.
