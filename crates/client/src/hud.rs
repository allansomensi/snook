//! In-game interface: scoreboard, shot panel, notices, help, game over and pause.

use macroquad::prelude::*;

#[allow(unused_imports)]
use crate::paint::{draw_circle, draw_circle_lines, draw_line, draw_rectangle, draw_triangle};

use crate::i18n::{t, tw};
use snook_core::{BallId, BallInHand, Group};

use crate::game::{Controller, Match, Phase};
use crate::icons::Icon;
use crate::textures::ball_color;
use crate::ui::{ALERT, BORDER, CREAM, FeedKind, FeedLine, GOLD, MUTED, Ui};

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
    /// The power the player last chose (kept from one turn to the next).
    power: f32,
    power_key: Option<(u32, u32)>,
    panel_key: Option<(u32, u32)>,
    /// Camera zoom, 0 (far) to 1 (close): set by the caller, changed by the zoom bar.
    pub zoom: f32,
    zoom_drag: bool,
    /// The reaction picker is open.
    reactions_open: bool,
    /// Offline: the game history panel is open (it starts folded).
    history_open: bool,
    /// Set by the caller: the camera is already in its default place.
    pub camera_default: bool,
}

/// What the player asked for through the HUD.
pub enum HudAction {
    None,
    MainMenu,
    /// Online: back to the lounge.
    Lounge,
    Settings,
    /// Back to the overhead view of the whole table.
    ResetCamera,
    /// The zoom bar moved (0 far … 1 close).
    Zoom(f32),
}

impl Default for Hud {
    fn default() -> Self {
        Hud {
            paused: false,
            show_help: false,
            chat_input: String::new(),
            // Development aid: SNOOK_CHAT_CLOSED starts with the chat folded.
            chat_open: !crate::dev::flag("SNOOK_CHAT_CLOSED"),
            chat_scroll: 0,
            confirm_leave: false,
            banner_key: None,
            banner: None,
            top_view: false,
            aim_sensitivity: 1.0,
            fine_last: None,
            stroking: false,
            popover: None,
            power: 0.5,
            power_key: None,
            panel_key: None,
            zoom: 0.5,
            zoom_drag: false,
            // Development aid: SNOOK_OPEN_REACTIONS opens the picker.
            reactions_open: crate::dev::flag("SNOOK_OPEN_REACTIONS"),
            camera_default: true,
            history_open: false,
        }
    }
}

impl Hud {
    /// The controls are only listed from the menu: never on the table.
    pub fn new(_show_help: bool) -> Self {
        Hud::default()
    }
}

const CHAT_FIELD: u64 = 40;

fn ball_icon(ui: &Ui, id: BallId, x: f32, y: f32, size: f32) {
    let c = ball_color(id);
    let color = Color::from_rgba(c[0], c[1], c[2], 255);
    let ivory = Color::from_rgba(246, 242, 228, 255);
    let r = size / 2.0;
    let (cx, cy) = (x + r, y + r);
    if id.0 > 8 {
        // Stripe: ivory ball with a coloured band, clipped to the circle.
        draw_circle(cx, cy, r, ivory);
        // The band is the part of the disc with |y| < band: a convex shape
        // (two arcs joined by straight edges), filled as a fan.
        let band = size * 0.27;
        let a = (band / r).clamp(0.0, 1.0).asin();
        let mut pts: Vec<Vec2> = Vec::with_capacity(20);
        for k in 0..=8 {
            let t = -a + 2.0 * a * k as f32 / 8.0;
            pts.push(vec2(cx + r * t.cos(), cy + r * t.sin()));
        }
        for k in 0..=8 {
            let t = std::f32::consts::PI - a + 2.0 * a * k as f32 / 8.0;
            pts.push(vec2(cx + r * t.cos(), cy + r * t.sin()));
        }
        let center = vec2(cx, cy);
        for i in 0..pts.len() {
            draw_triangle(center, pts[i], pts[(i + 1) % pts.len()], color);
        }
    } else {
        draw_circle(cx, cy, r, color);
    }
    draw_circle(cx, cy, r * 0.62, ivory);
    draw_circle_lines(cx, cy, r, 1.2, Color::new(0.0, 0.0, 0.0, 0.7));
    let label = id.0.to_string();
    let fs = if id.0 >= 10 { size * 0.44 } else { size * 0.56 };
    let w = ui.text_size(&label, fs, true).x;
    ui.text(&label, cx - w / 2.0, cy - fs * 0.55, fs, BLACK, true);
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
    let reserve = if m.online.is_some() || m.clock.is_some() { 96.0 } else { 40.0 };
    match &m.online {
        Some(o) => {
            let color = crate::ui::tier_color(snook_proto::Tier::of(o.ratings[seat]));
            ui.name_with_flag(
                &m.names[seat],
                &o.countries[seat],
                color,
                x + 10.0,
                y + 4.0,
                15.0,
                w - reserve,
            );
        }
        None => {
            let name = ui.ellipsize(&m.names[seat], w - reserve, 15.0);
            ui.text(&name, x + 10.0, y + 4.0, 15.0, if active { GOLD } else { CREAM }, true);
        }
    }
    let wins = m.wins[seat].to_string();
    let ww = ui.text_size(&wins, 18.0, true).x;
    ui.text(&wins, x + w - ww - 10.0, y + 3.0, 18.0, CREAM, true);
    if let Some(o) = &m.online {
        let rating = o.ratings[seat].to_string();
        let rw = ui.text_size(&rating, 12.0, false).x;
        ui.text(&rating, x + w - ww - rw - 20.0, y + 7.0, 12.0, MUTED, false);
    }
    if let Some((left, total)) = m.shot_clock()
        && active
        && matches!(m.phase, Phase::Turn { .. })
        && m.replay.is_none()
    {
        // Shot clock along the bottom of the card.
        let frac = (left / total.max(1.0)).clamp(0.0, 1.0);
        let color = if left < 10.0 { ALERT } else { GOLD };
        crate::ui::rrect(Rect::new(x + 6.0, y + CARD_H - 4.0, (w - 12.0) * frac, 2.5), 1.2, color);
        let secs = format!("{:.0}s", left.ceil());
        let sw_ = ui.text_size(&secs, 13.0, true).x;
        ui.text(&secs, x + w - sw_ - 10.0, y + 24.0, 13.0, color, true);
    }
    let ry = y + 23.0;
    match m.game.group_of(seat) {
        Some(group) => {
            let remaining = m.game.remaining(group);
            if remaining.is_empty() {
                ui.text(t("On the 8-ball"), x + 10.0, ry + 1.0, 13.0, GOLD, false);
                let tw_ = ui.text_size(t("On the 8-ball"), 13.0, false).x;
                ball_icon(ui, BallId::EIGHT, x + 16.0 + tw_, ry - 2.0, 20.0);
            } else {
                for (k, id) in remaining.into_iter().enumerate() {
                    ball_icon(ui, id, x + 10.0 + k as f32 * 22.0, ry - 2.0, 20.0);
                }
            }
        }
        None => ui.text(t("Open table"), x + 10.0, ry + 1.0, 13.0, MUTED, false),
    }
}

