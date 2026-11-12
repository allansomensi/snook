//! Reaction pictures from the bundled atlas (Twemoji, CC-BY 4.0).

use std::cell::OnceCell;

use macroquad::prelude::*;
use snookered_proto::REACTIONS;

const SIZE: f32 = 72.0;
const COLUMNS: usize = 8;

thread_local! {
    static ATLAS: OnceCell<Texture2D> = const { OnceCell::new() };
}

fn atlas() -> Texture2D {
    ATLAS.with(|a| {
        a.get_or_init(|| {
            let tex =
                Texture2D::from_file_with_format(include_bytes!("../assets/reactions.png"), None);
            tex.set_filter(FilterMode::Linear);
            tex
        })
        .clone()
    })
}

/// Draws reaction `code` centred on `at`, `size` wide, with `alpha`.
/// False for an unknown code.
pub fn draw(code: &str, at: Vec2, size: f32, alpha: f32) -> bool {
    let Some(i) = REACTIONS.iter().position(|r| r.code == code) else { return false };
    let (col, row) = (i % COLUMNS, i / COLUMNS);
    let src = Rect::new(col as f32 * SIZE, row as f32 * SIZE, SIZE, SIZE);
    let a = alpha;
    draw_texture_ex(
        &atlas(),
        at.x - size / 2.0,
        at.y - size / 2.0,
        Color::new(1.0, 1.0, 1.0, a),
        DrawTextureParams {
            dest_size: Some(vec2(size, size)),
            source: Some(src),
            ..Default::default()
        },
    );
    true
}

/// Like [`draw`], in grey (a reaction still locked).
pub fn draw_locked(code: &str, at: Vec2, size: f32) {
    let Some(i) = REACTIONS.iter().position(|r| r.code == code) else { return };
    let (col, row) = (i % COLUMNS, i / COLUMNS);
    let src = Rect::new(col as f32 * SIZE, row as f32 * SIZE, SIZE, SIZE);
    let a = crate::paint::opacity();
    draw_texture_ex(
        &atlas(),
        at.x - size / 2.0,
        at.y - size / 2.0,
        Color::new(0.55, 0.5, 0.45, 0.3 * a),
        DrawTextureParams {
            dest_size: Some(vec2(size, size)),
            source: Some(src),
            ..Default::default()
        },
    );
}
