# UI/UX review (v0.1.0)

What was reviewed in the game before the release, what changed and why.

## Start and menus

| Before | Problem | Now |
|---|---|---|
| The menu asked for the player's name, the second player's name and the level, all at once. | Too much at once; most of it irrelevant to what the player wants to do. | Four large choices (online, computer, two players, local network) and a row of icons (ranking, profile, settings, quit). Names, level, aiming guide and shot clock are asked on a setup screen for the chosen mode only. |
| Online status was invisible until entering the lounge. | No sense of identity or progress. | The player's card (flag, name in their tier colour, rating and tier) is always top right; the game signs in in the background when the session is saved. |
| The first connection to a sleeping server looked frozen. | Seemed broken. | A clear "the server is waking up" note with a spinner. |

## Lounge

- Tabs for **players**, **open tables**, **tournaments** and **friends**;
  rated/casual is one switch that applies to invitations and tables.
- Names are coloured by rating tier everywhere (lounge, chat, ranking,
  brackets, profiles), with the country's flag: status reads at a glance.
- Status dots (free / at a table / playing), "invite" disabled when it can't
  work, a red dot on Friends when a request arrives.
- Private tables and tournaments with a password: the password is asked
  inline, not in a dialog.
- Toasts (achievements, declined invitations, notices) stack at the bottom
  and never cover controls.

## At the table

- **Camera**: the "3D view / Top view / Reset camera" buttons became one
  icon (a camera with a back arrow: top view and default zoom, `C`). Only
  the right button turns the view (and goes 3D); the left button is only
  for aiming, so the camera never moves by accident.
- **Zoom bar**: a slim bar on the right edge, almost invisible until the
  pointer comes near, with + and −.
- **Replay**: a small icon next to the camera icon (and `R`) replays the last
  shot without touching the game; a discreet "REPLAY" tag shows it is one.
- **Chat**: online, what happens in the game (pocketed balls, fouls, ball in
  hand…) appears in the same chat, smaller, dimmer and without a name, with
  fouls in a muted red: easy to tell from what people write.
- **Calling the 8**: clicking a pocket on the table calls it (the pockets
  glow and the status line says so). The panel that used to cover the left
  of the table is gone. The call is binding for the rest of the game.
- **Folded chat**: a pulsing dot on the Chat button and the newest message
  for five seconds above it, so nothing is missed without opening it.
- **Reactions**: a small button by the chat opens a grid of pictures (locked
  ones in grey, with what unlocks them). A reaction pops up by the sender's
  card for under three seconds and stays visible while aiming.
- **Aiming guide online**: never the full guide against a person (it plays
  the cut for you): the standard one (ghost ball) or the short line, rated
  or not.
- **Shot clock** offline too (optional), on the player cards.
- The deciding shot is always shown before the result.

## Responsiveness

- Your own shot starts the instant you press the button (no round trip); the
  opponent's starts as soon as the server announces it; the opponent's cue
  glides instead of jumping 10 times a second.

## Watching friends

- An eye replaces "Invite" next to a friend who is playing. The game opens
  as for a player, but the bottom bar only says who is being watched, the
  reactions button and "Stop watching".
- Players see an eye and the number of spectators by their cards (hover for
  "3 watching"). Reactions from spectators are smaller and fainter than the
  players' own and drift down from the eye, a few at most.

## Review before launch

- Panels are almost opaque: the table behind no longer muddies the text.
- Settings use the same segmented controls as the setup screens (the old
  radio rows overlapped: "Off" ran into "2×").
- An empty lounge offers "Open a table" right there.
- The profile shows how far the next tier is and the reactions collected.
- The menu's left side is darker, so the buttons read over the table.
- Pocket castings are polished brass instead of flat cream plates.

## Text fields

Every field has a caret: arrows, Home/End, Delete, Ctrl+arrows (by word),
Ctrl+Backspace, Ctrl+A, Ctrl+C/V, and a click places the caret. Held keys
repeat. The LAN name field no longer refills itself when emptied.

## Accessibility and polish

- Interface scale follows the window, with a manual size in settings.
- Disabled buttons look disabled; every icon has a tooltip.
- English, Brazilian Portuguese and Spanish for every text, including server messages,
  achievements, countries (249) and match history.
- Gradients are textures (no banding); rounded rectangles no longer allocate
  every frame.
