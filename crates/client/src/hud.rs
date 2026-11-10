//! In-game interface: scoreboard, shot panel, notices, help, game over and pause.

use macroquad::prelude::*;

#[allow(unused_imports)]
use crate::paint::{draw_circle, draw_circle_lines, draw_line, draw_rectangle, draw_triangle};

use crate::i18n::{t, tw};
use snook_core::{BallId, BallInHand, Group};

use crate::game::{Controller, Match, Phase};
use crate::textures::ball_color;
use crate::ui::{ALERT, BORDER, CREAM, GOLD, MUTED, Ui};

pub struct Hud {
    /// The pause menu is open (online, the game keeps running behind it).
    pub paused: bool,
    pub show_help: bool,
    chat_input: String,
    chat_open: bool,
    chat_scroll: usize,
    confirm_leave: bool,
    /// Identifies the turn the banner was shown for.
    banner_key: Option<(u32, u32, usize)>,
    /// (title, subtitle, age in seconds).
    banner: Option<(String, String, f32)>,
    /// Set by the caller: the camera is in the overhead view.
    pub top_view: bool,
    /// Set by the caller: multiplier for fine aiming.
    pub aim_sensitivity: f32,
    /// Last pointer x while dragging the fine-aim strip.
    fine_last: Option<f32>,
    /// The power bar is being dragged.
    stroking: bool,
    /// The cue-setting panel that is open above the bottom bar.
    popover: Option<Pop>,
    /// The current stroke was cancelled (right click / Esc) until release.
    stroke_cancelled: bool,
}

/// What the player asked for through the HUD.
pub enum HudAction {
    None,
    MainMenu,
    /// Online: back to the lounge.
    Lounge,
    Settings,
    ToggleView,
    ResetCamera,
}

impl Default for Hud {
    fn default() -> Self {
        Hud {
            paused: false,
            show_help: true,
            chat_input: String::new(),
            chat_open: true,
            chat_scroll: 0,
            confirm_leave: false,
            banner_key: None,
            banner: None,
            top_view: false,
            aim_sensitivity: 1.0,
            fine_last: None,
            stroking: false,
            popover: None,
            stroke_cancelled: false,
        }
    }
}

impl Hud {
    pub fn new(show_help: bool) -> Self {
        Hud { show_help, ..Hud::default() }
    }
}

const CHAT_FIELD: u64 = 40;

fn ball_icon(ui: &Ui, id: BallId, x: f32, y: f32, size: f32) {
    let c = ball_color(id);
    let color = Color::from_rgba(c[0], c[1], c[2], 255);
    let r = size / 2.0;
    let (cx, cy) = (x + r, y + r);
    if id.0 > 8 {
        draw_circle(cx, cy, r, Color::from_rgba(246, 242, 228, 255));
        draw_rectangle(x + 1.0, cy - size * 0.22, size - 2.0, size * 0.44, color);
    } else {
        draw_circle(cx, cy, r, color);
    }
    draw_circle(cx, cy, r * 0.45, Color::from_rgba(246, 242, 228, 255));
    draw_circle_lines(cx, cy, r, 1.0, Color::new(0.0, 0.0, 0.0, 0.6));
    let label = id.0.to_string();
    let fs = size * 0.38;
    ui.text_centered(&label, cx, cy - fs / 2.0 - 1.0, fs, BLACK, true);
}

fn player_card(ui: &mut Ui, m: &Match, seat: usize, x: f32, y: f32, w: f32) {
    let active = m.game.current_player() == seat && !matches!(m.phase, Phase::GameOver);
    let r = Rect::new(x, y, w, CARD_H);
    crate::ui::rrect(
        r,
        8.0,
        if active {
            Color::new(0.20, 0.13, 0.06, 0.95)
        } else {
            Color::new(0.07, 0.045, 0.03, 0.9)
        },
    );
    crate::ui::rrect_lines(
        r,
        8.0,
        if active { 2.0 } else { 1.0 },
        if active { GOLD } else { crate::ui::BORDER },
    );
    let reserve = if m.online.is_some() { 96.0 } else { 40.0 };
    let name = ui.ellipsize(&m.names[seat], w - reserve, 15.0);
    ui.text(&name, x + 10.0, y + 4.0, 15.0, if active { GOLD } else { CREAM }, true);
    let wins = m.wins[seat].to_string();
    let ww = ui.text_size(&wins, 18.0, true).x;
    ui.text(&wins, x + w - ww - 10.0, y + 3.0, 18.0, CREAM, true);
    if let Some(o) = &m.online {
        let rating = o.ratings[seat].to_string();
        let rw = ui.text_size(&rating, 12.0, false).x;
        ui.text(&rating, x + w - ww - rw - 20.0, y + 7.0, 12.0, MUTED, false);
        if active && matches!(m.phase, Phase::Turn { .. }) {
            // Shot clock along the bottom of the card.
            let frac = (o.clock_left / o.shot_clock.max(1.0)).clamp(0.0, 1.0);
            let color = if o.clock_left < 10.0 { ALERT } else { GOLD };
            crate::ui::rrect(
                Rect::new(x + 6.0, y + CARD_H - 4.0, (w - 12.0) * frac, 2.5),
                1.2,
                color,
            );
            let secs = format!("{:.0}s", o.clock_left.ceil());
            let sw_ = ui.text_size(&secs, 13.0, true).x;
            ui.text(&secs, x + w - sw_ - 10.0, y + 24.0, 13.0, color, true);
        }
    }
    let ry = y + 23.0;
    match m.game.group_of(seat) {
        Some(group) => {
            let remaining = m.game.remaining(group);
            if remaining.is_empty() {
                ui.text(t("On the 8-ball"), x + 10.0, ry + 1.0, 13.0, GOLD, false);
                let tw_ = ui.text_size(t("On the 8-ball"), 13.0, false).x;
                ball_icon(ui, BallId::EIGHT, x + 16.0 + tw_, ry, 16.0);
            } else {
                for (k, id) in remaining.into_iter().enumerate() {
                    ball_icon(ui, id, x + 10.0 + k as f32 * 18.0, ry, 16.0);
                }
            }
        }
        None => ui.text(t("Open table"), x + 10.0, ry + 1.0, 13.0, MUTED, false),
    }
}

