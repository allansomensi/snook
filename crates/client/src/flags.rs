//! Country flags from the bundled atlas (flag-icons, MIT licence).

use std::cell::OnceCell;

use macroquad::prelude::*;
use snook_proto::{COUNTRIES, FLAG_COLUMNS, FLAG_H, FLAG_W, country_index};

thread_local! {
    static ATLAS: OnceCell<Option<Texture2D>> = const { OnceCell::new() };
}

fn atlas() -> Option<Texture2D> {
    ATLAS.with(|a| {
        a.get_or_init(|| {
            let tex = Texture2D::from_file_with_format(include_bytes!("../assets/flags.png"), None);
            tex.set_filter(FilterMode::Linear);
            Some(tex)
        })
        .clone()
    })
}

/// Draws the flag of `code` with its top-left corner at (`x`, `y`) and the
/// given height (4:3). Nothing is drawn for an unknown or empty code.
pub fn draw(code: &str, x: f32, y: f32, h: f32) -> bool {
    let Some(i) = country_index(code) else { return false };
    let Some(tex) = atlas() else { return false };
    let (col, row) = (i as u32 % FLAG_COLUMNS, i as u32 / FLAG_COLUMNS);
    let src = Rect::new((col * FLAG_W) as f32, (row * FLAG_H) as f32, FLAG_W as f32, FLAG_H as f32);
    let w = h * 4.0 / 3.0;
    let a = crate::paint::opacity();
    crate::paint::draw_rectangle(
        x - 0.5,
        y - 0.5,
        w + 1.0,
        h + 1.0,
        Color::new(0.0, 0.0, 0.0, 0.5),
    );
    draw_texture_ex(
        &tex,
        x,
        y,
        Color::new(1.0, 1.0, 1.0, a),
        DrawTextureParams { dest_size: Some(vec2(w, h)), source: Some(src), ..Default::default() },
    );
    true
}

/// Width of a flag drawn `h` high.
pub fn width(h: f32) -> f32 {
    h * 4.0 / 3.0
}

/// The country's name in the interface language.
pub fn name(code: &str) -> &'static str {
    match country_index(code) {
        Some(i) => match crate::i18n::lang() {
            crate::i18n::Lang::Portuguese => COUNTRIES[i].2,
            crate::i18n::Lang::English => COUNTRIES[i].1,
        },
        None => "",
    }
}

/// The country of the operating system's locale ("pt-BR" → "BR").
pub fn system_country() -> String {
    sys_locale::get_locale()
        .and_then(|l| {
            l.split(['-', '_'])
                .nth(1)
                .map(|c| c.to_uppercase())
                .filter(|c| c.len() == 2 && country_index(c).is_some())
        })
        .unwrap_or_default()
}
