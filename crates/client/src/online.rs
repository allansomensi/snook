//! The online part of the client: signing in, the lounge (players, chat,
//! invitations, leaderboard) and routing match messages to the table.

use macroquad::prelude::*;

use crate::i18n::{t, tw};
use snook_proto::{
    ChatLine, ClientMsg, LeaderboardEntry, MAX_CHAT_LEN, PlayerInfo, PlayerStatus, ServerMsg,
    valid_password, valid_username,
};

use crate::config::Config;
use crate::game::Match;
use crate::net::{Connection, NetEvent};
use crate::ui::{ALERT, BORDER, CREAM, GOLD, MUTED, Ui};

#[derive(Debug, Clone, PartialEq)]
pub enum Stage {
    /// Waiting for the socket to open.
    Connecting,
    /// The sign-in form.
    SignIn,
    /// Credentials sent, waiting for the answer.
    Authenticating,
    Lounge,
    /// The connection dropped while signed in; trying again.
    Reconnecting {
        attempt: u32,
        wait: f32,
    },
}

/// What the online screens ask of the rest of the game.
pub enum OnlineAction {
    None,
    /// Back to the main menu (the connection is closed).
    Leave,
}

pub struct Online {
    conn: Option<Connection>,
    pub stage: Stage,
    server: String,
    username: String,
    password: String,
    /// Error shown on the sign-in form.
    error: Option<String>,
    /// Short message in the lounge (invitation declined, …).
    info: Option<(String, f32)>,
    pub me: Option<PlayerInfo>,
    players: Vec<PlayerInfo>,
    chat: Vec<ChatLine>,
    chat_input: String,
    chat_scroll: usize,
    leaderboard: Vec<LeaderboardEntry>,
    leaderboard_age: f32,
    incoming: Vec<(String, bool)>,
    outgoing: Option<(String, bool)>,
    motd: String,
    token: String,
    /// The last thing sent to sign in, so it can be repeated after a reconnect.
    auto_resume: bool,
    rated_invites: bool,
    lounge_tab: Tab,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Players,
    Ranking,
}

const MAX_RECONNECTS: u32 = 12;

impl Online {
    /// Opens a connection; signs in automatically if a token was saved.
    pub fn start(config: &Config) -> Online {
        Online {
            conn: Some(Connection::open(&config.server)),
            stage: Stage::Connecting,
            server: config.server.clone(),
            username: config.username.clone(),
            password: String::new(),
            error: None,
            info: None,
            me: None,
            players: Vec::new(),
            chat: Vec::new(),
            chat_input: String::new(),
            chat_scroll: 0,
            leaderboard: Vec::new(),
            leaderboard_age: f32::MAX,
            incoming: Vec::new(),
            outgoing: None,
            motd: String::new(),
            token: config.token.clone(),
            auto_resume: !config.token.is_empty(),
            rated_invites: true,
            lounge_tab: Tab::Players,
        }
    }

    fn send(&self, msg: ClientMsg) {
        if let Some(c) = &self.conn {
            c.send(msg);
        }
    }

    pub fn signed_in(&self) -> bool {
        self.me.is_some()
    }

    /// Number of invitations waiting for an answer (shown during matches).
    pub fn pending_invites(&self) -> usize {
        self.incoming.len()
    }

    fn reconnect(&mut self) {
        self.conn = Some(Connection::open(&self.server));
    }