fn status_line(m: &Match) -> Option<String> {
    let name = &m.names[m.game.current_player()];
    match &m.phase {
        Phase::Awaiting => Some(t("Shooting…").into()),
        Phase::Turn { placing_cue_ball, computer } => {
            if computer.is_some() {
                return Some(tw("{name} is thinking…", &[("name", name)]));
            }
            if m.current_controller() == Controller::Remote {
                return Some(if m.game.ball_in_hand().is_some() {
                    tw("{name} has ball in hand…", &[("name", name)])
                } else {
                    tw("{name} is aiming…", &[("name", name)])
                });
            }
            if *placing_cue_ball {
                let zone = if m.game.ball_in_hand() == Some(BallInHand::Kitchen) {
                    t(" behind the line")
                } else {
                    ""
                };
                return Some(tw(
                    "Ball in hand: drag the cue ball{zone} · release to aim",
                    &[("zone", &zone)],
                ));
            }
            if m.game.is_break() {
                return Some(tw("{name} to break", &[("name", name)]));
            }
            if m.game.needs_called_pocket() && m.called_pocket.is_none() {
                return Some(tw(
                    "{name} is on the 8: click a pocket to call it",
                    &[("name", name)],
                ));
            }
            let target = match m.game.group_of(m.game.current_player()) {
                Some(Group::Solids) => t("solids"),
                Some(Group::Stripes) => t("stripes"),
                None => t("any ball"),
            };
            Some(tw("{name} to shoot ({target})", &[("name", name), ("target", &target)]))
        }
        _ => None,
    }
}

