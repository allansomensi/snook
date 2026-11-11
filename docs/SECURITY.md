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

- **Aim assistance in a modified game.** The server can't see what a client
  draws, so a modified game could show the full guide in rated games.
  Mitigation: reports, history review in the panel; a future option is
  server-side statistics on shot accuracy.
- **No e-mail.** Without e-mail there is no self-service password reset;
  staff can reset a password from the panel after verifying the player some
  other way.
- **One instance.** The lounge is in memory: a restart ends games in progress
  (accounts and results are safe). Fine for this size; scaling out would need
  a shared message bus.
- **macroquad soundness advisory (RUSTSEC-2025-0035).** The game's engine
  has known unsound internal APIs; they affect memory safety in edge cases of
  its own code, not network input. The server doesn't use it. Track upstream.
- **Unsigned Windows executable.** SmartScreen warns on first run until the
  game is code-signed or distributed through the Microsoft Store.
