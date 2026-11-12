# Security review (v0.1.0)

A review of the server, the protocol and the game before the first public
release: what was found, what changed, and what remains.

## Threat model

Anyone can run a modified game (the protocol is public), open many
connections, script messages, and read the network traffic of their own
machine. Goals: nobody can cheat the rules or the ratings, harass people
without consequence, take over accounts, or take the server down cheaply.

## Findings and fixes

| # | Finding | Severity | Fix |
|---|---|---|---|
| 1 | Session tokens stored in plain text: a leaked database would sign anyone in. | High | Tokens are 256-bit random values stored as SHA-256 hashes; sessions renew after a day of use and expire after 30 days; staff sessions are separate and last 8 hours. |
| 2 | Password guessing limited only per connection (reconnect to reset). | High | Failures are counted per account (8 / 15 min) and per address (20 / 15 min) across connections; unknown usernames still run Argon2 (no timing difference). |
| 3 | Unlimited account creation. | Medium | 5 new accounts per address per hour; reserved names (`admin`, `snook`, `moderator`…). |
| 4 | Outgoing queues unbounded: a client that stops reading made the server buffer every broadcast forever (memory exhaustion). | High | Bounded queues (512 messages); a client that can't keep up is disconnected. |
| 5 | No message rate limit per connection. | Medium | 40 messages/s sustained, bursts of 120; 16 KiB per message; 12 connections per address. |
| 6 | Shot simulation ran inside the lounge lock: every shot froze the whole server for its duration (×10 on a small shared CPU). | High | Simulation runs on a blocking worker; the lounge only validates and commits. Password hashing is limited to two at a time. |
| 7 | Infinite or NaN stroke values were not rejected before reaching the physics. | Medium | Every float in `shoot`, `aim` and `place_cue_ball` must be finite; shots carry the turn number (`turn`) so stale or repeated shots are refused. |
| 8 | Chat accepted bidirectional overrides and zero-width characters (impersonation, hidden text). | Low | Control and invisible formatting characters are removed, spaces collapsed, length capped. Repeating the same line is refused. |
| 9 | The shot clock stopped while either player was disconnected: a losing player could stall by dropping the connection. | Medium | The clock keeps running; the reconnection grace only decides when the game is lost. |
| 10 | "Signed in elsewhere" was recognised by matching an English error string. | Low | A typed `kicked` message; the game doesn't reconnect after it. |
| 11 | The settings file (with the session token) was world-readable and written in place. | Low | Written to a temporary file and renamed; `0600` on Unix. |
| 12 | Minimum password length 6. | Low | 8–72 characters, not only digits, not one repeated character. |
| 13 | The client trusted the address of the proxy as the player's address. | Medium (on hosts behind a proxy) | `SNOOK_CLIENT_IP_HEADER` with a list of headers; the staff panel warns when every player shares one address. |
| 14 | Lounge actions that are announced to everyone (opening and closing tables, tournaments, the queue) had no limit of their own: one connection could make the server send thousands of messages a second to the whole lounge. | Medium | Their own limit per player (12 at once, then one a second); leaving a tournament one is not in says nothing to anyone. |
| 15 | Requests answered from the database (profiles, friends, blocks, reports, watching) were limited only by the general message limit, also before signing in. | Medium | Their own limit per connection (30 at once, then two a second). |
| 16 | Only text frames counted towards the flood limit (pings and binary frames were free). | Low | Every frame counts. |
| 17 | Signing in from a second place left the first connection's open table and place in the queue behind: a "ghost" table anyone could join, or a game starting for a player who never asked. | Low | The old connection's table, queue place, invitations and spectating end with it (a match in progress still carries over). |
| 18 | A tournament whose host left before it started stayed open for ever, holding one of the 20 tournament slots. | Low | Removed five minutes after the host goes away. |
| 19 | A refunded VIP month was looked up by the payment id only (VIP months are recorded by invoice): VIP and its coins stayed after the refund. A refund interrupted half-way could leave the coins behind. | Medium | Found by payment or invoice; the coins are taken back exactly once, keyed by the payment; partial refunds leave the purchase alone. |
| 20 | The first invoice of a subscription can arrive before the checkout that links the customer to the player: the paid month was dropped. | Medium | The checkout grants the first month as well (once per invoice), and the invoice finds the player through the subscription's metadata. |

## Built in from the start

- The server is authoritative: clients send strokes only; the server checks
  the turn, the cue-ball placement and the 8-ball call, simulates and decides.
- Ratings change only on the server.
- Rated games show only the short aiming line in the official game.
- All SQL uses bound parameters; `LIKE` patterns are escaped.
- Passwords: Argon2id (default parameters).
- TLS: `wss://` with rustls and the webpki roots (no system certificate store).
- Players can block (no chat, no invitations, friendship removed) and report;
  staff can mute, ban (with duration and reason), disconnect and revoke
  sessions; every staff action is audited.
- Staff panel: bearer tokens (no cookies, so no CSRF), strict CSP
  (`default-src 'self'`, no inline scripts), `X-Frame-Options: DENY`,
  `nosniff`, `no-referrer`, `no-store`, HSTS; every value is rendered with
  `textContent`.
- Container: distroless image, unprivileged user, no shell.
- CI: `cargo audit` checks dependencies against the RustSec advisories.

## Remaining risks

- **Aim assistance in a modified game.** (Mitigated: the server keeps each
  player's potting by difficulty band; the staff panel lists outliers of
  their tier for review, with replays. Nobody is banned automatically.)
  Original note: The server can't see what a client
  draws, so a modified game could show the full guide in rated games.
  Mitigation: reports, history review in the panel; a future option is
  server-side statistics on shot accuracy.
- **E-mail (partly addressed).** Players can set a recovery e-mail (offered
  from the payment checkout), change their password and delete their
  account (anonymised). The server still sends no e-mail itself; staff
  verify by the recovery e-mail and reset from the panel. Original note: Without e-mail there is no self-service password reset;
  staff can reset a password from the panel after verifying the player some
  other way.
- **One instance (deploys addressed).** Games are saved after every stroke
  and handed to the next server on SIGTERM. Original note: The lounge is in memory: a restart ends games in progress
  (accounts and results are safe). Fine for this size; scaling out would need
  a shared message bus.
- **macroquad soundness advisory (RUSTSEC-2025-0035).** The game's engine
  has known unsound internal APIs; they affect memory safety in edge cases of
  its own code, not network input. The server doesn't use it. Track upstream.
- **Unsigned Windows executable.** SmartScreen warns on first run until the
  game is code-signed or distributed through the Microsoft Store.