impl Hud {
    pub fn draw(&mut self, ui: &mut Ui, m: &mut Match) -> HudAction {
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        let mut action = HudAction::None;

        if !ui.wants_keyboard {
            if is_key_pressed(KeyCode::Escape) && !m.aim.charging {
                self.paused = !self.paused;
                self.confirm_leave = false;
            }
            if is_key_pressed(KeyCode::H) || is_key_pressed(KeyCode::F1) {
                self.show_help = !self.show_help;
            }
        }

        // Top bar: menu and camera on the left, the players in the middle.
        draw_bar(Rect::new(0.0, 0.0, sw, TOP_BAR), true);
        // Cards centred, but never under the buttons on the left.
        let view = if self.top_view { t("3D view") } else { t("Top view") };
        let buttons_w: f32 = [t("Menu"), view, t("Reset camera")]
            .iter()
            .map(|l| ui.text_size(l, 13.0, true).x + 28.0)
            .sum::<f32>()
            + 14.0;
        let room = sw - 2.0 * buttons_w;
        let card_w = ((room - 10.0) / 2.0).clamp(150.0, 270.0);
        let total = card_w * 2.0 + 10.0;
        let x0 = ((sw - total) / 2.0).max(buttons_w).min(sw - total - 8.0);
        player_card(ui, m, 0, x0, 6.0, card_w);
        player_card(ui, m, 1, x0 + card_w + 10.0, 6.0, card_w);
        let mut y = TOP_BAR + 8.0;
        if let Some(status) = status_line(m) {
            let w = ui.text_size(&status, 15.0, false).x + 24.0;
            crate::ui::rrect(
                Rect::new((sw - w) / 2.0, y, w, 24.0),
                12.0,
                Color::new(0.0, 0.0, 0.0, 0.6),
            );
            ui.text_centered(&status, sw / 2.0, y + 4.0, 15.0, CREAM, false);
            y += 30.0;
        }
        // Only important messages float on the table; everything is in the log.
        for n in m.notices.iter().filter(|n| n.important) {
            let alpha = n.ttl.min(1.0);
            let base = if n.important { ALERT } else { CREAM };
            let color = Color::new(base.r, base.g, base.b, alpha);
            let w = ui.text_size(&n.text, 15.0, false).x + 20.0;
            crate::ui::rrect(
                Rect::new((sw - w) / 2.0, y, w, 22.0),
                11.0,
                Color::new(0.0, 0.0, 0.0, 0.45 * alpha),
            );
            ui.text_centered(&n.text, sw / 2.0, y + 3.0, 15.0, color, false);
            y += 25.0;
        }

        self.draw_turn_banner(ui, m);
        draw_power_meter(ui, m);

        // Cue panel (spin, elevation, fine aim) and the power stroke bar:
        // everything can be played with the mouse alone.
        self.draw_bottom_bar(ui, m);
        if m.is_human_turn() && m.game.needs_called_pocket() && !self.paused {
            draw_call_panel(ui, m);
        }

        // Menu and camera buttons in the top bar.
        if !self.paused {
            let view = if self.top_view { t("3D view") } else { t("Top view") };
            let mut x = 10.0;
            for (k, label) in [t("Menu"), view, t("Reset camera")].into_iter().enumerate() {
                let w = ui.text_size(label, 13.0, true).x + 22.0;
                if ui.button(Rect::new(x, 12.0, w, 30.0), label, true) {
                    match k {
                        0 => self.paused = true,
                        1 => action = HudAction::ToggleView,
                        _ => action = HudAction::ResetCamera,
                    }
                }
                x += w + 6.0;
            }
        }
        if self.show_help && !self.paused {
            let lines = [
                (t("Aim"), t("Drag the cue · click a spot")),
                (t("Fine aim"), t("Shift + drag · A / D")),
                (t("Quick aim"), t("Click a ball or a spot")),
                (t("Shoot"), t("Drag the power bar right, release")),
                (t("Spin"), t("Spin ball · ↑ ↓ · Q / E")),
                (t("Elevation"), t("W / S (jump & massé)")),
                (t("Camera"), t("Right-drag · Wheel · Tab · C")),
                (t("Ball in hand"), t("Drag the cue ball")),
                (t("Menu / help"), t("Esc · H")),
            ];
            let h = 34.0 + lines.len() as f32 * 19.0;
            let bottom = sh
                - BOTTOM_BAR
                - 8.0
                - if m.is_online() && self.chat_open { CHAT_H + 8.0 } else { 0.0 };
            let label_w =
                lines.iter().map(|(a, _)| ui.text_size(a, 13.0, true).x).fold(0.0, f32::max);
            let value_w =
                lines.iter().map(|(_, b)| ui.text_size(b, 13.0, false).x).fold(0.0, f32::max);
            let r = Rect::new(12.0, bottom - h, label_w + value_w + 40.0, h);
            ui.panel(r);
            ui.text(t("Controls"), r.x + 12.0, r.y + 8.0, 14.0, GOLD, true);
            for (k, (a, b)) in lines.iter().enumerate() {
                let yy = r.y + 30.0 + k as f32 * 19.0;
                ui.text(a, r.x + 12.0, yy, 13.0, CREAM, true);
                ui.text(b, r.x + label_w + 26.0, yy, 13.0, MUTED, false);
            }
        }

        if m.is_online() {
            self.draw_chat(ui, m);
        }

        // Game over.
        if let (Phase::GameOver, Some(over), true) = (&m.phase, m.game.game_over(), m.is_online()) {
            let o = m.online.as_ref().expect("online");
            let me = o.seat;
            let won = over.winner == me;
            let r = Rect::new(sw / 2.0 - 220.0, sh / 2.0 - 110.0, 440.0, 220.0);
            ui.panel(r);
            crate::ui::rrect_lines(r, 8.0, 2.0, if won { GOLD } else { BORDER });
            let title = if won {
                t("You win!").to_string()
            } else {
                tw("{name} wins", &[("name", &m.names[over.winner])])
            };
            ui.text_centered(&title, sw / 2.0, r.y + 16.0, 28.0, GOLD, true);
            ui.text_centered(
                crate::i18n::reason(over.reason),
                sw / 2.0,
                r.y + 56.0,
                15.0,
                CREAM,
                false,
            );
            let score = format!("{}  {} – {}  {}", m.names[0], m.wins[0], m.wins[1], m.names[1]);
            ui.text_centered(&score, sw / 2.0, r.y + 82.0, 15.0, CREAM, true);
            match &o.result {
                Some(res) if res.rated => {
                    let d = res.rating_delta[me];
                    let delta = format!("{}{}", if d >= 0 { "+" } else { "" }, d);
                    let text =
                        tw("Rating {r} ({d})", &[("r", &res.new_ratings[me]), ("d", &delta)]);
                    let color = if d >= 0 { Color::new(0.45, 1.0, 0.55, 1.0) } else { ALERT };
                    ui.text_centered(&text, sw / 2.0, r.y + 108.0, 18.0, color, true);
                }
                Some(_) => ui.text_centered(
                    t("Casual game: ratings unchanged"),
                    sw / 2.0,
                    r.y + 110.0,
                    14.0,
                    MUTED,
                    false,
                ),
                None => ui.text_centered(
                    t("Saving the result…"),
                    sw / 2.0,
                    r.y + 110.0,
                    14.0,
                    MUTED,
                    false,
                ),
            }
            let note = if o.opponent_left {
                t("Your opponent went back to the lounge.").to_string()
            } else if o.rematch_requested {
                tw("Waiting for {name}…", &[("name", &m.names[1 - me])])
            } else if o.rematch_offered {
                tw("{name} wants a rematch!", &[("name", &m.names[1 - me])])
            } else {
                String::new()
            };
            ui.text_centered(&note, sw / 2.0, r.y + 138.0, 14.0, GOLD, false);
            let can_rematch = !o.opponent_left && !o.rematch_requested;
            let label = if o.rematch_offered { t("Accept rematch") } else { t("Rematch") };
            let rr = Rect::new(r.x + 30.0, r.y + 164.0, 180.0, 38.0);
            if can_rematch {
                if ui.primary_button(rr, label) {
                    m.rematch();
                }
            } else {
                ui.button(rr, t("Rematch"), false);
            }
            if ui.button(
                Rect::new(r.x + 230.0, r.y + 164.0, 180.0, 38.0),
                t("Back to lounge"),
                true,
            ) {
                action = HudAction::Lounge;
            }
        } else if let (Phase::GameOver, Some(over)) = (&m.phase, m.game.game_over()) {
            let all_computers = m.controllers.iter().all(|c| matches!(c, Controller::Computer(_)));
            let r = Rect::new(sw / 2.0 - 210.0, sh / 2.0 - 95.0, 420.0, 190.0);
            ui.panel(r);
            ui.text_centered(
                &tw("{name} wins!", &[("name", &m.names[over.winner])]),
                sw / 2.0,
                r.y + 18.0,
                28.0,
                GOLD,
                true,
            );
            ui.text_centered(
                crate::i18n::reason(over.reason),
                sw / 2.0,
                r.y + 60.0,
                15.0,
                CREAM,
                false,
            );
            let score = format!("{}  {} – {}  {}", m.names[0], m.wins[0], m.wins[1], m.names[1]);
            ui.text_centered(&score, sw / 2.0, r.y + 88.0, 16.0, CREAM, true);
            if all_computers {
                ui.text_centered(
                    t("Next game starts in a moment…"),
                    sw / 2.0,
                    r.y + 130.0,
                    14.0,
                    MUTED,
                    false,
                );
            } else {
                if ui.primary_button(Rect::new(r.x + 30.0, r.y + 128.0, 170.0, 38.0), t("Rematch"))
                {
                    m.rematch();
                }
                if ui.button(Rect::new(r.x + 220.0, r.y + 128.0, 170.0, 38.0), t("Main menu"), true)
                {
                    action = HudAction::MainMenu;
                }
            }
        }

        // Pause menu (online the match keeps running behind it).
        if self.paused {
            let online = m.is_online();
            draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.45));
            let r = Rect::new(sw / 2.0 - 160.0, sh / 2.0 - 172.0, 320.0, 344.0);
            ui.panel(r);
            ui.text_centered(
                if online { t("Menu") } else { t("Paused") },
                sw / 2.0,
                r.y + 14.0,
                24.0,
                GOLD,
                true,
            );
            if online {
                ui.text_centered(
                    t("The game goes on while this is open."),
                    sw / 2.0,
                    r.y + 44.0,
                    13.0,
                    MUTED,
                    false,
                );
            }
            let bx = r.x + 30.0;
            let bw = r.w - 60.0;
            if ui.primary_button(Rect::new(bx, r.y + 66.0, bw, 38.0), t("Resume")) {
                self.paused = false;
                self.confirm_leave = false;
            }
            if ui.button(Rect::new(bx, r.y + 116.0, bw, 34.0), t("Settings"), true) {
                action = HudAction::Settings;
            }
            let help = if self.show_help { t("Hide controls") } else { t("Controls") };
            if ui.button(Rect::new(bx, r.y + 160.0, bw, 34.0), help, true) {
                self.show_help = !self.show_help;
                self.paused = false;
            }
            let my_turn = m.current_controller() == Controller::Human;
            let can_resign = m.game.game_over().is_none() && (online || my_turn);
            if ui.button(Rect::new(bx, r.y + 204.0, bw, 34.0), t("Resign this game"), can_resign) {
                m.resign();
                self.paused = false;
            }
            let leave = if online { t("Leave to the lounge") } else { t("Main menu") };
            if self.confirm_leave {
                ui.text_centered(
                    t("Leaving now counts as a loss."),
                    sw / 2.0,
                    r.y + 250.0,
                    13.0,
                    ALERT,
                    false,
                );
                if ui.button(Rect::new(bx, r.y + 272.0, bw / 2.0 - 5.0, 34.0), t("Leave"), true) {
                    self.paused = false;
                    action = if online { HudAction::Lounge } else { HudAction::MainMenu };
                }
                if ui.button(
                    Rect::new(bx + bw / 2.0 + 5.0, r.y + 272.0, bw / 2.0 - 5.0, 34.0),
                    t("Stay"),
                    true,
                ) {
                    self.confirm_leave = false;
                }
            } else if ui.button(Rect::new(bx, r.y + 256.0, bw, 34.0), leave, true) {
                if online && m.game.game_over().is_none() {
                    self.confirm_leave = true;
                } else {
                    self.paused = false;
                    action = if online { HudAction::Lounge } else { HudAction::MainMenu };
                }
            }
        }
        action
    }
}

