//! Choosing a country: a field showing the flag, and a searchable list.

use std::cell::RefCell;

use macroquad::prelude::*;
use snook_proto::COUNTRIES;

use crate::i18n::t;
use crate::ui::{BORDER, CREAM, GOLD, MUTED, Ui, rrect, rrect_lines};

#[derive(Default)]
struct Picker {
    open: bool,
    query: String,
    first: usize,
}

thread_local! {
    static PICKER: RefCell<Picker> = RefCell::new(Picker::default());
}

/// A field that shows the chosen country; true when clicked.
pub fn field(ui: &mut Ui, r: Rect, code: &str, label: &str) -> bool {
    let clicked = ui.button(r, "", true);
    let x = if crate::flags::draw(code, r.x + 10.0, r.y + (r.h - 16.0) / 2.0, 16.0) {
        r.x + 36.0
    } else {
        r.x + 10.0
    };
    ui.text(label, x, r.y + (r.h - 15.0) / 2.0, 15.0, CREAM, false);
    ui.text("▾", r.x + r.w - 20.0, r.y + (r.h - 15.0) / 2.0, 15.0, MUTED, false);
    clicked
}

pub fn open() {
    PICKER.with(|p| {
        let mut p = p.borrow_mut();
        p.open = true;
        p.query.clear();
        p.first = 0;
    });
}

pub fn is_open() -> bool {
    PICKER.with(|p| p.borrow().open)
}

/// Draws the list when open; returns the code chosen ("" for none).
pub fn draw_picker(ui: &mut Ui) -> Option<String> {
    if !is_open() {
        return None;
    }
    let sw = crate::ui::vw();
    let sh = crate::ui::vh();
    draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.55));
    let w = 380.0f32.min(sw - 20.0);
    let h = (sh - 60.0).min(560.0);
    let r = Rect::new((sw - w) / 2.0, (sh - h) / 2.0, w, h);
    ui.panel(r);
    ui.block(Rect::new(0.0, 0.0, sw, sh));
    ui.text(t("Country"), r.x + 16.0, r.y + 14.0, 18.0, GOLD, true);
    let mut chosen = None;
    let close = ui.icon_button_ex(
        Rect::new(r.x + r.w - 40.0, r.y + 10.0, 30.0, 30.0),
        crate::icons::Icon::Close,
        t("Close"),
        true,
        true,
    );
    PICKER.with(|p| {
        let mut p = p.borrow_mut();
        let input = Rect::new(r.x + 14.0, r.y + 48.0, r.w - 28.0, 32.0);
        if ui.focus.is_none() {
            ui.focus = Some(60);
        }
        ui.text_input(60, input, &mut p.query, 30);
        if p.query.is_empty() {
            ui.text(t("Search…"), input.x + 9.0, input.y + 9.0, 14.0, MUTED, false);
        }
        let q = p.query.to_lowercase();
        let lang_pt = crate::i18n::lang() == crate::i18n::Lang::Portuguese;
        let mut list: Vec<(&str, &str)> = COUNTRIES
            .iter()
            .map(|(c, en, pt)| (*c, if lang_pt { *pt } else { *en }))
            .filter(|(c, name)| q.is_empty() || name.to_lowercase().contains(&q) || c.to_lowercase() == q)
            .collect();
        list.sort_by_key(|(_, name)| name.to_lowercase());
        let area = Rect::new(r.x + 10.0, input.y + 42.0, r.w - 20.0, r.y + r.h - input.y - 52.0 - 44.0);
        let row_h = 28.0;
        let visible = (area.h / row_h).floor().max(1.0) as usize;
        p.first = ui.scroll_list(area, p.first, list.len(), visible);
        for (k, (code, name)) in list.iter().skip(p.first).take(visible).enumerate() {
            let row = Rect::new(area.x, area.y + k as f32 * row_h, area.w - 8.0, row_h - 2.0);
            let hover = ui.hovering(row);
            if hover {
                rrect(row, 5.0, Color::new(1.0, 1.0, 1.0, 0.07));
            }
            crate::flags::draw(code, row.x + 8.0, row.y + 6.0, 15.0);
            ui.text(name, row.x + 36.0, row.y + 6.0, 14.0, CREAM, false);
            if hover && ui.clicked(row) {
                chosen = Some(code.to_string());
            }
        }
        if (is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter)) && list.len() == 1 {
            chosen = Some(list[0].0.to_string());
        }
        let none = Rect::new(r.x + 14.0, r.y + r.h - 40.0, 160.0, 28.0);
        if ui.button(none, t("Don't show a flag"), true) {
            chosen = Some(String::new());
        }
        rrect_lines(area, 5.0, 1.0, BORDER);
        if chosen.is_some() || close || is_key_pressed(KeyCode::Escape) {
            p.open = false;
            ui.focus = None;
        }
    });
    chosen
}
