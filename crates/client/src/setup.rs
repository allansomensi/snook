//! Setting up an offline game: names, the computer's level, the aiming
//! guide and the shot clock.

use macroquad::prelude::*;
use snook_core::Difficulty;

use crate::config::{AimGuide, Config};
use crate::i18n::t;
use crate::ui::{GOLD, MUTED, Ui};
use crate::{MatchMode, MatchSettings};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupKind {
    VsComputer,
    TwoPlayers,
}

pub enum SetupAction {
    None,
    Back,
    Start(MatchSettings),
}

pub struct Setup {
    pub kind: SetupKind,
}

fn difficulty(i: u8) -> Difficulty {
    match i {
        0 => Difficulty::Easy,
        2 => Difficulty::Hard,
        _ => Difficulty::Medium,
    }
}

impl Setup {
    pub fn new(kind: SetupKind) -> Setup {
        Setup { kind }
    }

    pub fn draw(&mut self, ui: &mut Ui, config: &mut Config) -> SetupAction {
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.45));
        let w = 520.0f32.min(sw - 20.0);
        let h = 500.0;
        let r = Rect::new((sw - w) / 2.0, ((sh - h) / 2.0).max(8.0), w, h);
        ui.panel(r);
        let x = r.x + 30.0;
        let inner = w - 60.0;
        let (icon, title) = match self.kind {
            SetupKind::VsComputer => (crate::icons::Icon::Robot, t("Against the computer")),
            SetupKind::TwoPlayers => (crate::icons::Icon::People, t("Two players")),
        };
        crate::icons::draw(icon, vec2(x + 14.0, r.y + 32.0), 26.0, GOLD);
        ui.text(title, x + 40.0, r.y + 20.0, 24.0, GOLD, true);
        let mut y = r.y + 72.0;
        let label = |ui: &Ui, text: &str, y: f32| ui.text(text, x, y, 13.0, MUTED, true);

        // Names.
        match self.kind {
            SetupKind::VsComputer => {
                label(ui, t("Your name"), y);
                ui.text_input(10, Rect::new(x, y + 18.0, inner, 34.0), &mut config.player_name, 20);
                if config.player_name.is_empty() && ui.focus != Some(10) {
                    ui.text(t("Player"), x + 9.0, y + 27.0, 15.0, MUTED, false);
                }
                y += 66.0;
                label(ui, t("Opponent"), y);
                let levels = [
                    crate::i18n::difficulty(Difficulty::Easy),
                    crate::i18n::difficulty(Difficulty::Medium),
                    crate::i18n::difficulty(Difficulty::Hard),
                ];
                if let Some(i) = ui.segmented(Rect::new(x, y + 18.0, inner, 34.0), &levels, config.difficulty as usize) {
                    config.difficulty = i as u8;
                }
                let hint = match config.difficulty {
                    0 => t("Misses now and then: good for learning."),
                    2 => t("Plans position and plays safeties."),
                    _ => t("A solid club player."),
                };
                ui.text(hint, x, y + 58.0, 12.0, MUTED, false);
                y += 82.0;
            }
            SetupKind::TwoPlayers => {
                let half = (inner - 12.0) / 2.0;
                label(ui, t("Player 1"), y);
                ui.text_input(10, Rect::new(x, y + 18.0, half, 34.0), &mut config.player_name, 20);
                if config.player_name.is_empty() && ui.focus != Some(10) {
                    ui.text(t("Player"), x + 9.0, y + 27.0, 15.0, MUTED, false);
                }
                ui.text(t("Player 2"), x + half + 12.0, y, 13.0, MUTED, true);
                ui.text_input(11, Rect::new(x + half + 12.0, y + 18.0, half, 34.0), &mut config.second_player_name, 20);
                if config.second_player_name.is_empty() && ui.focus != Some(11) {
                    ui.text(t("Player 2"), x + half + 21.0, y + 27.0, 15.0, MUTED, false);
                }
                y += 82.0;
            }
        }

        // Aiming help.
        label(ui, t("Aiming guide"), y);
        let guides = [t("Full"), t("Standard"), t("Short")];
        let sel = AimGuide::ALL.iter().position(|g| *g == config.guide).unwrap_or(1);
        if let Some(i) = ui.segmented(Rect::new(x, y + 18.0, inner, 34.0), &guides, sel) {
            config.guide = AimGuide::ALL[i];
        }
        let hint = match config.guide {
            AimGuide::Full => t("Shows where the object ball and the cue ball go."),
            AimGuide::Standard => t("Shows the contact point; judging the cut is up to you."),
            AimGuide::Short => t("A short line, as in rated online games."),
        };
        ui.text(hint, x, y + 58.0, 12.0, MUTED, false);
        y += 82.0;

        // Shot clock.
        label(ui, t("Time per shot"), y);
        let clocks = [t("No limit"), "30 s", "60 s"];
        let sel = match config.shot_clock {
            30 => 1,
            60 => 2,
            _ => 0,
        };
        if let Some(i) = ui.segmented(Rect::new(x, y + 18.0, inner, 34.0), &clocks, sel) {
            config.shot_clock = [0, 30, 60][i];
        }
        ui.text(t("When time runs out, the opponent gets ball in hand."), x, y + 58.0, 12.0, MUTED, false);

        let by = r.y + h - 62.0;
        let mut action = SetupAction::None;
        if ui.button(Rect::new(x, by, 150.0, 42.0), t("Back"), true)
            || (is_key_pressed(KeyCode::Escape) && ui.focus.is_none())
        {
            action = SetupAction::Back;
        }
        if ui.primary_button(Rect::new(x + inner - 220.0, by, 220.0, 42.0), t("Play")) {
            let mode = match self.kind {
                SetupKind::VsComputer => MatchMode::VsComputer(difficulty(config.difficulty)),
                SetupKind::TwoPlayers => MatchMode::HotSeat,
            };
            action = SetupAction::Start(MatchSettings::from_config(mode, config));
        }
        action
    }
}