const CHAT_W: f32 = 300.0;
const CHAT_H: f32 = 190.0;

impl Hud {
    /// The chat with the opponent (online matches).
    fn draw_chat(&mut self, ui: &mut Ui, m: &mut Match) {
        let sh = crate::ui::vh();
        let Some(o) = m.online.as_mut() else { return };
        if !self.chat_open {
            let label = if o.unread > 0 {
                tw("Chat ({n})", &[("n", &o.unread)])
            } else {
                t("Chat").to_string()
            };
            if ui.button(Rect::new(12.0, sh - BOTTOM_BAR - 40.0, 110.0, 32.0), &label, true) {
                self.chat_open = true;
            }
            return;
        }
        o.unread = 0;
        let r = Rect::new(12.0, sh - BOTTOM_BAR - CHAT_H - 8.0, CHAT_W, CHAT_H);
        ui.panel(r);
        ui.text(t("Chat"), r.x + 10.0, r.y + 7.0, 14.0, GOLD, true);
        if ui.button(Rect::new(r.x + r.w - 30.0, r.y + 5.0, 24.0, 20.0), "–", true) {
            self.chat_open = false;
        }
        let log = Rect::new(r.x + 6.0, r.y + 28.0, r.w - 12.0, r.h - 28.0 - 40.0);
        let me = m.names[o.seat].clone();
        self.chat_scroll = ui.chat_log(log, &o.chat, Some(&me), self.chat_scroll);
        let input = Rect::new(r.x + 6.0, r.y + r.h - 36.0, r.w - 12.0, 30.0);
        let sent =
            ui.text_input(CHAT_FIELD, input, &mut self.chat_input, snook_proto::MAX_CHAT_LEN);
        if self.chat_input.is_empty() && ui.focus != Some(CHAT_FIELD) {
            ui.text(
                t("Click to chat, Enter to send"),
                input.x + 8.0,
                input.y + 8.0,
                13.0,
                MUTED,
                false,
            );
        }
        if sent {
            let text = std::mem::take(&mut self.chat_input);
            m.send_chat(&text);
            self.chat_scroll = 0;
            ui.focus = None;
        }
    }
}

const BANNER_TIME: f32 = 1.9;

