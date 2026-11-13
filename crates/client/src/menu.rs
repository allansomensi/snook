//! Main menu: who is signed in, the four ways to play, and a row of icons.

use macroquad::prelude::*;

#[allow(unused_imports)]
use crate::paint::{draw_circle, draw_line, draw_rectangle};

use crate::i18n::{t, tw};
use crate::icons::Icon;
use crate::online::{Online, Stage};
use crate::ui::{BORDER, CREAM, GOLD, MUTED, Ui, rrect, rrect_lines};

pub enum MenuAction {
    None,
    Online,
    VsComputer,
    /// Picks up the saved game against the computer.
    ContinueGame,
    /// The button under a note on the board.
    Note(snookered_proto::NoteAction),
    TwoPlayers,
    Lan,
    Ranking,
    Profile,
    Shop,
    Settings,
    Quit,
}

#[derive(Default)]
pub struct Menu {
    /// Hover animation of each item (0..1).
    hover: [f32; 4],
    shade: Option<Texture2D>,
}

/// A smooth left-to-right darkening (a texture, so there is no banding).
fn shade_texture() -> Texture2D {
    const W: u16 = 256;
    let mut rgba = Vec::with_capacity(W as usize * 4);
    for x in 0..W {
        let k = x as f32 / (W - 1) as f32;
        let a = 0.82 * (1.0 - k * k * (3.0 - 2.0 * k));
        rgba.extend_from_slice(&[0, 0, 0, (a * 255.0) as u8]);
    }
    let tex = Texture2D::from_rgba8(W, 1, &rgba);
    tex.set_filter(FilterMode::Linear);
    tex
}

impl Menu {
    /// `saved`: object balls left in the saved game against the computer.
    pub fn draw(&mut self, ui: &mut Ui, online: &Online, saved: Option<usize>) -> MenuAction {
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        let dt = get_frame_time();
        let mut action = MenuAction::None;
        // Darker on the left, where the menu is, so the table stays visible.
        let shade = self.shade.get_or_insert_with(shade_texture);
        draw_texture_ex(
            shade,
            0.0,
            0.0,
            WHITE,
            DrawTextureParams { dest_size: Some(vec2(sw * 0.7, sh)), ..Default::default() },
        );

        let left = (sw * 0.06).clamp(24.0, 90.0);
        let top = (sh * 0.09).clamp(20.0, 80.0);
        // The emblem, then the name.
        let size = 92.0;
        crate::icon::draw(left - 4.0, top + 2.0, size);
        let tx = left + size + 10.0;
        let name = snookered_proto::GAME_NAME.to_uppercase();
        ui.text(&name, tx + 3.0, top + 3.0, 64.0, Color::new(0.0, 0.0, 0.0, 0.6), true);
        ui.text(&name, tx, top, 64.0, GOLD, true);
        ui.text(t("eight-ball pool"), tx + 4.0, top + 74.0, 17.0, CREAM, false);

        // The four ways to play.
        let resume_line = saved.map(|n| tw("Game in progress: {n} balls left", &[("n", &n)]));
        let items = [
            (Icon::Globe, t("Play online"), t("Rated games, tables, tournaments and friends")),
            (
                Icon::Robot,
                t("Against the computer"),
                resume_line.as_deref().unwrap_or(t("Three levels, from relaxed to ruthless")),
            ),
            (Icon::People, t("Two players"), t("Take turns on this computer")),
            (Icon::Network, t("Local network"), t("Play with friends on the same network")),
        ];
        let w = (sw * 0.36).clamp(300.0, 440.0);
        let h = 66.0;
        let gap = 12.0;
        let total = items.len() as f32 * (h + gap);
        let mut y = ((sh - total) / 2.0 + 40.0).max(top + 120.0);
        for (k, (icon, title, sub)) in items.into_iter().enumerate() {
            let r = Rect::new(left, y, w, h);
            let hover = ui.hovering(r);
            let target = if hover { 1.0 } else { 0.0 };
            self.hover[k] += (target - self.hover[k]) * (1.0 - (-dt * 14.0).exp());
            let a = self.hover[k];
            let shifted = Rect::new(r.x + a * 8.0, r.y, r.w, r.h);
            ui.block(r);
            let primary = k == 0;
            let bg = if primary {
                Color::new(0.10 + 0.06 * a, 0.36 + 0.1 * a, 0.20 + 0.06 * a, 0.98)
            } else {
                Color::new(0.10 + 0.12 * a, 0.065 + 0.08 * a, 0.04 + 0.04 * a, 0.97)
            };
            rrect(
                Rect::new(shifted.x, shifted.y + 3.0, shifted.w, shifted.h),
                10.0,
                Color::new(0.0, 0.0, 0.0, 0.35),
            );
            rrect(shifted, 10.0, bg);
            rrect(
                Rect::new(shifted.x + 1.0, shifted.y + 1.0, shifted.w - 2.0, shifted.h * 0.45),
                9.0,
                Color::new(1.0, 1.0, 1.0, 0.05),
            );
            let edge = if hover {
                GOLD
            } else if primary {
                Color::new(0.35, 0.75, 0.5, 1.0)
            } else {
                BORDER
            };
            rrect_lines(shifted, 10.0, 1.4, edge);
            crate::icons::draw(
                icon,
                vec2(shifted.x + 36.0, shifted.y + h / 2.0),
                28.0,
                if hover { GOLD } else { CREAM },
            );
            ui.text(
                title,
                shifted.x + 70.0,
                shifted.y + 13.0,
                20.0,
                if primary { WHITE } else { CREAM },
                true,
            );
            ui.text(
                sub,
                shifted.x + 70.0,
                shifted.y + 39.0,
                13.0,
                Color::new(0.85, 0.8, 0.72, 0.85),
                false,
            );
            // The saved game: a button of its own inside the card.
            let resume = (k == 1 && saved.is_some()).then(|| {
                let bw = 104.0f32.min(w * 0.3);
                Rect::new(shifted.x + shifted.w - bw - 12.0, shifted.y + (h - 34.0) / 2.0, bw, 34.0)
            });
            if let Some(b) = resume
                && ui.primary_button(b, t("Continue"))
            {
                action = MenuAction::ContinueGame;
            }
            if hover && ui.clicked(r) && !resume.is_some_and(|b| ui.hovering(b)) {
                action = match k {
                    0 => MenuAction::Online,
                    1 => MenuAction::VsComputer,
                    2 => MenuAction::TwoPlayers,
                    _ => MenuAction::Lan,
                };
            }
            y += h + gap;
        }

        // Icons along the bottom.
        let icons = [
            (Icon::Trophy, t("World ranking"), MenuAction::Ranking),
            (Icon::Person, t("Profile"), MenuAction::Profile),
            (Icon::Shop, t("Shop"), MenuAction::Shop),
            (Icon::Gear, t("Settings"), MenuAction::Settings),
            (Icon::Power, t("Quit"), MenuAction::Quit),
        ];
        let size = 46.0;
        let by = (y + 14.0).min(sh - size - 20.0);
        for (k, (icon, tip, act)) in icons.into_iter().enumerate() {
            let r = Rect::new(left + k as f32 * (size + 10.0), by, size, size);
            if ui.icon_button(r, icon, tip, true) {
                action = act;
            }
        }

        // Who is signed in, top right, and the board under it.
        self.player_card(ui, online, &mut action);
        self.board(ui, online, &mut action);

        let version = concat!("v", env!("CARGO_PKG_VERSION"));
        let vw = ui.text_size(version, 12.0, false).x;
        ui.text(version, sw - vw - 14.0, sh - 24.0, 12.0, MUTED, false);
        action
    }

