# Staff panel

The server serves a small web app at `/admin` (plain HTML/JS, no external
resources, strict Content Security Policy). Staff sign in with their game
account; the session is separate from the game's and lasts 8 hours.

**Why in the server and not a separate site?** One deployment, the same
origin (no CORS, no API exposed to other sites), the same database
connection and direct access to the live lounge (who's online, games in
progress, kicking a player). A separate Vercel site would need a public admin
API, CORS and a second set of credentials for no gain at this size.

## Roles

| | Player | Moderator | Administrator |
|---|---|---|---|
| Overview, players, reports, live games, audit log | | ✓ | ✓ |
| Ban / unban, mute / unmute, disconnect, sign out everywhere | | ✓ (not on staff) | ✓ |
| Resolve reports, announcements in the lounge | | ✓ | ✓ |
| Change roles, reset a password, change the welcome message | | | ✓ |

The first administrators come from `SNOOKERED_ADMINS`; they can then promote
others from the panel. Nobody can act on their own account (except signing
out everywhere).

## What each action does

- **Ban** (1 day … permanent, with a reason): the player is disconnected,
  every session is revoked and signing in shows the reason and duration.
  Banned players leave the leaderboard.
- **Mute** (15 minutes … 1 week): the player can play but not chat.
- **Disconnect**: closes the connection (no reconnection loop).
- **Sign out everywhere**: revokes every saved session.
- **Reset password**: for account recovery (there are no e-mails): sets a new
  password and signs the player out everywhere.
- **Reports** come with the reporter, the reason, the player's note and the
  recent chat lines of the reported player (lounge and shared match).

Every staff action is written to the audit log with who, what, whom and when.