impl Hud {
    /// A large t("Your turn") style message whenever the turn changes.
    fn draw_turn_banner(&mut self, ui: &mut Ui, m: &Match) {
        let dt = get_frame_time();
        let all_computers = m.controllers.iter().all(|c| matches!(c, Controller::Computer(_)));
        if let Phase::Turn { .. } = m.phase
            && !all_computers
        {
            let player = m.game.current_player();
            let key = (m.games_played, m.game.shots_taken(), player);
            if self.banner_key != Some(key) {
                let first = self.banner_key.is_none_or(|k| k.0 != key.0 || k.2 != key.2);
                self.banner_key = Some(key);
                // Only announce a change of player (or a new game), not every shot.
                if first {
                    self.banner = Some((turn_title(m), turn_subtitle(m), 0.0));
                }
            }
        }
        let Some((title, subtitle, age)) = &mut self.banner else { return };
        *age += dt;
        if *age > BANNER_TIME || self.paused {
            self.banner = None;
            return;
        }
        let t = *age;
        let alpha = (t / 0.18).min(1.0) * ((BANNER_TIME - t) / 0.45).min(1.0);
        let slide = (1.0 - (t / 0.25).min(1.0)).powi(2) * 30.0;
        let sw = crate::ui::vw();
        let y = crate::ui::vh() * 0.21 + slide;
        let size = 40.0;
        let w = ui.text_size(title, size, true).x.max(ui.text_size(subtitle, 17.0, false).x) + 80.0;
        let r = Rect::new((sw - w) / 2.0, y - 14.0, w, 92.0);
        crate::ui::rrect(r, 14.0, Color::new(0.0, 0.0, 0.0, 0.55 * alpha));
        let gold = Color::new(GOLD.r, GOLD.g, GOLD.b, alpha);
        draw_line(
            r.x + 30.0,
            r.y + 64.0,
            r.x + r.w - 30.0,
            r.y + 64.0,
            1.0,
            Color::new(GOLD.r, GOLD.g, GOLD.b, 0.4 * alpha),
        );
        ui.text_centered(
            title,
            sw / 2.0 + 2.0,
            y + 2.0,
            size,
            Color::new(0.0, 0.0, 0.0, 0.5 * alpha),
            true,
        );
        ui.text_centered(title, sw / 2.0, y, size, gold, true);
        ui.text_centered(
            subtitle,
            sw / 2.0,
            y + 56.0,
            17.0,
            Color::new(CREAM.r, CREAM.g, CREAM.b, alpha),
            false,
        );
    }
}

fn turn_title(m: &Match) -> String {
    let player = m.game.current_player();
    let name = &m.names[player];
    let mine = match &m.online {
        Some(o) => o.seat == player,
        None => {
            m.controllers[player] == Controller::Human
                && m.controllers.iter().filter(|c| **c == Controller::Human).count() == 1
        }
    };
    if mine { t("Your turn").into() } else { tw("{name}'s turn", &[("name", name)]) }
}

fn turn_subtitle(m: &Match) -> String {
    let player = m.game.current_player();
    if m.game.is_break() {
        return t("Break the rack").into();
    }
    if m.game.needs_called_pocket() {
        return t("On the 8-ball: call a pocket").into();
    }
    let group = match m.game.group_of(player) {
        Some(Group::Solids) => t("Solids (1–7)"),
        Some(Group::Stripes) => t("Stripes (9–15)"),
        None => t("Open table"),
    };
    match m.game.ball_in_hand() {
        Some(_) => tw("Ball in hand  ·  {group}", &[("group", &group)]),
        None => group.into(),
    }
}

/// Shown while the cue is being drawn back (for any player, so spectators see it).
fn draw_power_meter(ui: &mut Ui, m: &Match) {
    // The player's own power is shown on the stroke bar.
    if !m.aim.charging || !matches!(m.phase, Phase::Turn { .. }) || m.is_human_turn() {
        return;
    }
    let sw = crate::ui::vw();
    let sh = crate::ui::vh();
    let w = (sw * 0.32).clamp(260.0, 420.0);
    let r = Rect::new((sw - w) / 2.0, sh - 78.0, w, 22.0);
    crate::ui::rrect(
        Rect::new(r.x - 10.0, r.y - 28.0, r.w + 20.0, r.h + 44.0),
        12.0,
        Color::new(0.0, 0.0, 0.0, 0.55),
    );
    crate::ui::rrect(r, 11.0, Color::new(0.06, 0.04, 0.03, 1.0));
    let p = m.aim.power.clamp(0.0, 1.0);
    // Filled with a green → yellow → red ramp, in slices.
    let slices = 40;
    let filled = (p * slices as f32).ceil() as usize;
    for k in 0..filled {
        let t = k as f32 / slices as f32;
        let color = if t < 0.5 {
            Color::new(0.25 + 1.3 * t, 0.85, 0.3, 1.0)
        } else {
            Color::new(0.95, 0.85 - 1.2 * (t - 0.5), 0.25, 1.0)
        };
        let x0 = r.x + 3.0 + t * (r.w - 6.0);
        let x1 = (r.x + 3.0 + ((k + 1) as f32 / slices as f32).min(p) * (r.w - 6.0)).max(x0);
        draw_rectangle(x0, r.y + 4.0, x1 - x0, r.h - 8.0, color);
    }
    for k in 1..4 {
        let x = r.x + r.w * k as f32 / 4.0;
        draw_line(x, r.y + 3.0, x, r.y + r.h - 3.0, 1.0, Color::new(0.0, 0.0, 0.0, 0.5));
    }
    crate::ui::rrect_lines(r, 11.0, 1.2, BORDER);
    ui.text(
        &tw("Power {pct}%", &[("pct", &format!("{:.0}", p * 100.0))]),
        r.x,
        r.y - 22.0,
        15.0,
        CREAM,
        true,
    );
    if m.is_human_turn() {
        let hint = t("release Space to shoot · Esc cancels");
        let hw = ui.text_size(hint, 13.0, false).x;
        ui.text(hint, r.x + r.w - hw, r.y - 20.0, 13.0, MUTED, false);
    }
}