    /// The newest notes on the lounge's board (pinned first), for players
    /// who don't go to the lounge.
    fn board(&self, ui: &mut Ui, online: &Online, action: &mut MenuAction) {
        let notes = online.board_now();
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        // Beside the menu, only where the window leaves room.
        let menu_right = (sw * 0.06).clamp(24.0, 90.0) + (sw * 0.36).clamp(300.0, 440.0);
        let w = 320.0f32.min(sw * 0.3);
        let x = sw - w - 20.0;
        if notes.is_empty() || x < menu_right + 24.0 {
            return;
        }
        let mut y = 100.0;
        ui.text(t("Board"), x + 2.0, y, 15.0, GOLD, true);
        y += 24.0;
        for n in notes.iter().take(3) {
            let h = crate::online::note_height(ui, n, w);
            if y + h > sh - 60.0 {
                break;
            }
            let card = Rect::new(x, y, w, h);
            rrect(card, 8.0, Color::new(0.05, 0.035, 0.025, 0.86));
            if let Some(a) = crate::online::draw_note(ui, n, card) {
                *action = MenuAction::Note(a);
            }
            ui.block(card);
            y += h + 8.0;
        }
    }

    fn player_card(&self, ui: &mut Ui, online: &Online, action: &mut MenuAction) {
        let sw = crate::ui::vw();
        let w = 280.0f32.min(sw * 0.4);
        let r = Rect::new(sw - w - 20.0, 20.0, w, 64.0);
        let hover = ui.hovering(r);
        rrect(r, 10.0, Color::new(0.05, 0.035, 0.025, if hover { 0.92 } else { 0.78 }));
        rrect_lines(r, 10.0, 1.2, if hover { GOLD } else { BORDER });
        ui.block(r);
        match &online.me {
            Some(me) => {
                crate::icons::draw(
                    Icon::Person,
                    vec2(r.x + 30.0, r.y + 32.0),
                    28.0,
                    crate::ui::name_color(me),
                );
                ui.player_name(me, r.x + 56.0, r.y + 12.0, 17.0, r.w - 70.0);
                let line = format!("{} · {}", me.rating, t(me.tier().name()));
                ui.text(&line, r.x + 56.0, r.y + 36.0, 13.0, MUTED, false);
                draw_circle(r.x + r.w - 16.0, r.y + 16.0, 4.0, Color::new(0.35, 0.85, 0.45, 1.0));
            }
            None => {
                crate::icons::draw(Icon::Person, vec2(r.x + 30.0, r.y + 32.0), 28.0, MUTED);
                let (title, sub) = match online.stage {
                    Stage::Connecting | Stage::Authenticating if online.has_session() => {
                        (t("Signing in…").to_string(), t("connecting to the server").to_string())
                    }
                    Stage::Reconnecting { .. } => {
                        (t("Reconnecting…").to_string(), t("the connection dropped").to_string())
                    }
                    _ => (t("Sign in").to_string(), t("to play online").to_string()),
                };
                ui.text(&title, r.x + 56.0, r.y + 12.0, 17.0, CREAM, true);
                let sub = ui.ellipsize(&sub, r.w - 70.0, 12.0);
                ui.text(&sub, r.x + 56.0, r.y + 37.0, 12.0, MUTED, false);
            }
        }
        if hover && ui.clicked(r) {
            *action = if online.me.is_some() { MenuAction::Profile } else { MenuAction::Online };
        }
        if hover {
            let tip = if online.me.is_some() {
                t("Your profile")
            } else {
                t("Sign in or create an account")
            };
            ui.tooltip(tip, r);
        }
    }
}