    /// Handles network traffic. Creates, updates or ends `current` when a match
    /// message arrives. Returns true if the config changed and should be saved.
    pub fn update(&mut self, dt: f32, current: &mut Option<Match>, config: &mut Config) -> bool {
        let mut save = false;
        if let Some((_, t)) = &mut self.info {
            *t -= dt;
            if *t <= 0.0 {
                self.info = None;
            }
        }
        // Outgoing match messages.
        if let Some(m) = current.as_mut()
            && let Some(o) = m.online.as_mut()
        {
            // Keep messages while reconnecting; they go out once connected.
            if let Some(c) = &self.conn
                && self.signed_in()
            {
                for msg in o.outbox.drain(..) {
                    c.send(msg);
                }
            }
        }
        if let Stage::Reconnecting { attempt, wait } = &mut self.stage {
            *wait -= dt;
            if *wait <= 0.0 && self.conn.is_none() {
                *attempt += 1;
                self.reconnect();
            }
        }
        if self.stage == Stage::Lounge {
            self.leaderboard_age += dt;
            if self.leaderboard_age > 30.0 && current.is_none() {
                self.leaderboard_age = 0.0;
                self.send(ClientMsg::Leaderboard);
            }
        }

        while let Some(event) = self.conn.as_mut().and_then(|c| c.poll()) {
            match event {
                NetEvent::Connected => {
                    if self.auto_resume && !self.token.is_empty() {
                        self.send(ClientMsg::Resume { token: self.token.clone() });
                        if !matches!(self.stage, Stage::Reconnecting { .. }) {
                            self.stage = Stage::Authenticating;
                        }
                    } else {
                        self.stage = Stage::SignIn;
                    }
                }
                NetEvent::Closed(reason) => {
                    self.conn = None;
                    let was_in = self.signed_in();
                    let attempt = match self.stage {
                        Stage::Reconnecting { attempt, .. } => attempt,
                        _ => 0,
                    };
                    if was_in && !self.token.is_empty() && attempt < MAX_RECONNECTS {
                        if attempt == 0
                            && let Some(m) = current.as_mut()
                        {
                            m.notify(t("Connection lost. Reconnecting…"), true);
                        }
                        self.stage = Stage::Reconnecting { attempt, wait: 2.0 + attempt as f32 };
                    } else {
                        if let Some(m) = current.take_if(|m| m.is_online()) {
                            drop(m);
                        }
                        self.me = None;
                        self.stage = Stage::SignIn;
                        self.error = Some(crate::i18n::server_text(&reason));
                    }
                }
                NetEvent::Message(msg) => save |= self.handle(*msg, current, config),
            }
        }
        save
    }

    fn handle(&mut self, msg: ServerMsg, current: &mut Option<Match>, config: &mut Config) -> bool {
        let mut save = false;
        match msg {
            ServerMsg::AuthOk { player, token } => {
                if let Some(m) = current.as_mut()
                    && matches!(self.stage, Stage::Reconnecting { .. })
                {
                    m.notify(t("Reconnected."), false);
                }
                self.username = player.username.clone();
                self.me = Some(player);
                self.token = token.clone();
                self.auto_resume = true;
                self.password.clear();
                self.error = None;
                self.stage = Stage::Lounge;
                self.leaderboard_age = f32::MAX;
                config.username = self.username.clone();
                config.token = token;
                config.server = self.server.clone();
                save = true;
            }
            ServerMsg::AuthError { reason } => {
                if self.auto_resume && !self.token.is_empty() {
                    // The saved session expired: ask for the password.
                    self.token.clear();
                    config.token.clear();
                    save = true;
                }
                self.auto_resume = false;
                self.me = None;
                if let Some(m) = current.take_if(|m| m.is_online()) {
                    drop(m);
                }
                self.stage = Stage::SignIn;
                self.error = Some(crate::i18n::server_text(&reason));
            }
            ServerMsg::Welcome { motd } => self.motd = crate::i18n::server_text(&motd),
            ServerMsg::Lounge { players } => self.players = players,
            ServerMsg::PlayerUpdated { player } => {
                if self.me.as_ref().is_some_and(|me| me.username == player.username) {
                    self.me = Some(player.clone());
                }
                match self.players.iter_mut().find(|p| p.username == player.username) {
                    Some(p) => *p = player,
                    None => self.players.push(player),
                }
            }
            ServerMsg::PlayerLeft { username } => {
                self.players.retain(|p| p.username != username);
                self.incoming.retain(|(from, _)| *from != username);
                if self.outgoing.as_ref().is_some_and(|(to, _)| *to == username) {
                    self.outgoing = None;
                }
            }
            ServerMsg::ChatHistory { lines } => {
                self.chat = lines;
                self.chat_scroll = 0;
            }
            ServerMsg::Chat { line } => {
                self.chat.push(line);
                if self.chat.len() > 200 {
                    self.chat.remove(0);
                }
                if self.chat_scroll > 0 {
                    self.chat_scroll += 1;
                }
            }
            ServerMsg::Invited { from, rated } => {
                self.incoming.retain(|(f, _)| *f != from);
                self.incoming.push((from, rated));
            }
            ServerMsg::InviteSent { to, rated } => self.outgoing = Some((to, rated)),
            ServerMsg::InviteCancelled { from } => self.incoming.retain(|(f, _)| *f != from),
            ServerMsg::InviteDeclined { by } => {
                self.outgoing = None;
                self.flash(tw("{by} declined your invitation.", &[("by", &by)]));
            }
            ServerMsg::InviteFailed { reason } => {
                self.outgoing = None;
                self.flash(crate::i18n::server_text(&reason));
            }
            ServerMsg::Leaderboard { entries } => self.leaderboard = entries,
            ServerMsg::MatchStarted {
                match_id,
                seat,
                players,
                rated,
                game,
                shot_clock_secs,
                clock_left,
            } => {
                self.outgoing = None;
                self.incoming.clear();
                let next = Match::online(
                    match_id,
                    seat,
                    &players,
                    rated,
                    game,
                    shot_clock_secs,
                    clock_left,
                    config.guide,
                );
                match current.as_mut() {
                    // Same match after reconnecting, or a rematch with the same opponent.
                    Some(m) if m.is_online() && same_players(m, &next) => m.continue_online(next),
                    _ => *current = Some(next),
                }
                // A message sent just before the match (e.g. "this game is casual").
                if let (Some((text, _)), Some(m)) = (self.info.take(), current.as_mut()) {
                    m.notify(text, true);
                }
            }
            ServerMsg::Error { message } if message.starts_with("You logged in from another") => {
                // Signed in elsewhere: don't fight over the account by reconnecting.
                self.token.clear();
                self.auto_resume = false;
                self.me = None;
                if let Some(m) = current.take_if(|m| m.is_online()) {
                    drop(m);
                }
                self.conn = None;
                self.stage = Stage::SignIn;
                self.error = Some(t("You signed in from another window or computer.").into());
            }
            ServerMsg::Error { message } => match current.as_mut() {
                Some(m) if m.is_online() => m.handle_server(ServerMsg::Error { message }),
                _ => self.flash(crate::i18n::server_text(&message)),
            },
            ServerMsg::Pong => {}
            other => {
                if let Some(m) = current.as_mut().filter(|m| m.is_online()) {
                    m.handle_server(other);
                }
            }
        }
        save
    }