impl Hud {
    /// The bottom bar: game log, the power bar and the cue settings.
    fn draw_bottom_bar(&mut self, ui: &mut Ui, m: &mut Match) {
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        let bar = Rect::new(0.0, sh - BOTTOM_BAR, sw, BOTTOM_BAR);
        draw_bar(bar, false);
        let my_turn = m.is_human_turn() && !self.paused;

        // Log on the left.
        let log_w = (sw * 0.30).clamp(230.0, 400.0);
        draw_log(ui, m, Rect::new(8.0, bar.y + 8.0, log_w, bar.h - 16.0));

        // Cue setting buttons on the right; each opens a small panel above.
        let bw = 92.0;
        let pops = [Pop::Spin, Pop::Elevation, Pop::FineAim];
        let bx0 = sw - 10.0 - pops.len() as f32 * (bw + 6.0) + 6.0;
        for (k, pop) in pops.into_iter().enumerate() {
            let r = Rect::new(bx0 + k as f32 * (bw + 6.0), bar.y + 10.0, bw, bar.h - 20.0);
            let open = self.popover == Some(pop);
            if ui.toggle(r, open) {
                self.popover = if open { None } else { Some(pop) };
            }
            draw_pop_icon(ui, m, pop, r);
        }

        // Power bar in the middle: press, drag right for more power, release
        // to shoot. Right click or Esc cancels.
        let px0 = log_w + 30.0;
        let px1 = bx0 - 24.0;
        let track = Rect::new(px0, bar.y + bar.h / 2.0 - 9.0, (px1 - px0).max(120.0), 18.0);
        if my_turn && !m.is_placing_cue_ball() {
            self.stroke(ui, m, track);
        } else {
            self.stroking = false;
            draw_power_track(
                ui,
                track,
                if m.aim.charging { m.aim.power } else { 0.0 },
                false,
                false,
            );
        }

        // The open panel.
        if let Some(pop) = self.popover {
            if !my_turn {
                return;
            }
            let w = 250.0;
            let h = match pop {
                Pop::Spin => 186.0,
                Pop::Elevation => 92.0,
                Pop::FineAim => 96.0,
            };
            let r = Rect::new(sw - w - 10.0, bar.y - h - 8.0, w, h);
            ui.panel(r);
            match pop {
                Pop::Spin => self.spin_panel(ui, m, r),
                Pop::Elevation => elevation_panel(ui, m, r),
                Pop::FineAim => self.fine_aim_panel(ui, m, r),
            }
        }
    }

    fn stroke(&mut self, ui: &mut Ui, m: &mut Match, track: Rect) {
        let hit = Rect::new(track.x - 10.0, track.y - 14.0, track.w + 20.0, track.h + 28.0);
        let drag = ui.drag_area(90, hit);
        match drag {
            crate::ui::Drag::Held(_)
                if self.stroke_cancelled
                    || is_mouse_button_pressed(MouseButton::Right)
                    || is_key_pressed(KeyCode::Escape) =>
            {
                self.stroke_cancelled = true;
                self.stroking = false;
                m.aim.charging = false;
                m.aim.power = 0.0;
            }
            crate::ui::Drag::Held(p) => {
                if m.game.needs_called_pocket() && m.called_pocket.is_none() {
                    if !self.stroking {
                        m.notify(t("Call a pocket for the 8-ball first: click a pocket."), true);
                    }
                    self.stroking = true;
                } else {
                    self.stroking = true;
                    m.aim.charging = true;
                    m.aim.power = ((p.x - track.x) / track.w).clamp(0.0, 1.0);
                }
            }
            crate::ui::Drag::Released(_) => {
                if self.stroking && m.aim.charging {
                    if m.aim.power > 0.02 {
                        m.shoot();
                    } else {
                        m.aim.charging = false;
                        m.aim.power = 0.0;
                    }
                }
                self.stroking = false;
                self.stroke_cancelled = false;
            }
            _ => {
                self.stroke_cancelled = false;
                self.stroking = false;
            }
        }
        let hot = matches!(drag, crate::ui::Drag::Held(_) | crate::ui::Drag::Hover);
        let p = if m.aim.charging { m.aim.power } else { 0.0 };
        draw_power_track(ui, track, p, hot, true);
        let hint = if self.stroking && m.aim.charging {
            Some(t("Right click or Esc cancels"))
        } else if hot {
            Some(t("Drag right for more power, release to shoot"))
        } else {
            None
        };
        if let Some(hint) = hint {
            let w = ui.text_size(hint, 13.0, false).x + 20.0;
            let r = Rect::new(track.x + (track.w - w) / 2.0, track.y - 40.0, w, 24.0);
            crate::ui::rrect(r, 12.0, Color::new(0.0, 0.0, 0.0, 0.8));
            ui.text(hint, r.x + 10.0, r.y + 5.0, 13.0, CREAM, false);
        }
    }

    fn spin_panel(&mut self, ui: &mut Ui, m: &mut Match, r: Rect) {
        ui.text(t("Cue ball contact"), r.x + 12.0, r.y + 8.0, 14.0, GOLD, true);
        let mut tip = (m.aim.tip.x as f32, m.aim.tip.y as f32);
        ui.spin_ball(1, vec2(r.x + r.w / 2.0, r.y + 92.0), 56.0, &mut tip);
        m.aim.tip = ::glam::DVec2::new(tip.0 as f64, tip.1 as f64);
        let label = match (tip.1 > 0.15, tip.1 < -0.15) {
            (true, _) => t("follow"),
            (_, true) => t("draw"),
            _ => t("stun"),
        };
        let side = if tip.0 > 0.15 {
            t(" · right english")
        } else if tip.0 < -0.15 {
            t(" · left english")
        } else {
            ""
        };
        if ui.button(Rect::new(r.x + 12.0, r.y + r.h - 30.0, 62.0, 22.0), t("Center"), true) {
            m.aim.tip = ::glam::DVec2::ZERO;
        }
        ui.text(&format!("{label}{side}"), r.x + 82.0, r.y + r.h - 27.0, 13.0, MUTED, false);
    }

