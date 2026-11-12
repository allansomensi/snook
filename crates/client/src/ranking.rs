//! The world ranking: the top 100 rated players.

use macroquad::prelude::*;

#[allow(unused_imports)]
use crate::paint::{draw_circle, draw_line, draw_rectangle};

use crate::i18n::t;
use crate::icons::Icon;
use crate::online::Online;
use crate::ui::{BORDER, CREAM, GOLD, MUTED, Ui, rrect, tier_color};

pub enum RankingAction {
    None,
    Back,
    Profile(String),
}

#[derive(Default)]
pub struct Ranking {
    first: usize,
}

impl Ranking {
    pub fn open(&mut self, online: &mut Online) {
        self.first = 0;
        online.request_leaderboard();
    }

    pub fn draw(&mut self, ui: &mut Ui, online: &mut Online) -> RankingAction {
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.5));
        let w = (sw - 40.0).min(760.0);
        let h = sh - 40.0;
        let r = Rect::new((sw - w) / 2.0, 20.0, w, h);
        ui.panel(r);
        let mut action = RankingAction::None;
        if ui.icon_button(
            Rect::new(r.x + 14.0, r.y + 14.0, 34.0, 34.0),
            Icon::Back,
            t("Back"),
            true,
        ) || (is_key_pressed(KeyCode::Escape) && ui.focus.is_none())
        {
            action = RankingAction::Back;
        }
        crate::icons::draw(Icon::Trophy, vec2(r.x + 74.0, r.y + 31.0), 26.0, GOLD);
        ui.text(t("World ranking"), r.x + 96.0, r.y + 16.0, 24.0, GOLD, true);
        ui.text(t("Top 100 · rated games"), r.x + 96.0, r.y + 44.0, 12.0, MUTED, false);
        if ui.icon_button(
            Rect::new(r.x + r.w - 48.0, r.y + 14.0, 34.0, 34.0),
            Icon::Refresh,
            t("Refresh"),
            true,
        ) {
            online.request_leaderboard();
        }
        let cols = [
            (r.x + 24.0, "#"),
            (r.x + 70.0, t("Player")),
            (r.x + r.w - 300.0, t("Rating")),
            (r.x + r.w - 210.0, t("Tier")),
            (r.x + r.w - 130.0, t("Games")),
            (r.x + r.w - 58.0, t("Win %")),
        ];
        let head_y = r.y + 76.0;
        for (x, label) in cols {
            ui.text(label, x, head_y, 12.0, MUTED, true);
        }
        draw_line(r.x + 16.0, head_y + 20.0, r.x + r.w - 16.0, head_y + 20.0, 1.0, BORDER);
        let list = Rect::new(r.x + 12.0, head_y + 26.0, r.w - 24.0, r.y + r.h - head_y - 40.0);
        if online.leaderboard.is_empty() {
            let text = if online.leaderboard_loaded {
                t("No rated games yet. Be the first!")
            } else {
                t("Loading…")
            };
            ui.text_centered(text, sw / 2.0, list.y + 30.0, 15.0, MUTED, false);
            return action;
        }
        let me = online.me.as_ref().map(|m| m.username.clone()).unwrap_or_default();
        let row_h = 34.0;
        let visible = (list.h / row_h).floor().max(1.0) as usize;
        let entries = online.leaderboard.clone();
        self.first = ui.scroll_list(list, self.first, entries.len(), visible);
        for (k, e) in entries.iter().skip(self.first).take(visible).enumerate() {
            let y = list.y + k as f32 * row_h;
            let row = Rect::new(list.x, y, list.w - 8.0, row_h - 3.0);
            let mine = e.username == me;
            let hover = ui.hovering(row);
            let bg = if mine {
                Color::new(0.91, 0.75, 0.38, 0.16)
            } else if hover {
                Color::new(1.0, 1.0, 1.0, 0.07)
            } else if k % 2 == 0 {
                Color::new(1.0, 1.0, 1.0, 0.025)
            } else {
                Color::new(0.0, 0.0, 0.0, 0.0)
            };
            rrect(row, 5.0, bg);
            let medal = match e.rank {
                1 => Some(GOLD),
                2 => Some(Color::new(0.82, 0.82, 0.86, 1.0)),
                3 => Some(Color::new(0.80, 0.55, 0.33, 1.0)),
                _ => None,
            };
            let ty = y + 8.0;
            match medal {
                Some(c) => {
                    draw_circle(r.x + 30.0, y + row_h / 2.0 - 1.0, 11.0, c);
                    ui.text_centered(
                        &e.rank.to_string(),
                        r.x + 30.0,
                        ty,
                        13.0,
                        Color::new(0.1, 0.07, 0.04, 1.0),
                        true,
                    );
                }
                None => ui.text(&e.rank.to_string(), r.x + 24.0, ty, 14.0, MUTED, true),
            }
            let tier = snookered_proto::Tier::of(e.rating);
            let color = if mine { GOLD } else { tier_color(tier) };
            let name_w = ui.name_with_flag(
                &e.username,
                &e.country,
                color,
                r.x + 70.0,
                ty - 1.0,
                15.0,
                r.w - 420.0,
            );
            if e.vip {
                // VIP members: the crown, as everywhere else.
                crate::icons::draw(
                    Icon::Crown,
                    vec2(r.x + 70.0 + name_w + 13.0, ty + 7.0),
                    13.0,
                    GOLD,
                );
            }
            ui.text(&e.rating.to_string(), r.x + r.w - 300.0, ty, 15.0, CREAM, true);
            ui.text(t(tier.name()), r.x + r.w - 210.0, ty + 1.0, 13.0, tier_color(tier), false);
            ui.text(&e.games.to_string(), r.x + r.w - 130.0, ty + 1.0, 13.0, MUTED, false);
            let pct = (e.wins * 100).checked_div(e.games).unwrap_or(0);
            ui.text(&format!("{pct}%"), r.x + r.w - 58.0, ty + 1.0, 13.0, MUTED, false);
            if hover && ui.clicked(row) {
                action = RankingAction::Profile(e.username.clone());
            }
        }
        action
    }
}