    fn flash(&mut self, text: impl Into<String>) {
        self.info = Some((text.into(), 6.0));
    }

    /// Called when the player leaves the table to go back to the lounge.
    pub fn leave_match(&mut self, m: &Match) {
        if m.game.game_over().is_none() {
            self.send(ClientMsg::Resign);
        }
        self.send(ClientMsg::LeaveMatch);
        self.leaderboard_age = f32::MAX;
    }

    pub fn logout(&mut self, config: &mut Config) {
        self.send(ClientMsg::Logout);
        self.conn = None;
        self.me = None;
        self.token.clear();
        self.auto_resume = false;
        config.token.clear();
        config.save();
        self.reconnect();
        self.stage = Stage::Connecting;
    }

    /// Invitations received while at the table: answered from a small banner.
    pub fn draw_invites(&mut self, ui: &mut Ui, in_match: bool) {
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        let mut y = if in_match { 116.0 } else { (sh * 0.3).max(90.0) };
        if !in_match && !self.incoming.is_empty() {
            draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.4));
        }
        let mut answer = None;
        for (from, rated) in self.incoming.iter().take(3) {
            let w = 430.0;
            let r = Rect::new((sw - w) / 2.0, y, w, 74.0);
            ui.panel(r);
            crate::ui::rrect_lines(r, 8.0, 2.0, GOLD);
            let kind = if *rated { t("a rated game") } else { t("a casual game") };
            ui.text(
                &tw("{from} invites you to {kind}", &[("from", from), ("kind", &kind)]),
                r.x + 14.0,
                r.y + 10.0,
                16.0,
                CREAM,
                true,
            );
            if in_match {
                ui.text(t("Finish this match first."), r.x + 14.0, r.y + 42.0, 14.0, MUTED, false);
            } else if ui
                .primary_button(Rect::new(r.x + w - 214.0, r.y + 38.0, 96.0, 28.0), t("Accept"))
            {
                answer = Some((from.clone(), true));
            }
            if ui.button(Rect::new(r.x + w - 110.0, r.y + 38.0, 96.0, 28.0), t("Decline"), true) {
                answer = Some((from.clone(), false));
            }
            y += 82.0;
        }
        if let Some((from, accept)) = answer {
            self.incoming.retain(|(f, _)| *f != from);
            self.send(ClientMsg::AnswerInvite { from, accept });
        }
    }

    /// Draws the sign-in form or the lounge.
    pub fn draw(&mut self, ui: &mut Ui, config: &mut Config) -> OnlineAction {
        match self.stage.clone() {
            Stage::Lounge => self.draw_lounge(ui, config),
            Stage::Reconnecting { attempt, .. } => {
                let text =
                    tw("Connection lost. Reconnecting (attempt {n})…", &[("n", &(attempt + 1))]);
                self.waiting(ui, &text)
            }
            Stage::Connecting => {
                let text = tw("Connecting to {server}…", &[("server", &self.server)]);
                self.waiting(ui, &text)
            }
            Stage::Authenticating => self.waiting(ui, t("Signing in…")),
            Stage::SignIn => self.draw_sign_in(ui, config),
        }
    }

    fn waiting(&mut self, ui: &mut Ui, text: &str) -> OnlineAction {
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        let r = Rect::new(sw / 2.0 - 230.0, sh / 2.0 - 70.0, 460.0, 140.0);
        ui.panel(r);
        ui.text_centered(text, sw / 2.0, r.y + 28.0, 16.0, CREAM, false);
        let dots = ((get_time() * 3.0) as usize) % 4;
        ui.text_centered(&"●".repeat(dots + 1), sw / 2.0, r.y + 56.0, 12.0, GOLD, false);
        if ui.button(Rect::new(sw / 2.0 - 70.0, r.y + 90.0, 140.0, 32.0), t("Cancel"), true) {
            self.conn = None;
            return OnlineAction::Leave;
        }
        OnlineAction::None
    }

    fn draw_sign_in(&mut self, ui: &mut Ui, config: &mut Config) -> OnlineAction {
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        let w = 420.0;
        let h = 372.0;
        let r = Rect::new((sw - w) / 2.0, ((sh - h) / 2.0).max(10.0), w, h);
        ui.panel(r);
        let x = r.x + 26.0;
        let inner = w - 52.0;
        let mut y = r.y + 18.0;
        ui.text_centered(t("Online lounge"), sw / 2.0, y, 24.0, GOLD, true);
        y += 36.0;
        ui.text_centered(
            t("Play people around the world and climb the ranking."),
            sw / 2.0,
            y,
            14.0,
            MUTED,
            false,
        );
        y += 34.0;

        let mut submit = false;
        ui.text(t("Username"), x, y + 7.0, 15.0, CREAM, false);
        submit |=
            ui.text_input(20, Rect::new(x + 104.0, y, inner - 104.0, 32.0), &mut self.username, 16);
        y += 44.0;
        ui.text(t("Password"), x, y + 7.0, 15.0, CREAM, false);
        submit |= ui.password_input(
            21,
            Rect::new(x + 104.0, y, inner - 104.0, 32.0),
            &mut self.password,
            72,
        );
        y += 48.0;

        let name_ok = valid_username(self.username.trim());
        let pass_ok = valid_password(&self.password);
        let connected = self.conn.is_some();
        let can = name_ok && pass_ok && connected;
        let mut action = None;
        if ui.primary_button(Rect::new(x, y, inner / 2.0 - 6.0, 38.0), t("Log in"))
            || (submit && can)
        {
            action = Some(false);
        }
        if ui.button(
            Rect::new(x + inner / 2.0 + 6.0, y, inner / 2.0 - 6.0, 38.0),
            t("Create account"),
            true,
        ) {
            action = Some(true);
        }
        y += 52.0;
        if let Some(register) = action {
            if !connected {
                self.reconnect();
                self.stage = Stage::Connecting;
                self.error = None;
            } else if !name_ok {
                self.error = Some(t("Usernames have 3–16 letters, digits or underscores.").into());
            } else if !pass_ok {
                self.error = Some(t("Passwords need at least 6 characters.").into());
            } else {
                let username = self.username.trim().to_string();
                let password = self.password.clone();
                self.send(if register {
                    ClientMsg::Register { username, password }
                } else {
                    ClientMsg::Login { username, password }
                });
                self.auto_resume = false;
                self.error = None;
                self.stage = Stage::Authenticating;
            }
        }
        match &self.error {
            Some(e) => {
                for (k, line) in ui.wrap(e, inner, 14.0).iter().take(3).enumerate() {
                    ui.text_centered(line, sw / 2.0, y + k as f32 * 18.0, 14.0, ALERT, false);
                }
            }
            None => ui.text_centered(
                t("New here? Pick a name and password, then Create account."),
                sw / 2.0,
                y,
                13.0,
                MUTED,
                false,
            ),
        }
        y += 58.0;
        ui.text(&tw("Server: {s}", &[("s", &self.server)]), x, y + 6.0, 12.0, MUTED, false);
        if !connected && ui.button(Rect::new(x + inner - 100.0, y, 100.0, 26.0), t("Retry"), true) {
            self.server = config.server.clone();
            self.reconnect();
            self.stage = Stage::Connecting;
            self.error = None;
        }
        y += 34.0;
        if ui.button(Rect::new(x, y, inner, 32.0), t("Back to menu"), true) {
            self.conn = None;
            return OnlineAction::Leave;
        }
        OnlineAction::None
    }

    fn draw_lounge(&mut self, ui: &mut Ui, config: &mut Config) -> OnlineAction {
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.5));
        let mut action = OnlineAction::None;
        let margin = 14.0;
        let top = 14.0;

        // Header: who am I.
        let header = Rect::new(margin, top, sw - margin * 2.0, 54.0);
        ui.panel(header);
        ui.text("SNOOK", header.x + 16.0, header.y + 13.0, 26.0, GOLD, true);
        let title_w = ui.text_size("SNOOK", 26.0, true).x;
        ui.text(t("lounge"), header.x + 26.0 + title_w, header.y + 22.0, 15.0, MUTED, false);
        if let Some(me) = &self.me {
            let text = tw(
                "{name}  ·  rating {r}  ·  {g} games, {w} wins",
                &[("name", &me.username), ("r", &me.rating), ("g", &me.games), ("w", &me.wins)],
            );
            let w = ui.text_size(&text, 16.0, true).x;
            ui.text(&text, header.x + header.w - w - 210.0, header.y + 17.0, 16.0, CREAM, true);
        }
        if ui.button(
            Rect::new(header.x + header.w - 196.0, header.y + 12.0, 86.0, 30.0),
            t("Log out"),
            true,
        ) {
            self.logout(config);
            return OnlineAction::None;
        }
        if ui.button(
            Rect::new(header.x + header.w - 100.0, header.y + 12.0, 86.0, 30.0),
            t("Menu"),
            true,
        ) {
            // Keep the saved session: the next visit signs in automatically.
            self.conn = None;
            action = OnlineAction::Leave;
        }

        let body_y = header.y + header.h + 10.0;
        let body_h = sh - body_y - margin;
        let left_w = (sw * 0.36).clamp(320.0, 460.0);
        let left = Rect::new(margin, body_y, left_w, body_h);
        let right =
            Rect::new(margin + left_w + 10.0, body_y, sw - left_w - margin * 2.0 - 10.0, body_h);

        // Left: tabs with players and the ranking.
        ui.panel(left);
        let tab_w = (left.w - 30.0) / 2.0;
        for (k, (tab, label)) in
            [(Tab::Players, t("Players online")), (Tab::Ranking, t("Top players"))]
                .into_iter()
                .enumerate()
        {
            let r =
                Rect::new(left.x + 10.0 + k as f32 * (tab_w + 10.0), left.y + 10.0, tab_w, 30.0);
            if ui.tab(r, label, self.lounge_tab == tab) {
                self.lounge_tab = tab;
                if tab == Tab::Ranking {
                    self.send(ClientMsg::Leaderboard);
                }
            }
        }
        let reserve = if self.outgoing.is_some() { 64.0 } else { 0.0 };
        let list = Rect::new(left.x + 10.0, left.y + 50.0, left.w - 20.0, left.h - 60.0 - reserve);
        match self.lounge_tab {
            Tab::Players => self.draw_players(ui, list),
            Tab::Ranking => self.draw_ranking(ui, list),
        }

        // Right: chat.
        ui.panel(right);
        ui.text(t("Lounge chat"), right.x + 14.0, right.y + 12.0, 16.0, GOLD, true);
        if !self.motd.is_empty() {
            let w = right.w - 150.0;
            let motd = ui.ellipsize(&self.motd, w, 13.0);
            ui.text(&motd, right.x + 130.0, right.y + 15.0, 13.0, MUTED, false);
        }
        let input_h = 34.0;
        let log = Rect::new(
            right.x + 12.0,
            right.y + 42.0,
            right.w - 24.0,
            right.h - 42.0 - input_h - 22.0,
        );
        crate::ui::rrect(log, 6.0, Color::new(0.0, 0.0, 0.0, 0.25));
        self.chat_scroll = ui.chat_log(
            log,
            &self.chat,
            self.me.as_ref().map(|m| m.username.as_str()),
            self.chat_scroll,
        );
        let input = Rect::new(
            right.x + 12.0,
            right.y + right.h - input_h - 12.0,
            right.w - 24.0 - 90.0,
            input_h,
        );
        let sent = ui.text_input(30, input, &mut self.chat_input, MAX_CHAT_LEN);
        if self.chat_input.is_empty() && ui.focus != Some(30) {
            ui.text(
                t("Say hello… (Enter to send)"),
                input.x + 9.0,
                input.y + 9.0,
                15.0,
                MUTED,
                false,
            );
        }
        let send_r = Rect::new(input.x + input.w + 8.0, input.y, 82.0, input_h);
        if (ui.button(send_r, t("Send"), !self.chat_input.trim().is_empty()) || sent)
            && !self.chat_input.trim().is_empty()
        {
            let text = snook_proto::clean_chat(&self.chat_input);
            self.send(ClientMsg::Chat { text });
            self.chat_input.clear();
            self.chat_scroll = 0;
            ui.focus = Some(30);
        }

        // Outgoing invitation.
        if let Some((to, rated)) = self.outgoing.clone() {
            let r = Rect::new(left.x + 10.0, left.y + left.h - 62.0, left.w - 20.0, 52.0);
            let w = r.w;
            ui.panel(r);
            crate::ui::rrect_lines(r, 8.0, 2.0, GOLD);
            let kind = if rated { t("rated") } else { t("casual") };
            let text = ui.ellipsize(
                &tw("Waiting for {to} ({kind})…", &[("to", &to), ("kind", &kind)]),
                w - 130.0,
                15.0,
            );
            ui.text(&text, r.x + 14.0, r.y + 16.0, 15.0, CREAM, false);
            if ui.button(Rect::new(r.x + w - 100.0, r.y + 11.0, 88.0, 30.0), t("Cancel"), true) {
                self.send(ClientMsg::CancelInvite { to });
                self.outgoing = None;
            }
        }
        if let Some((text, t)) = &self.info {
            let alpha = t.min(1.0);
            let w = ui.text_size(text, 15.0, false).x + 30.0;
            let r = Rect::new((sw - w) / 2.0, sh - 120.0, w, 34.0);
            crate::ui::rrect(r, 17.0, Color::new(0.0, 0.0, 0.0, 0.75 * alpha));
            ui.text_centered(
                text,
                sw / 2.0,
                r.y + 9.0,
                15.0,
                Color::new(ALERT.r, ALERT.g, ALERT.b, alpha),
                false,
            );
        }
        self.draw_invites(ui, false);
        action
    }

    fn draw_players(&mut self, ui: &mut Ui, area: Rect) {
        ui.checkbox(area.x, area.y, t("Rated games (affect your rating)"), &mut self.rated_invites);
        let me = self.me.as_ref().map(|m| m.username.clone()).unwrap_or_default();
        let mut players: Vec<&PlayerInfo> =
            self.players.iter().filter(|p| p.username != me).collect();
        players.sort_by(|a, b| {
            (a.status == PlayerStatus::Playing)
                .cmp(&(b.status == PlayerStatus::Playing))
                .then(b.rating.cmp(&a.rating))
        });
        let mut y = area.y + 30.0;
        if players.is_empty() {
            ui.text(t("Nobody else is here right now."), area.x, y + 10.0, 15.0, MUTED, false);
            ui.text(
                t("Say hi in the chat, or invite a friend!"),
                area.x,
                y + 32.0,
                14.0,
                MUTED,
                false,
            );
            return;
        }
        let row_h = 40.0;
        let mut invite = None;
        let rows = ((area.y + area.h - y) / row_h).floor().max(0.0) as usize;
        for p in players.iter().take(rows) {
            let r = Rect::new(area.x, y, area.w, row_h - 4.0);
            crate::ui::rrect(r, 6.0, Color::new(1.0, 1.0, 1.0, 0.04));
            let dot = if p.status == PlayerStatus::Playing {
                Color::new(0.9, 0.6, 0.2, 1.0)
            } else {
                Color::new(0.35, 0.85, 0.45, 1.0)
            };
            draw_circle(r.x + 12.0, r.y + r.h / 2.0, 5.0, dot);
            let name = ui.ellipsize(&p.username, r.w - 190.0, 15.0);
            ui.text(&name, r.x + 26.0, r.y + 4.0, 15.0, CREAM, true);
            let sub = if p.status == PlayerStatus::Playing {
                tw("{r} · playing", &[("r", &p.rating)])
            } else {
                tw("{r} · {g} games", &[("r", &p.rating), ("g", &p.games)])
            };
            ui.text(&sub, r.x + 26.0, r.y + 21.0, 12.0, MUTED, false);
            let can = p.status == PlayerStatus::Available && self.outgoing.is_none();
            if ui.button(Rect::new(r.x + r.w - 92.0, r.y + 5.0, 86.0, r.h - 10.0), t("Invite"), can)
            {
                invite = Some(p.username.clone());
            }
            y += row_h;
        }
        if players.len() > rows {
            ui.text(
                &tw("…and {n} more", &[("n", &(players.len() - rows))]),
                area.x,
                area.y + area.h - 16.0,
                12.0,
                MUTED,
                false,
            );
        }
        if let Some(to) = invite {
            self.send(ClientMsg::Invite { to, rated: self.rated_invites });
        }
    }

    fn draw_ranking(&mut self, ui: &mut Ui, area: Rect) {
        let me = self.me.as_ref().map(|m| m.username.clone()).unwrap_or_default();
        ui.text("#", area.x + 4.0, area.y, 13.0, MUTED, true);
        ui.text(t("Player"), area.x + 40.0, area.y, 13.0, MUTED, true);
        ui.text(t("Rating"), area.x + area.w - 150.0, area.y, 13.0, MUTED, true);
        ui.text(t("Won"), area.x + area.w - 60.0, area.y, 13.0, MUTED, true);
        draw_line(area.x, area.y + 20.0, area.x + area.w, area.y + 20.0, 1.0, BORDER);
        if self.leaderboard.is_empty() {
            ui.text(
                t("No rated games yet. Be the first!"),
                area.x + 4.0,
                area.y + 34.0,
                15.0,
                MUTED,
                false,
            );
            return;
        }
        let row_h = 24.0;
        let rows = ((area.h - 28.0) / row_h).floor().max(0.0) as usize;
        for (k, e) in self.leaderboard.iter().take(rows).enumerate() {
            let y = area.y + 28.0 + k as f32 * row_h;
            let mine = e.username == me;
            if mine {
                draw_rectangle(area.x, y - 3.0, area.w, row_h, Color::new(0.91, 0.75, 0.38, 0.15));
            }
            let color = match e.rank {
                1 => GOLD,
                2 => Color::new(0.82, 0.82, 0.86, 1.0),
                3 => Color::new(0.80, 0.55, 0.33, 1.0),
                _ => CREAM,
            };
            ui.text(&e.rank.to_string(), area.x + 4.0, y, 14.0, color, true);
            let name = ui.ellipsize(&e.username, area.w - 200.0, 14.0);
            ui.text(&name, area.x + 40.0, y, 14.0, if mine { GOLD } else { CREAM }, mine);
            ui.text(&e.rating.to_string(), area.x + area.w - 150.0, y, 14.0, CREAM, true);
            let pct = (e.wins * 100).checked_div(e.games).unwrap_or(0);
            ui.text(&format!("{pct}%"), area.x + area.w - 60.0, y, 14.0, MUTED, false);
        }
    }
}

fn same_players(a: &Match, b: &Match) -> bool {
    let mut x = a.names.clone();
    let mut y = b.names.clone();
    x.sort();
    y.sort();
    x == y
}