    fn fine_aim_panel(&mut self, ui: &mut Ui, m: &mut Match, r: Rect) {
        ui.text(t("Fine aim"), r.x + 12.0, r.y + 8.0, 14.0, GOLD, true);
        let deg = (m.aim.yaw.to_degrees()).rem_euclid(360.0);
        let deg_text = format!("{deg:.1}°");
        let dw = ui.text_size(&deg_text, 13.0, false).x;
        ui.text(&deg_text, r.x + r.w - 12.0 - dw, r.y + 10.0, 13.0, MUTED, false);
        let y = r.y + 46.0;
        let dt = get_frame_time() as f64;
        let rate = 0.9f64.to_radians() * self.aim_sensitivity as f64;
        if ui.hold_button(80, Rect::new(r.x + 12.0, y, 32.0, 32.0), "◀") {
            m.aim.yaw += rate * dt;
        }
        if ui.hold_button(81, Rect::new(r.x + r.w - 44.0, y, 32.0, 32.0), "▶") {
            m.aim.yaw -= rate * dt;
        }
        let dial = Rect::new(r.x + 50.0, y, r.w - 100.0, 32.0);
        let drag = ui.drag_area(82, dial);
        match drag {
            crate::ui::Drag::Held(p) => {
                if let Some(last) = self.fine_last {
                    m.aim.yaw -= ((p.x - last) as f64) * 0.00025 * self.aim_sensitivity as f64;
                }
                self.fine_last = Some(p.x);
            }
            _ => self.fine_last = None,
        }
        crate::ui::rrect(dial, 6.0, Color::new(0.05, 0.035, 0.025, 1.0));
        let spacing = 10.0;
        let offset = ((m.aim.yaw * 4000.0) as f32).rem_euclid(spacing);
        let mut tx = dial.x + offset;
        while tx < dial.x + dial.w {
            let edge = ((tx - dial.x) / dial.w - 0.5).abs() * 2.0;
            let alpha = 0.8 * (1.0 - edge * edge);
            draw_line(
                tx,
                dial.y + 6.0,
                tx,
                dial.y + dial.h - 6.0,
                1.5,
                Color::new(0.9, 0.8, 0.6, alpha),
            );
            tx += spacing;
        }
        let hot = matches!(drag, crate::ui::Drag::Held(_) | crate::ui::Drag::Hover);
        crate::ui::rrect_lines(dial, 6.0, 1.2, if hot { GOLD } else { crate::ui::BORDER });
        m.aim.yaw = m.aim.yaw.rem_euclid(std::f64::consts::TAU);
    }
}

