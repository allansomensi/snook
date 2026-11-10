//! Main menu.

use macroquad::prelude::*;

use crate::i18n::t;
use snook_core::Difficulty;

use crate::MatchMode;
use crate::config::Config;
use crate::ui::{CREAM, GOLD, MUTED, Ui};

pub enum MenuAction {
    None,
    Start(MatchMode),
    Online,
    Settings,
    Quit,
}

pub struct Menu {
    difficulty: Difficulty,
}

impl Default for Menu {
    fn default() -> Self {
        Menu { difficulty: Difficulty::Medium }
    }
}

impl Menu {
    pub fn draw(&mut self, ui: &mut Ui, settings: &mut Config) -> MenuAction {
        let sw = screen_width();
        let sh = screen_height();
        let mut action = MenuAction::None;

        // Title.
        let title_y = (sh * 0.07).max(20.0);
        ui.text_centered(
            "SNOOK",
            sw / 2.0 + 2.0,
            title_y + 2.0,
            60.0,
            Color::new(0.0, 0.0, 0.0, 0.6),
            true,
        );
        ui.text_centered("SNOOK", sw / 2.0, title_y, 60.0, GOLD, true);
        ui.text_centered(
            t("eight-ball  ·  play, chat, climb the ranking"),
            sw / 2.0,
            title_y + 70.0,
            18.0,
            CREAM,
            false,
        );

        let w = 400.0;
        let h = 432.0;
        let r = Rect::new(
            (sw - w) / 2.0,
            (title_y + 110.0).max((sh - h) / 2.0 + 40.0).min(sh - h - 10.0),
            w,
            h,
        );
        ui.panel(r);
        let x = r.x + 22.0;
        let inner = w - 44.0;
        let mut y = r.y + 18.0;

        ui.text(t("Your name"), x, y + 6.0, 15.0, CREAM, false);
        ui.text_input(
            10,
            Rect::new(x + 100.0, y, inner - 100.0, 30.0),
            &mut settings.player_name,
            20,
        );
        if settings.player_name.is_empty() && ui.focus != Some(10) {
            ui.text(t("Player"), x + 109.0, y + 7.0, 15.0, MUTED, false);
        }
        y += 48.0;

        ui.text(t("Play against the computer"), x, y, 16.0, GOLD, true);
        y += 26.0;
        let mut rx = x;
        for d in Difficulty::ALL {
            if ui.radio(rx, y, crate::i18n::difficulty(d), self.difficulty == d) {
                self.difficulty = d;
            }
            rx += 110.0;
        }
        y += 30.0;
        if ui.primary_button(Rect::new(x, y, inner, 42.0), t("Play")) {
            action = MenuAction::Start(MatchMode::VsComputer(self.difficulty));
        }
        y += 60.0;

        ui.text(t("Two players on this computer"), x, y, 16.0, GOLD, true);
        y += 26.0;
        ui.text(t("Player 2"), x, y + 6.0, 15.0, CREAM, false);
        ui.text_input(
            11,
            Rect::new(x + 100.0, y, inner - 100.0, 30.0),
            &mut settings.second_player_name,
            20,
        );
        if settings.second_player_name.is_empty() && ui.focus != Some(11) {
            ui.text(t("Player 2"), x + 109.0, y + 7.0, 15.0, MUTED, false);
        }
        y += 40.0;
        if ui.button(Rect::new(x, y, inner, 34.0), t("Play two players"), true) {
            action = MenuAction::Start(MatchMode::HotSeat);
        }
        y += 50.0;

        ui.text(t("Online"), x, y, 16.0, GOLD, true);
        y += 26.0;
        if ui.primary_button(Rect::new(x, y, inner, 42.0), t("Enter the lounge")) {
            action = MenuAction::Online;
        }
        y += 58.0;

        let bw = (inner - 20.0) / 3.0;
        if ui.button(Rect::new(x, y, bw, 32.0), t("Settings"), true) {
            action = MenuAction::Settings;
        }
        if ui.button(Rect::new(x + bw + 10.0, y, bw, 32.0), t("Watch demo"), true) {
            action = MenuAction::Start(MatchMode::Demo);
        }
        if ui.button(Rect::new(x + 2.0 * (bw + 10.0), y, bw, 32.0), t("Quit"), true) {
            action = MenuAction::Quit;
        }

        let version = concat!("v", env!("CARGO_PKG_VERSION"));
        let vw = ui.text_size(version, 12.0, false).x;
        ui.text(version, sw - vw - 12.0, sh - 22.0, 12.0, MUTED, false);
        action
    }
}