fn status_line(m: &Match) -> Option<String> {
    let name = &m.names[m.game.current_player()];
    match &m.phase {
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
            if *placing_cue_ball || m.game.ball_in_hand().is_some() {
                let zone = if m.game.ball_in_hand() == Some(BallInHand::Kitchen) {
                    t(" behind the line")
                } else {
                    ""
                };
                return Some(tw(
                    "Ball in hand: drag the cue ball{zone} to move it",
                    &[("zone", &zone)],
                ));
            }
            if m.game.is_break() {
                return Some(tw("{name} to break", &[("name", name)]));
            }
            if let Some(pocket) = m.game.called_pocket_of(m.game.current_player())
                && m.game.needs_called_pocket()
            {
                return Some(tw(
                    "{name} is on the 8: called pocket {pocket}",
                    &[("name", name), ("pocket", &crate::i18n::pocket(pocket))],
                ));
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
                // Official rule: the break never decides the groups; the
                // first ball made after it does.
                None => t("open table: the next ball you make sets your group"),
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
            if is_key_pressed(KeyCode::Escape) && self.reactions_open {
                self.reactions_open = false;
            } else if is_key_pressed(KeyCode::Escape) && !m.aim.charging {
                self.paused = !self.paused;
                self.confirm_leave = false;
            }
            if is_key_pressed(KeyCode::H) || is_key_pressed(KeyCode::F1) {
                // The list of controls, in the menu.
                self.show_help = !(self.paused && self.show_help);
                self.paused = self.show_help;
            }
            if !self.paused {
                self.show_help = false;
            }
        }

        if !ui.wants_keyboard && is_key_pressed(KeyCode::R) {
            if m.replay.is_some() {
                m.stop_replay();
            } else {
                m.start_replay();
            }
        }
        // Top bar: menu and camera on the left, the players in the middle.
        draw_bar(Rect::new(0.0, 0.0, sw, TOP_BAR), true);
        // Cards centred, but never under the buttons on the left.
        let buttons_w = ui.text_size(t("Menu"), 13.0, true).x + 22.0 + 2.0 * 36.0 + 30.0;
        let room = sw - 2.0 * buttons_w;
        let card_w = ((room - 10.0) / 2.0).clamp(150.0, 270.0);
        let total = card_w * 2.0 + 10.0;
        let x0 = ((sw - total) / 2.0).max(buttons_w).min(sw - total - 8.0);
        player_card(ui, m, 0, x0, 6.0, card_w);
        player_card(ui, m, 1, x0 + card_w + 10.0, 6.0, card_w);
        // How many are watching: an eye and the number, by the cards.
        let eye = vec2(x0 + total + 22.0, 27.0);
        if let Some(o) = &m.online
            && (o.spectators > 0 || o.spectator)
        {
            let n = o.spectators.to_string();
            let tip =
                Rect::new(eye.x - 14.0, eye.y - 14.0, 28.0 + ui.text_size(&n, 13.0, true).x, 28.0);
            crate::icons::draw(Icon::Eye, eye, 20.0, MUTED);
            ui.text(&n, eye.x + 14.0, eye.y - 8.0, 13.0, MUTED, true);
            if ui.hovering(tip) {
                ui.tooltip(&tw("{n} watching", &[("n", &n)]), tip);
            }
        }
        draw_reactions(m, [x0 + 34.0, x0 + total - 34.0], eye);
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

        // Cue panel (spin, elevation, fine aim) and the power stroke bar:
        // everything can be played with the mouse alone.
        if m.is_spectating() {
            if self.draw_spectator_bar(ui, m) {
                action = HudAction::Lounge;
            }
        } else {
            self.draw_bottom_bar(ui, m);
        }

        // Menu, the overhead view and the replay, in the top bar.
        if !self.paused {
            let label = if m.is_spectating() { t("Leave") } else { t("Menu") };
            let w = ui.text_size(t("Menu"), 13.0, true).x + 22.0;
            if ui.button(Rect::new(10.0, 12.0, w, 30.0), label, true) {
                // A spectator has nothing to pause: the menu just leaves.
                if m.is_spectating() {
                    action = HudAction::Lounge;
                } else {
                    self.paused = true;
                }
            }
            let mut x = 10.0 + w + 8.0;
            if ui.icon_button(
                Rect::new(x, 12.0, 30.0, 30.0),
                Icon::CameraReset,
                t("Reset the camera: top view, no zoom (C)"),
                !self.camera_default,
            ) {
                action = HudAction::ResetCamera;
            }
            x += 36.0;
            let can_replay = m.last_shot.is_some() && !m.shot_in_progress();
            let tip = if m.replay.is_some() {
                t("Stop the replay (R)")
            } else {
                t("Watch the last shot again (R)")
            };
            if ui.icon_button_ex(
                Rect::new(x, 12.0, 30.0, 30.0),
                Icon::Replay,
                tip,
                can_replay || m.replay.is_some(),
                true,
            ) {
                if m.replay.is_some() {
                    m.stop_replay();
                } else {
                    m.start_replay();
                }
            }
            if let Some(z) = self.zoom_bar(ui) {
                action = HudAction::Zoom(z);
            }
        }
        if m.replay.is_some() {
            let label = t("REPLAY");
            let w = ui.text_size(label, 14.0, true).x + 40.0;
            let r = Rect::new((sw - w) / 2.0, TOP_BAR + 8.0, w, 26.0);
            let pulse = 0.6 + 0.4 * (get_time() as f32 * 3.0).sin();
            crate::ui::rrect(r, 13.0, Color::new(0.0, 0.0, 0.0, 0.7));
            draw_circle(r.x + 14.0, r.y + 13.0, 4.5, Color::new(0.95, 0.3, 0.25, pulse));
            ui.text(label, r.x + 26.0, r.y + 6.0, 14.0, CREAM, true);
        }
        if self.show_help && self.paused {
            draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.45));
            let lines = [
                (t("Aim"), t("Drag the cue · click a spot")),
                (t("Fine aim"), t("Shift + drag · A / D")),
                (t("Quick aim"), t("Click a ball or a spot")),
                (t("Shoot"), t("Set the power, then press the green button")),
                (t("Spin"), t("Spin ball · ↑ ↓ · Q / E")),
                (t("Elevation"), t("W / S (jump & massé)")),
                (t("Camera"), t("Right-drag · Wheel · C")),
                (t("Replay"), t("R: watch the last shot again")),
                (t("Ball in hand"), t("Drag the cue ball")),
                (t("Menu / help"), t("Esc · H")),
            ];
            let h = 34.0 + lines.len() as f32 * 21.0 + 52.0;
            let label_w =
                lines.iter().map(|(a, _)| ui.text_size(a, 13.0, true).x).fold(0.0, f32::max);
            let value_w =
                lines.iter().map(|(_, b)| ui.text_size(b, 13.0, false).x).fold(0.0, f32::max);
            let w = (label_w + value_w + 50.0).min(sw - 20.0);
            let r = Rect::new((sw - w) / 2.0, (sh - h) / 2.0, w, h);
            ui.panel(r);
            ui.text(t("Controls"), r.x + 14.0, r.y + 10.0, 16.0, GOLD, true);
            for (k, (a, b)) in lines.iter().enumerate() {
                let yy = r.y + 38.0 + k as f32 * 21.0;
                ui.text(a, r.x + 14.0, yy, 13.0, CREAM, true);
                let room = r.w - label_w - 40.0;
                ui.text(&ui.ellipsize(b, room, 13.0), r.x + label_w + 28.0, yy, 13.0, MUTED, false);
            }
            if ui.button(
                Rect::new(r.x + r.w / 2.0 - 70.0, r.y + r.h - 44.0, 140.0, 32.0),
                t("Back"),
                true,
            ) {
                self.show_help = false;
            }
        }

        if m.is_online() && !m.is_spectating() {
            self.draw_chat(ui, m);
        } else if !m.is_online() {
            self.draw_history(ui, m);
        }

        // Game over.
        if let (Phase::GameOver, Some(over), true) = (&m.phase, m.game.game_over(), m.is_online()) {
            let o = m.online.as_ref().expect("online");
            let me = o.seat;
            let won = over.winner == me && !o.spectator;
            let r = Rect::new(sw / 2.0 - 220.0, sh / 2.0 - 110.0 - STATS_H / 2.0, 440.0, 220.0);
            ui.panel(r);
            draw_match_stats(ui, m, Rect::new(r.x, r.y + r.h + 8.0, r.w, STATS_H - 8.0));
            crate::ui::rrect_lines(r, 8.0, 2.0, if won { GOLD } else { BORDER });
            let title = if won {
                t("You win!").to_string()
            } else {
                tw("{name} wins", &[("name", &m.names[over.winner])])
            };
            ui.text_centered(&title, sw / 2.0, r.y + 16.0, 28.0, GOLD, true);
            ui.text_centered(
                &crate::i18n::result_line(
                    over.reason,
                    &m.names[over.winner],
                    &m.names[1 - over.winner],
                ),
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
            let note = if o.spectator {
                String::new()
            } else if o.tournament.is_some() {
                t("Tournament game: the next round starts from the lounge.").to_string()
            } else if o.opponent_left {
                t("Your opponent went back to the lounge.").to_string()
            } else if o.rematch_requested {
                tw("Waiting for {name}…", &[("name", &m.names[1 - me])])
            } else if o.rematch_offered {
                tw("{name} wants a rematch!", &[("name", &m.names[1 - me])])
            } else {
                String::new()
            };
            ui.text_centered(&note, sw / 2.0, r.y + 138.0, 14.0, GOLD, false);
            if o.spectator {
                ui.text_centered(
                    t("If they play again, you keep watching."),
                    sw / 2.0,
                    r.y + 138.0,
                    14.0,
                    MUTED,
                    false,
                );
                if ui.button(
                    Rect::new(sw / 2.0 - 100.0, r.y + 164.0, 200.0, 38.0),
                    t("Stop watching"),
                    true,
                ) {
                    action = HudAction::Lounge;
                }
            }
            if !o.spectator {
                let can_rematch =
                    !o.opponent_left && !o.rematch_requested && o.tournament.is_none();
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
            }
        } else if let (Phase::GameOver, Some(over)) = (&m.phase, m.game.game_over()) {
            let all_computers = m.controllers.iter().all(|c| matches!(c, Controller::Computer(_)));
            let r = Rect::new(sw / 2.0 - 210.0, sh / 2.0 - 95.0 - STATS_H / 2.0, 420.0, 190.0);
            ui.panel(r);
            draw_match_stats(ui, m, Rect::new(r.x, r.y + r.h + 8.0, r.w, STATS_H - 8.0));
            ui.text_centered(
                &tw("{name} wins!", &[("name", &m.names[over.winner])]),
                sw / 2.0,
                r.y + 18.0,
                28.0,
                GOLD,
                true,
            );
            ui.text_centered(
                &crate::i18n::result_line(
                    over.reason,
                    &m.names[over.winner],
                    &m.names[1 - over.winner],
                ),
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
        if self.paused && !self.show_help {
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
            if ui.button(Rect::new(bx, r.y + 160.0, bw, 34.0), t("Controls"), true) {
                self.show_help = true;
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

/// The chat (online) or the game history (offline): smaller on small
/// windows, so it never hides much of the table.
fn chat_w() -> f32 {
    (crate::ui::vw() * 0.3).clamp(220.0, 320.0)
}

fn chat_h() -> f32 {
    ((crate::ui::vh() - TOP_BAR - BOTTOM_BAR) * 0.38).clamp(120.0, 210.0)
}

impl Hud {
    /// A slim vertical zoom control on the right edge: almost invisible
    /// until the pointer comes near.
    fn zoom_bar(&mut self, ui: &mut Ui) -> Option<f32> {
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        let h = (sh * 0.28).clamp(120.0, 220.0);
        let track = Rect::new(sw - 26.0, (sh - h) / 2.0, 6.0, h);
        let zone = Rect::new(track.x - 16.0, track.y - 34.0, 40.0, h + 68.0);
        let near = ui.hovering(zone) || self.zoom_drag;
        let alpha = if near { 0.95 } else { 0.35 };
        let mut out = None;
        // + and − buttons.
        let plus = Rect::new(track.x - 9.0, track.y - 30.0, 24.0, 24.0);
        let minus = Rect::new(track.x - 9.0, track.y + h + 6.0, 24.0, 24.0);
        if near {
            if ui.icon_button_ex(plus, Icon::Plus, t("Zoom in"), true, true) {
                out = Some((self.zoom + 0.12).min(1.0));
            }
            if ui.icon_button_ex(minus, Icon::Minus, t("Zoom out"), true, true) {
                out = Some((self.zoom - 0.12).max(0.0));
            }
        } else {
            crate::icons::draw(
                Icon::Plus,
                vec2(plus.x + 12.0, plus.y + 12.0),
                10.0,
                Color::new(1.0, 1.0, 1.0, 0.3),
            );
            crate::icons::draw(
                Icon::Minus,
                vec2(minus.x + 12.0, minus.y + 12.0),
                10.0,
                Color::new(1.0, 1.0, 1.0, 0.3),
            );
        }
        let drag = ui.drag_area(95, Rect::new(track.x - 10.0, track.y, 26.0, h));
        self.zoom_drag = matches!(drag, crate::ui::Drag::Held(_));
        if let crate::ui::Drag::Held(p) | crate::ui::Drag::Released(p) = drag {
            out = Some((1.0 - (p.y - track.y) / h).clamp(0.0, 1.0));
        }
        crate::ui::rrect(track, 3.0, Color::new(0.0, 0.0, 0.0, 0.45 * alpha));
        let fill = h * self.zoom;
        crate::ui::rrect(
            Rect::new(track.x, track.y + h - fill, track.w, fill),
            3.0,
            Color::new(GOLD.r, GOLD.g, GOLD.b, 0.55 * alpha),
        );
        let ky = track.y + h - fill;
        draw_circle(
            track.x + 3.0,
            ky,
            if near { 8.0 } else { 5.5 },
            Color::new(0.93, 0.9, 0.84, alpha),
        );
        if let Some(z) = out {
            self.zoom = z;
        }
        out
    }

    /// The chat with the opponent (online matches), and the reactions.
    fn draw_chat(&mut self, ui: &mut Ui, m: &mut Match) {
        let sh = crate::ui::vh();
        let dt = get_frame_time();
        let Some(o) = m.online.as_mut() else { return };
        if !self.chat_open {
            let button = Rect::new(12.0, sh - BOTTOM_BAR - 40.0, 110.0, 32.0);
            let label = if o.unread > 0 {
                tw("Chat ({n})", &[("n", &o.unread)])
            } else {
                t("Chat").to_string()
            };
            if ui.button(button, &label, true) {
                self.chat_open = true;
            }
            if o.unread > 0 {
                // A dot that pulses while there is something new.
                let pulse = 0.5 + 0.5 * (get_time() as f32 * 5.0).sin();
                let c = vec2(button.x + button.w - 4.0, button.y + 4.0);
                draw_circle(c.x, c.y, 6.0 + pulse * 1.5, Color::new(0.0, 0.0, 0.0, 0.5));
                draw_circle(c.x, c.y, 5.0 + pulse * 1.5, GOLD);
            }
            // The newest message, for a few seconds, above the button.
            if let Some((line, age)) = &mut o.preview {
                *age += dt;
                let alpha = (*age / 0.2).min(1.0) * ((PREVIEW_TIME - *age) / 0.6).clamp(0.0, 1.0);
                if *age > PREVIEW_TIME {
                    o.preview = None;
                } else {
                    let who = line.from.as_ref().map(|(n, _)| format!("{n}: ")).unwrap_or_default();
                    let mut text = line.text.clone();
                    let max_w = 300.0;
                    let who_w = ui.text_size(&who, 13.0, true).x;
                    while !text.is_empty() && who_w + ui.text_size(&text, 13.0, false).x > max_w {
                        text.pop();
                        if ui.text_size(&text, 13.0, false).x + who_w <= max_w - 12.0 {
                            text.push('…');
                            break;
                        }
                    }
                    let w = who_w + ui.text_size(&text, 13.0, false).x + 20.0;
                    let r = Rect::new(button.x, button.y - 32.0, w, 26.0);
                    crate::ui::rrect(r, 13.0, Color::new(0.0, 0.0, 0.0, 0.7 * alpha));
                    ui.text(
                        &who,
                        r.x + 10.0,
                        r.y + 6.0,
                        13.0,
                        Color::new(GOLD.r, GOLD.g, GOLD.b, alpha),
                        true,
                    );
                    ui.text(
                        &text,
                        r.x + 10.0 + who_w,
                        r.y + 6.0,
                        13.0,
                        Color::new(CREAM.r, CREAM.g, CREAM.b, alpha),
                        false,
                    );
                }
            }
            let rb = Rect::new(button.x + button.w + 6.0, button.y, 32.0, 32.0);
            self.reaction_button(ui, m, rb);
            return;
        }
        o.unread = 0;
        o.preview = None;
        let r = Rect::new(12.0, sh - BOTTOM_BAR - chat_h() - 8.0, chat_w(), chat_h());
        ui.panel(r);
        ui.text(t("Chat"), r.x + 10.0, r.y + 7.0, 14.0, GOLD, true);
        if ui.button(Rect::new(r.x + r.w - 30.0, r.y + 5.0, 24.0, 20.0), "–", true) {
            self.chat_open = false;
        }
        let log = Rect::new(r.x + 6.0, r.y + 28.0, r.w - 12.0, r.h - 28.0 - 40.0);
        let me = m.names[o.seat].clone();
        self.chat_scroll = ui.chat_log(log, &o.feed, Some(&me), self.chat_scroll);
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
        self.reaction_button(ui, m, Rect::new(r.x + r.w + 6.0, r.y + r.h - 34.0, 32.0, 32.0));
    }

    /// Offline, what happened in the game so far, in the chat's place (no
    /// one to write to).
    fn draw_history(&mut self, ui: &mut Ui, m: &mut Match) {
        let sh = crate::ui::vh();
        if !self.history_open {
            let label = t("History");
            let w = ui.text_size(label, 13.0, true).x + 30.0;
            if ui.button(Rect::new(12.0, sh - BOTTOM_BAR - 40.0, w, 32.0), label, true) {
                self.history_open = true;
            }
            return;
        }
        let r = Rect::new(12.0, sh - BOTTOM_BAR - chat_h() - 8.0, chat_w(), chat_h());
        ui.panel(r);
        ui.text(t("Game history"), r.x + 10.0, r.y + 7.0, 14.0, GOLD, true);
        if ui.button(Rect::new(r.x + r.w - 30.0, r.y + 5.0, 24.0, 20.0), "–", true) {
            self.history_open = false;
        }
        let lines: Vec<FeedLine> = m
            .log
            .iter()
            .map(|(text, important)| FeedLine {
                from: None,
                text: text.clone(),
                kind: if *important { FeedKind::Alert } else { FeedKind::Event },
            })
            .collect();
        let log = Rect::new(r.x + 6.0, r.y + 28.0, r.w - 12.0, r.h - 34.0);
        if lines.is_empty() {
            ui.text(t("Nothing yet."), log.x + 6.0, log.y + 6.0, 13.0, MUTED, false);
        } else {
            self.chat_scroll = ui.chat_log(log, &lines, None, self.chat_scroll);
        }
    }

    /// The reactions button and, when open, the picker above it.
    fn reaction_button(&mut self, ui: &mut Ui, m: &mut Match, b: Rect) {
        let Some(o) = m.online.as_ref() else { return };
        ui.block(b);
        let hover = ui.hovering(b);
        crate::ui::rrect(
            b,
            7.0,
            if self.reactions_open || hover {
                Color::new(0.38, 0.25, 0.12, 0.95)
            } else {
                Color::new(0.07, 0.045, 0.03, 0.85)
            },
        );
        crate::ui::rrect_lines(
            b,
            7.0,
            1.2,
            if self.reactions_open || hover { GOLD } else { BORDER },
        );
        crate::reactions::draw(
            "thumbs_up",
            vec2(b.x + b.w / 2.0, b.y + b.h / 2.0),
            20.0,
            crate::paint::opacity() * if hover || self.reactions_open { 1.0 } else { 0.8 },
        );
        if hover && !self.reactions_open {
            ui.tooltip(t("Reactions"), b);
        }
        let clicked_button = hover && is_mouse_button_released(MouseButton::Left);
        if !self.reactions_open {
            if clicked_button {
                self.reactions_open = true;
            }
            return;
        }
        const COLS: usize = 7;
        const CELL: f32 = 40.0;
        let n = snook_proto::REACTIONS.len();
        let rows = n.div_ceil(COLS);
        let w = COLS as f32 * CELL + 16.0;
        let h = rows as f32 * CELL + 40.0;
        let p = Rect::new(b.x.min(crate::ui::vw() - w - 8.0), b.y - h - 8.0, w, h);
        ui.panel(p);
        let open = snook_proto::REACTIONS.iter().filter(|r| r.unlocked(&o.unlocked)).count();
        ui.text(
            &tw("Reactions {n}/{total}", &[("n", &open), ("total", &n)]),
            p.x + 10.0,
            p.y + 9.0,
            13.0,
            GOLD,
            true,
        );
        let mut chosen = None;
        let mut tip: Option<(String, Rect)> = None;
        for (k, r) in snook_proto::REACTIONS.iter().enumerate() {
            let c = Rect::new(
                p.x + 8.0 + (k % COLS) as f32 * CELL,
                p.y + 32.0 + (k / COLS) as f32 * CELL,
                CELL,
                CELL,
            );
            let center = vec2(c.x + c.w / 2.0, c.y + c.h / 2.0);
            let unlocked = r.unlocked(&o.unlocked);
            let hover = ui.hovering(c);
            if unlocked {
                if hover {
                    crate::ui::rrect(c, 8.0, Color::new(1.0, 1.0, 1.0, 0.10));
                }
                crate::reactions::draw(
                    r.code,
                    center,
                    if hover { 32.0 } else { 28.0 },
                    crate::paint::opacity(),
                );
                if hover && is_mouse_button_released(MouseButton::Left) {
                    chosen = Some(r.code);
                }
            } else {
                crate::reactions::draw_locked(r.code, center, 26.0);
                crate::icons::draw(
                    crate::icons::Icon::Lock,
                    vec2(c.x + c.w - 10.0, c.y + c.h - 10.0),
                    11.0,
                    MUTED,
                );
                if hover {
                    let how = r
                        .unlock
                        .and_then(|a| snook_proto::ACHIEVEMENTS.iter().find(|x| x.code == a))
                        .map(|a| {
                            tw(
                                "{title}: {how}",
                                &[("title", &t(a.title)), ("how", &t(a.description))],
                            )
                        })
                        .unwrap_or_default();
                    tip = Some((how, c));
                }
            }
        }
        if let Some((text, c)) = tip {
            ui.tooltip(&text, c);
        }
        ui.block(p);
        if let Some(code) = chosen {
            m.send_reaction(code);
            self.reactions_open = false;
        } else if (is_mouse_button_released(MouseButton::Left)
            && !ui.hovering(p)
            && !clicked_button)
            || clicked_button
        {
            self.reactions_open = false;
        }
    }
}

const PREVIEW_TIME: f32 = 5.0;
/// Height of the numbers panel under the game-over panel (and the gap).
const STATS_H: f32 = 150.0;

/// The game in numbers, per player: potting success, balls made, best run,
/// fouls and how often they had the cue ball in hand.
fn draw_match_stats(ui: &mut Ui, m: &Match, r: Rect) {
    ui.panel(r);
    let s = &m.stats;
    let rows = [
        (t("Potting success"), format!("{}%", s[0].pot_rate()), format!("{}%", s[1].pot_rate())),
        (t("Balls made"), s[0].balls.to_string(), s[1].balls.to_string()),
        (t("Best run"), s[0].best_run.to_string(), s[1].best_run.to_string()),
        (t("Fouls"), s[0].fouls.to_string(), s[1].fouls.to_string()),
        (
            t("Ball in hand"),
            format!("{}%", s[0].in_hand_rate()),
            format!("{}%", s[1].in_hand_rate()),
        ),
    ];
    let label_x = r.x + 16.0;
    let col = [r.x + r.w * 0.62, r.x + r.w * 0.86];
    for (k, name) in m.names.iter().enumerate() {
        let (name, size) = ui.fitted(name, 13.0, r.w * 0.22, true);
        ui.text_centered(&name, col[k], r.y + 10.0, size, GOLD, true);
    }
    for (i, (label, a, b)) in rows.iter().enumerate() {
        let y = r.y + 32.0 + i as f32 * 21.0;
        if i % 2 == 0 {
            crate::ui::rrect(
                Rect::new(r.x + 8.0, y - 3.0, r.w - 16.0, 20.0),
                4.0,
                Color::new(1.0, 1.0, 1.0, 0.035),
            );
        }
        let (label, size) = ui.fitted(label, 13.0, r.w * 0.45, false);
        ui.text(&label, label_x, y, size, MUTED, false);
        ui.text_centered(a, col[0], y, 13.0, CREAM, true);
        ui.text_centered(b, col[1], y, 13.0, CREAM, true);
    }
}
const REACTION_TIME: f32 = 2.8;

/// Reactions float down from the sender's card and fade (drawn even while
/// the HUD fades for aiming: they are meant to be seen).
fn draw_reactions(m: &mut Match, xs: [f32; 2], eye: Vec2) {
    let dt = get_frame_time();
    let Some(o) = m.online.as_mut() else { return };
    let mut crowd = 0;
    for (seat, code, age) in &mut o.reactions {
        *age += dt;
        let t = *age;
        if *seat >= 2 {
            // From the spectators: small, faint, drifting down from the eye,
            // side by side, so the players hardly notice them.
            let alpha = 0.75 * (t / 0.2).min(1.0) * ((REACTION_TIME - t) / 0.8).clamp(0.0, 1.0);
            let at = vec2(
                eye.x + 4.0 + crowd as f32 * 20.0,
                TOP_BAR + 14.0 + 18.0 * (t / REACTION_TIME),
            );
            crate::reactions::draw(code, at, 18.0, alpha);
            crowd += 1;
            continue;
        }
        let pop = if t < 0.25 {
            0.6 + 0.4 * (t / 0.25) + 0.25 * (t / 0.25 * std::f32::consts::PI).sin()
        } else {
            1.0
        };
        let alpha = (t / 0.12).min(1.0) * ((REACTION_TIME - t) / 0.7).clamp(0.0, 1.0);
        let y = TOP_BAR + 30.0 + 14.0 * (t / REACTION_TIME).sqrt();
        let size = 38.0 * pop;
        macroquad::shapes::draw_circle(
            xs[*seat],
            y,
            size * 0.62,
            Color::new(0.0, 0.0, 0.0, 0.35 * alpha),
        );
        crate::reactions::draw(code, vec2(xs[*seat], y), size, alpha);
    }
    o.reactions.retain(|(.., age)| *age < REACTION_TIME);
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
        Some(o) => o.seat == player && !o.spectator,
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
        return match m.game.called_pocket_of(player) {
            Some(p) => tw("On the 8-ball: {pocket} pocket", &[("pocket", &crate::i18n::pocket(p))]),
            None => t("On the 8-ball: call a pocket").into(),
        };
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

impl Hud {
    /// The bottom bar: game log, the power bar and the cue settings.
    /// Watching a friend: who, the reactions and a way out. Returns true to
    /// stop watching.
    fn draw_spectator_bar(&mut self, ui: &mut Ui, m: &mut Match) -> bool {
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        let bar = Rect::new(0.0, sh - BOTTOM_BAR, sw, BOTTOM_BAR);
        draw_bar(bar, false);
        let Some(o) = m.online.as_ref() else { return false };
        let friend = m.names[o.seat].clone();
        let cy = bar.y + bar.h / 2.0;
        crate::icons::draw(Icon::Eye, vec2(30.0, cy), 22.0, GOLD);
        ui.text(&tw("Watching {name}", &[("name", &friend)]), 50.0, cy - 17.0, 16.0, CREAM, true);
        ui.text(t("Your reactions reach the table quietly."), 50.0, cy + 4.0, 12.0, MUTED, false);
        let text_w = ui
            .text_size(&tw("Watching {name}", &[("name", &friend)]), 16.0, true)
            .x
            .max(ui.text_size(t("Your reactions reach the table quietly."), 12.0, false).x);
        self.reaction_button(ui, m, Rect::new(50.0 + text_w + 18.0, cy - 18.0, 36.0, 36.0));
        let w = ui.text_size(t("Stop watching"), 13.0, true).x + 32.0;
        ui.button(Rect::new(sw - w - 14.0, cy - 18.0, w, 36.0), t("Stop watching"), true)
    }

    fn draw_bottom_bar(&mut self, ui: &mut Ui, m: &mut Match) {
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        let bar = Rect::new(0.0, sh - BOTTOM_BAR, sw, BOTTOM_BAR);
        draw_bar(bar, false);
        let my_turn = m.is_human_turn() && !self.paused;

        // Cue settings on the left, the power bar in the middle and the shoot
        // button on the right: the bar reads evenly from edge to edge.
        let gap = 8.0;
        let bh = bar.h - 18.0;
        let shoot = Rect::new(sw - 10.0 - bh * 1.15, bar.y + 9.0, bh * 1.15, bh);
        // Narrow windows shrink the buttons before anything overlaps.
        let bw = (sw * 0.075).clamp(62.0, 84.0);
        let pops = [Pop::Elevation, Pop::Spin];
        let bx0 = 10.0;
        let mut anchors = [0.0f32; 2];
        for (k, pop) in pops.into_iter().enumerate() {
            let r = Rect::new(bx0 + k as f32 * (bw + gap), bar.y + 9.0, bw, bh);
            anchors[k] = r.x + r.w / 2.0;
            let open = self.popover == Some(pop);
            if ui.toggle(r, open) {
                self.popover = if open { None } else { Some(pop) };
            }
            draw_pop_icon(ui, m, pop, r);
        }
        // Fine aim right on the bar: one click on an arrow, one tiny step.
        let fw = (sw * 0.17).clamp(150.0, 210.0);
        let fine = Rect::new(bx0 + pops.len() as f32 * (bw + gap), bar.y + 9.0, fw, bh);
        self.fine_aim_inline(ui, m, fine, my_turn);
        let left_end = fine.x + fine.w + gap;
        // The power bar, centred in the space between (room for "100%" on its right).
        let room = (shoot.x - 60.0) - (left_end + 24.0);
        let tw = room.clamp(120.0, 560.0);
        let px0 = left_end + 24.0 + (room - tw) / 2.0;
        let track = Rect::new(px0, bar.y + bar.h / 2.0 - 9.0, tw, 18.0);

        // A new turn starts with the power the player chose last time.
        let key = (m.games_played, m.game.shots_taken());
        // Any shot (by anyone) closes the open cue-setting panel.
        if self.panel_key != Some(key) {
            self.panel_key = Some(key);
            self.popover = None;
        }
        if my_turn && self.power_key != Some(key) {
            self.power_key = Some(key);
            m.aim.power = self.power;
            m.aim.charging = self.power > 0.02;
        }
        let can_shoot = my_turn && !m.is_placing_cue_ball();
        if shoot_button(ui, shoot, can_shoot && m.aim.power > 0.02) {
            if m.game.needs_called_pocket() && m.called_pocket.is_none() {
                m.notify(t("Call a pocket for the 8-ball first: click a pocket."), true);
            } else if m.shoot() {
                self.popover = None;
            }
        }
        if can_shoot {
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
            // Closed as soon as the shot is taken (button, Space or online),
            // and when the turn passes.
            if !my_turn {
                self.popover = None;
                return;
            }
            let w = 250.0;
            let h = match pop {
                Pop::Spin => 186.0,
                Pop::Elevation => 92.0,
            };
            let k = match pop {
                Pop::Elevation => 0,
                Pop::Spin => 1,
            };
            let x = (anchors[k] - w / 2.0).clamp(10.0, sw - w - 10.0);
            let r = Rect::new(x, bar.y - h - 8.0, w, h);
            ui.panel(r);
            match pop {
                Pop::Spin => self.spin_panel(ui, m, r),
                Pop::Elevation => elevation_panel(ui, m, r),
            }
        }
    }

    /// The power bar: drag (or click) to set the power. Shooting is a separate
    /// button, so the player can set everything up first.
    fn stroke(&mut self, ui: &mut Ui, m: &mut Match, track: Rect) {
        let hit = Rect::new(track.x - 10.0, track.y - 14.0, track.w + 20.0, track.h + 28.0);
        let drag = ui.drag_area(90, hit);
        if let crate::ui::Drag::Held(p) | crate::ui::Drag::Released(p) = drag {
            let power = ((p.x - track.x) / track.w).clamp(0.0, 1.0);
            m.aim.power = power;
            m.aim.charging = power > 0.02;
            self.power = power;
        }
        let hot = matches!(drag, crate::ui::Drag::Held(_) | crate::ui::Drag::Hover);
        draw_power_track(ui, track, m.aim.power, hot, true);
        if hot && !matches!(drag, crate::ui::Drag::Held(_)) {
            let hint = t("Drag to set the power");
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

    /// Fine aim, built into the bar: the arrows turn the cue by one tiny
    /// step per click (holding doesn't repeat); the dial in between can be
    /// dragged for slightly bigger moves.
    fn fine_aim_inline(&mut self, ui: &mut Ui, m: &mut Match, r: Rect, enabled: bool) {
        crate::ui::rrect(r, 8.0, Color::new(0.12, 0.08, 0.05, 1.0));
        crate::ui::rrect_lines(r, 8.0, 1.2, crate::ui::BORDER);
        let label = t("Fine aim");
        let size = ui.fit(12.0, label, r.w - 16.0);
        ui.text_centered(label, r.x + r.w / 2.0, r.y + r.h - size - 6.0, size, CREAM, true);
        let row = Rect::new(r.x + 6.0, r.y + 6.0, r.w - 12.0, (r.h - size - 16.0).max(24.0));
        let aw = row.h.min(34.0);
        let step = 0.05f64.to_radians() * self.aim_sensitivity as f64;
        let left = Rect::new(row.x, row.y, aw, row.h);
        let right = Rect::new(row.x + row.w - aw, row.y, aw, row.h);
        if ui.button(left, "◀", enabled) {
            m.aim.yaw += step;
        }
        if ui.button(right, "▶", enabled) {
            m.aim.yaw -= step;
        }
        let dial = Rect::new(left.x + aw + 4.0, row.y, row.w - 2.0 * aw - 8.0, row.h);
        let drag = if enabled { ui.drag_area(82, dial) } else { crate::ui::Drag::Idle };
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
        let spacing = 8.0;
        let offset = ((m.aim.yaw * 4000.0) as f32).rem_euclid(spacing);
        let mut tx = dial.x + offset;
        let a = if enabled { 0.8 } else { 0.3 };
        while tx < dial.x + dial.w {
            let edge = ((tx - dial.x) / dial.w - 0.5).abs() * 2.0;
            let alpha = a * (1.0 - edge * edge);
            draw_line(
                tx,
                dial.y + 5.0,
                tx,
                dial.y + dial.h - 5.0,
                1.5,
                Color::new(0.9, 0.8, 0.6, alpha),
            );
            tx += spacing;
        }
        let cx = dial.x + dial.w / 2.0;
        draw_line(
            cx,
            dial.y + 2.0,
            cx,
            dial.y + dial.h - 2.0,
            2.0,
            Color::new(GOLD.r, GOLD.g, GOLD.b, a),
        );
        let hot = matches!(drag, crate::ui::Drag::Held(_) | crate::ui::Drag::Hover);
        crate::ui::rrect_lines(dial, 6.0, 1.2, if hot { GOLD } else { crate::ui::BORDER });
        if hot {
            ui.tooltip(t("Fine aim: click the arrows, or drag here"), r);
        }
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
    // The outline goes under the knob.
    crate::ui::rrect_lines(track, 9.0, 1.2, if hot { GOLD } else { crate::ui::BORDER });
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

const TOP_BAR: f32 = 54.0;
const BOTTOM_BAR: f32 = 78.0;
const CARD_H: f32 = 44.0;

/// The game log inside the bottom bar: the last few things that happened.
/// The shoot button: a cue striking a ball, on green. Unmistakable, no text.
fn shoot_button(ui: &mut Ui, r: Rect, enabled: bool) -> bool {
    ui.block(r);
    let hover = enabled && r.contains(ui.mouse());
    let down = hover && is_mouse_button_down(MouseButton::Left);
    let bg = if !enabled {
        Color::new(0.11, 0.14, 0.11, 0.9)
    } else if down {
        Color::new(0.10, 0.42, 0.20, 1.0)
    } else if hover {
        Color::new(0.20, 0.66, 0.34, 1.0)
    } else {
        Color::new(0.13, 0.52, 0.27, 1.0)
    };
    if enabled {
        let pulse = 0.5 + 0.5 * (get_time() as f32 * 3.0).sin();
        crate::ui::rrect(
            Rect::new(r.x - 3.0, r.y - 3.0, r.w + 6.0, r.h + 6.0),
            12.0,
            Color::new(0.3, 0.9, 0.45, 0.10 + 0.14 * pulse),
        );
    }
    crate::ui::rrect(r, 10.0, bg);
    crate::ui::rrect(
        Rect::new(r.x + 1.0, r.y + 1.0, r.w - 2.0, r.h * 0.45),
        9.0,
        Color::new(1.0, 1.0, 1.0, if enabled { 0.10 } else { 0.03 }),
    );
    let edge =
        if hover { GOLD } else { Color::new(0.35, 0.75, 0.5, if enabled { 1.0 } else { 0.3 }) };
    crate::ui::rrect_lines(r, 10.0, 1.5, edge);

    // The picture: a ball on the right, the cue coming in from the lower left
    // with its tip touching the ball, and a few impact lines.
    let a = if enabled { 1.0 } else { 0.35 };
    let s = r.h.min(r.w);
    let ball = vec2(r.x + r.w * 0.64, r.y + r.h * 0.42);
    let br = s * 0.17;
    let dir = vec2(1.0, -0.55).normalize();
    let tip = ball - dir * (br + 1.5);
    let butt = tip - dir * s * 0.62;
    // Shaft (light), wrap (dark) and the blue tip.
    draw_line(butt.x, butt.y, tip.x, tip.y, s * 0.075, Color::new(0.86, 0.74, 0.52, a));
    let wrap = butt + dir * s * 0.16;
    draw_line(butt.x, butt.y, wrap.x, wrap.y, s * 0.085, Color::new(0.20, 0.12, 0.08, a));
    let ferrule = tip - dir * s * 0.05;
    draw_line(ferrule.x, ferrule.y, tip.x, tip.y, s * 0.075, Color::new(0.35, 0.55, 0.9, a));
    // Impact lines on the far side of the ball.
    for k in [-1.0f32, 0.0, 1.0] {
        let n = vec2(-dir.y, dir.x);
        let from = ball + dir * (br + 3.0) + n * k * br * 0.7;
        let to = from + dir * s * 0.12;
        draw_line(from.x, from.y, to.x, to.y, 1.6, Color::new(1.0, 1.0, 1.0, 0.75 * a));
    }
    draw_circle(ball.x, ball.y + 1.5, br, Color::new(0.0, 0.0, 0.0, 0.3 * a));
    draw_circle(ball.x, ball.y, br, Color::new(0.97, 0.96, 0.92, a));
    draw_circle(
        ball.x - br * 0.35,
        ball.y - br * 0.35,
        br * 0.32,
        Color::new(1.0, 1.0, 1.0, 0.85 * a),
    );
    let clicked = enabled && hover && is_mouse_button_released(MouseButton::Left);
    if hover {
        let label = t("Shoot!");
        let w = ui.text_size(label, 13.0, true).x + 20.0;
        let tr = Rect::new(
            (r.x + r.w / 2.0 - w / 2.0).min(crate::ui::vw() - w - 6.0),
            r.y - 32.0,
            w,
            24.0,
        );
        crate::ui::rrect(tr, 12.0, Color::new(0.0, 0.0, 0.0, 0.8));
        ui.text(label, tr.x + 10.0, tr.y + 5.0, 13.0, CREAM, true);
    }
    clicked
}