fn elevation_panel(ui: &mut Ui, m: &mut Match, r: Rect) {
    let max = m.game.params().max_elevation.to_degrees() as f32;
    let mut elevation = m.aim.elevation.to_degrees() as f32;
    ui.text(
        &tw("Elevation {deg}°", &[("deg", &format!("{elevation:.0}"))]),
        r.x + 12.0,
        r.y + 8.0,
        14.0,
        GOLD,
        true,
    );
    ui.text(t("W / S (jump & massé)"), r.x + 12.0, r.y + 30.0, 12.0, MUTED, false);
    if ui.slider(2, Rect::new(r.x + 14.0, r.y + 58.0, r.w - 28.0, 18.0), &mut elevation, 0.0, max) {
        m.aim.elevation = (elevation as f64).to_radians();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pop {
    Spin,
    Elevation,
    FineAim,
}

/// A small picture on each cue-setting button showing the current value.
fn draw_pop_icon(ui: &mut Ui, m: &Match, pop: Pop, r: Rect) {
    let c = vec2(r.x + r.w / 2.0, r.y + r.h / 2.0 - 7.0);
    let label = match pop {
        Pop::Spin => {
            let rad = 15.0;
            draw_circle(c.x, c.y, rad, Color::new(0.95, 0.93, 0.88, 1.0));
            draw_circle_lines(c.x, c.y, rad, 1.0, Color::new(0.4, 0.33, 0.25, 1.0));
            let d = c + vec2(m.aim.tip.x as f32, -m.aim.tip.y as f32) * rad * 0.85;
            draw_circle(d.x, d.y, 3.5, Color::new(0.8, 0.15, 0.15, 1.0));
            t("Spin")
        }
        Pop::Elevation => {
            let a = m.aim.elevation as f32;
            let base = c + vec2(14.0, 9.0);
            draw_line(
                c.x - 22.0,
                base.y + 3.0,
                c.x + 22.0,
                base.y + 3.0,
                1.5,
                Color::new(0.4, 0.7, 0.4, 1.0),
            );
            draw_circle(base.x, base.y - 3.0, 5.0, Color::new(0.95, 0.93, 0.88, 1.0));
            let tip = base + vec2(-7.0, -3.0);
            let butt = tip + vec2(-a.cos(), -a.sin()) * 30.0;
            draw_line(tip.x, tip.y, butt.x, butt.y, 2.5, Color::new(0.85, 0.7, 0.45, 1.0));
            t("Elevation")
        }
        Pop::FineAim => {
            for k in -3..=3 {
                let x = c.x + k as f32 * 6.0;
                let h = if k == 0 { 12.0 } else { 7.0 };
                draw_line(x, c.y - h, x, c.y + h, 1.5, if k == 0 { GOLD } else { MUTED });
            }
            t("Fine aim")
        }
    };
    let size = ui.fit(12.0, label, r.w - 8.0);
    ui.text_centered(label, r.x + r.w / 2.0, r.y + r.h - size - 6.0, size, CREAM, true);
}

fn draw_power_track(ui: &mut Ui, track: Rect, p: f32, hot: bool, enabled: bool) {
    crate::ui::rrect(track, 9.0, Color::new(0.04, 0.03, 0.02, 1.0));
    // Ramp from green to red underneath, faint; bright where filled.
    let slices = 40;
    for k in 0..slices {
        let t0 = k as f32 / slices as f32;
        let t1 = (k + 1) as f32 / slices as f32;
        let color = if t0 < 0.5 {
            Color::new(0.25 + 1.3 * t0, 0.85, 0.3, 1.0)
        } else {
            Color::new(0.95, 0.85 - 1.2 * (t0 - 0.5), 0.25, 1.0)
        };
        let a = if t0 < p {
            1.0
        } else if enabled {
            0.22
        } else {
            0.1
        };
        let x0 = track.x + 4.0 + t0 * (track.w - 8.0);
        let x1 = track.x + 4.0 + t1.min(if t0 < p { p } else { t1 }) * (track.w - 8.0);
        draw_rectangle(
            x0,
            track.y + 5.0,
            (x1 - x0).max(0.0),
            track.h - 10.0,
            Color::new(color.r, color.g, color.b, a),
        );
    }
    // The knob.
    let kx = track.x + 4.0 + p * (track.w - 8.0);
    let ky = track.y + track.h / 2.0;
    draw_circle(kx, ky + 1.5, 13.0, Color::new(0.0, 0.0, 0.0, 0.35));
    draw_circle(
        kx,
        ky,
        12.0,
        if enabled { Color::new(0.93, 0.92, 0.88, 1.0) } else { Color::new(0.5, 0.48, 0.45, 1.0) },
    );
    draw_circle(kx - 3.5, ky - 3.5, 4.0, Color::new(1.0, 1.0, 1.0, 0.7));
    crate::ui::rrect_lines(track, 9.0, 1.2, if hot { GOLD } else { crate::ui::BORDER });
    if p > 0.0 {
        let text = format!("{:.0}%", p * 100.0);
        ui.text(&text, track.x + track.w + 8.0, track.y + 1.0, 14.0, CREAM, true);
    }
}

/// A full-width bar at the top or bottom of the screen.
fn draw_bar(r: Rect, top: bool) {
    draw_rectangle(r.x, r.y, r.w, r.h, Color::new(0.045, 0.03, 0.02, 0.94));
    let edge = if top { r.y + r.h - 1.0 } else { r.y };
    draw_line(r.x, edge, r.x + r.w, edge, 1.5, Color::new(0.45, 0.31, 0.16, 0.9));
}

/// Choosing the pocket for the 8-ball: a little map of the table.
fn draw_call_panel(ui: &mut Ui, m: &mut Match) {
    let sh = crate::ui::vh();
    let w = 250.0;
    let h = 214.0;
    let r = Rect::new(12.0, (sh * 0.22).max(56.0), w, h);
    ui.panel(r);
    crate::ui::rrect_lines(r, 8.0, 2.0, GOLD);
    ui.text(t("Call the pocket for the 8-ball"), r.x + 12.0, r.y + 10.0, 14.0, GOLD, true);
    let map = Rect::new(r.x + 25.0, r.y + 42.0, w - 50.0, (w - 50.0) / 2.0);
    crate::ui::rrect(
        Rect::new(map.x - 8.0, map.y - 8.0, map.w + 16.0, map.h + 16.0),
        6.0,
        Color::new(0.42, 0.22, 0.09, 1.0),
    );
    draw_rectangle(map.x, map.y, map.w, map.h, Color::new(0.10, 0.45, 0.24, 1.0));
    // The 8-ball's position on the map.
    let spec = m.game.table().spec;
    let to_map = |p: ::glam::DVec2| {
        vec2(
            map.x + ((p.x / spec.length + 0.5) as f32) * map.w,
            map.y + ((0.5 - p.y / spec.width) as f32) * map.h,
        )
    };
    let eight = to_map(m.game.ball(BallId::EIGHT).xy());
    draw_circle(eight.x, eight.y, 5.0, BLACK);
    draw_circle_lines(eight.x, eight.y, 5.0, 1.0, Color::new(1.0, 1.0, 1.0, 0.6));
    let mut chosen = None;
    for id in snook_core::PocketId::ALL {
        let c = to_map(m.game.table().pocket(id).mouth_center);
        let hover = ui.mouse().distance(c) < 13.0;
        let selected = m.called_pocket == Some(id);
        draw_circle(
            c.x,
            c.y,
            11.0,
            if selected { GOLD } else { Color::new(0.03, 0.03, 0.03, 1.0) },
        );
        draw_circle_lines(
            c.x,
            c.y,
            11.0,
            2.0,
            if hover || selected { GOLD } else { Color::new(0.5, 0.4, 0.3, 1.0) },
        );
        if hover && is_mouse_button_released(MouseButton::Left) {
            chosen = Some(id);
        }
    }
    ui.block(map);
    if let Some(id) = chosen {
        m.called_pocket = Some(id);
        let name = m.names[m.game.current_player()].clone();
        m.notify(
            tw(
                "{name} calls the 8-ball in the {pocket} pocket.",
                &[("name", &name), ("pocket", &crate::i18n::pocket(id))],
            ),
            false,
        );
    }
    let y = map.y + map.h + 16.0;
    match m.called_pocket {
        Some(id) => ui.text(
            &tw("Called: {pocket}", &[("pocket", &crate::i18n::pocket(id))]),
            r.x + 12.0,
            y,
            13.0,
            CREAM,
            true,
        ),
        None => ui.text(
            t("Click a pocket on the table, or pick one here:"),
            r.x + 12.0,
            y,
            12.0,
            MUTED,
            false,
        ),
    }
    for (k, line) in ui
        .wrap(t("Sinking the 8 in any other pocket loses the game."), w - 24.0, 12.0)
        .iter()
        .enumerate()
    {
        ui.text(line, r.x + 12.0, y + 20.0 + k as f32 * 15.0, 12.0, ALERT, false);
    }
}

const TOP_BAR: f32 = 54.0;
const BOTTOM_BAR: f32 = 78.0;
const CARD_H: f32 = 44.0;

/// The game log inside the bottom bar: the last few things that happened.
fn draw_log(ui: &mut Ui, m: &Match, r: Rect) {
    if m.log.is_empty() {
        return;
    }
    ui.block(r);
    crate::ui::rrect(r, 6.0, Color::new(0.0, 0.0, 0.0, 0.35));
    let lines_fit = ((r.h - 6.0) / 17.0).floor().max(1.0) as usize;
    let start = m.log.len().saturating_sub(lines_fit);
    let lines = &m.log[start..];
    for (k, (text, important)) in lines.iter().enumerate() {
        let newest = k + 1 == lines.len();
        let base = if *important {
            ALERT
        } else if newest {
            CREAM
        } else {
            MUTED
        };
        let text = ui.ellipsize(text, r.w - 16.0, 13.0);
        ui.text(&text, r.x + 8.0, r.y + 4.0 + k as f32 * 17.0, 13.0, base, newest);
    }
}
