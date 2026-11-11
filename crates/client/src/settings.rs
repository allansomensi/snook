//! The settings screen: graphics, game and sound, in tabs.

use macroquad::prelude::*;

use crate::config::{AimGuide, Config, Quality};
use crate::i18n::{Lang, t, tw};
use crate::ui::{ALERT, CREAM, GOLD, MUTED, Ui};

pub enum SettingsAction {
    None,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Graphics,
    Game,
    Sound,
}

#[derive(Default)]
pub struct SettingsScreen {
    /// MSAA when the game started (changes apply after a restart).
    pub startup_msaa: Option<i32>,
    tab: Tab,
    /// The interface size being dragged (applied on release).
    pending_size: Option<f32>,
}

impl SettingsScreen {
    pub fn draw(&mut self, ui: &mut Ui, config: &mut Config, in_match: bool) -> SettingsAction {
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        let startup_msaa = *self.startup_msaa.get_or_insert(config.msaa);
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.5));
        let w = 520.0;
        let h = 520.0;
        let r = Rect::new((sw - w) / 2.0, ((sh - h) / 2.0).max(8.0), w, h);
        ui.panel(r);
        let x = r.x + 28.0;
        let inner = w - 56.0;
        ui.text_centered(t("Settings"), sw / 2.0, r.y + 14.0, 24.0, GOLD, true);

        // Tabs.
        let tabs =
            [(Tab::Graphics, t("Graphics")), (Tab::Game, t("Game")), (Tab::Sound, t("Sound"))];
        let tab_w = (inner - 20.0) / 3.0;
        for (k, (tab, label)) in tabs.into_iter().enumerate() {
            let tr = Rect::new(x + k as f32 * (tab_w + 10.0), r.y + 54.0, tab_w, 32.0);
            if ui.tab(tr, label, self.tab == tab) {
                self.tab = tab;
            }
        }
        let mut y = r.y + 106.0;
        match self.tab {
            Tab::Graphics => {
                ui.text(t("Graphics quality"), x, y, 16.0, GOLD, true);
                y += 26.0;
                let labels: Vec<&str> = Quality::ALL.iter().map(|q| t(q.name())).collect();
                let sel = Quality::ALL.iter().position(|q| *q == config.quality).unwrap_or(0);
                if let Some(i) = ui.segmented(Rect::new(x, y, inner, 32.0), &labels, sel)
                    && Quality::ALL[i] != config.quality
                {
                    config.quality = Quality::ALL[i];
                    config.msaa = config.quality.default_msaa();
                }
                y += 40.0;
                let hint = match config.quality {
                    Quality::Low => "For old or very slow computers.",
                    Quality::Medium => "Balanced: smooth on most laptops.",
                    Quality::High => "Soft shadows from every lamp, contact shading.",
                    Quality::Ultra => "Everything, with the sharpest balls: for gaming PCs.",
                };
                ui.text(t(hint), x, y, 13.0, MUTED, false);
                y += 34.0;
                ui.text(t("Anti-aliasing (smooth edges)"), x, y, 16.0, GOLD, true);
                y += 26.0;
                let levels = [1, 2, 4, 8];
                let sel = levels.iter().position(|n| *n == config.msaa).unwrap_or(0);
                if let Some(i) =
                    ui.segmented(Rect::new(x, y, inner, 32.0), &[t("Off"), "2×", "4×", "8×"], sel)
                {
                    config.msaa = levels[i];
                }
                y += 40.0;
                if config.msaa != startup_msaa {
                    ui.text(t("Restart the game to apply this change."), x, y, 13.0, ALERT, false);
                }
                y += 34.0;
                if ui.checkbox(x, y, t("Full screen (F11)"), &mut config.fullscreen) {
                    set_fullscreen(config.fullscreen);
                }
                y += 32.0;
                ui.checkbox(x, y, t("Vignette (cinematic edges)"), &mut config.vignette);
                y += 32.0;
                ui.checkbox(
                    x,
                    y,
                    t("Dark room (spotlight on the table)"),
                    &mut config.focused_scene,
                );
                y += 32.0;
                ui.checkbox(x, y, t("Show FPS"), &mut config.show_fps);
                ui.checkbox(x + inner / 2.0, y, t("Show ping (online)"), &mut config.show_ping);
            }
            Tab::Game => {
                ui.text(t("Language"), x, y + 8.0, 15.0, CREAM, false);
                let names: Vec<&str> = Lang::ALL.iter().map(|l| l.name()).collect();
                let sel = Lang::ALL.iter().position(|l| *l == config.language).unwrap_or(0);
                if let Some(i) =
                    ui.segmented(Rect::new(x + 150.0, y, inner - 150.0, 30.0), &names, sel)
                {
                    config.language = Lang::ALL[i];
                }
                y += 40.0;
                ui.text(t("Aiming guide"), x, y + 8.0, 15.0, CREAM, false);
                let names: Vec<&str> = AimGuide::ALL.iter().map(|g| t(g.name())).collect();
                let sel = AimGuide::ALL.iter().position(|g| *g == config.guide).unwrap_or(0);
                if let Some(i) =
                    ui.segmented(Rect::new(x + 150.0, y, inner - 150.0, 30.0), &names, sel)
                {
                    config.guide = AimGuide::ALL[i];
                }
                y += 40.0;
                ui.text(t("Mouse aim sensitivity"), x, y, 15.0, CREAM, false);
                ui.slider(
                    72,
                    Rect::new(x + 262.0, y, inner - 262.0, 18.0),
                    &mut config.aim_sensitivity,
                    0.3,
                    2.5,
                );
                y += 38.0;
                ui.text(
                    &tw(
                        "Interface size  {v}%",
                        &[("v", &format!("{:.0}", config.ui_size * 100.0))],
                    ),
                    x,
                    y,
                    15.0,
                    CREAM,
                    false,
                );
                let mut size = config.ui_size;
                ui.slider(73, Rect::new(x + 262.0, y, inner - 262.0, 18.0), &mut size, 0.7, 1.5);
                // Applied when the slider is let go, so it doesn't move under the pointer.
                if !is_mouse_button_down(MouseButton::Left) {
                    config.ui_size = (size * 20.0).round() / 20.0;
                } else {
                    self.pending_size = Some(size);
                }
                if let Some(p) =
                    self.pending_size.filter(|_| !is_mouse_button_down(MouseButton::Left))
                {
                    config.ui_size = (p * 20.0).round() / 20.0;
                    self.pending_size = None;
                }
                y += 38.0;
                ui.checkbox(x, y, t("Camera follows the cue"), &mut config.camera_follow);
                if config.camera_follow {
                    ui.checkbox(x + 260.0, y, t("Pull back for shots"), &mut config.shot_camera);
                }
                y += 32.0;
                ui.checkbox(x, y, t("Show the controls panel"), &mut config.show_help);
                y += 38.0;
                if !in_match {
                    ui.text(t("Game server"), x, y + 7.0, 15.0, CREAM, false);
                    ui.text_input(
                        71,
                        Rect::new(x + 150.0, y, inner - 150.0, 30.0),
                        &mut config.server,
                        200,
                    );
                    y += 36.0;
                    if ui.button(Rect::new(x + 150.0, y, 150.0, 24.0), t("Use default"), true) {
                        config.server = crate::config::DEFAULT_SERVER.into();
                    }
                }
            }
            Tab::Sound => {
                ui.text(
                    &tw("Sound volume  {v}%", &[("v", &format!("{:.0}", config.volume * 100.0))]),
                    x,
                    y,
                    15.0,
                    CREAM,
                    false,
                );
                y += 24.0;
                ui.slider(70, Rect::new(x, y, inner, 18.0), &mut config.volume, 0.0, 1.0);
            }
        }

        let by = r.y + h - 56.0;
        if ui.button(Rect::new(x, by, 190.0, 38.0), t("Reset to defaults"), true) {
            let keep = config.clone();
            *config = Config {
                player_name: keep.player_name,
                second_player_name: keep.second_player_name,
                username: keep.username,
                token: keep.token,
                language: keep.language,
                graphics_checked: true,
                ..Config::default()
            };
            set_fullscreen(config.fullscreen);
        }
        if ui.primary_button(Rect::new(x + inner - 190.0, by, 190.0, 38.0), t("Done"))
            || (is_key_pressed(KeyCode::Escape) && ui.focus.is_none())
        {
            if config.server.trim().is_empty() {
                config.server = crate::config::DEFAULT_SERVER.into();
            }
            config.server = config.server.trim().to_string();
            return SettingsAction::Close;
        }
        SettingsAction::None
    }
}
