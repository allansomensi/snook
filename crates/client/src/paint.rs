//! Drawing helpers with the same names as macroquad's shape functions, but
//! multiplied by a global opacity, so a whole layer of the interface (the
//! in-game HUD) can fade in and out smoothly.

use std::sync::atomic::{AtomicU32, Ordering};

use macroquad::prelude::{Color, Vec2};
use macroquad::shapes as mq;

static OPACITY: AtomicU32 = AtomicU32::new(0x3f80_0000);

/// Opacity applied to everything drawn through this module (and the UI text).
pub fn set_opacity(a: f32) {
    OPACITY.store(a.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
}

pub fn opacity() -> f32 {
    f32::from_bits(OPACITY.load(Ordering::Relaxed))
}

pub fn fade(c: Color) -> Color {
    Color::new(c.r, c.g, c.b, c.a * opacity())
}

pub fn draw_rectangle(x: f32, y: f32, w: f32, h: f32, c: Color) {
    mq::draw_rectangle(x, y, w, h, fade(c));
}

pub fn draw_circle(x: f32, y: f32, r: f32, c: Color) {
    mq::draw_circle(x, y, r, fade(c));
}

pub fn draw_circle_lines(x: f32, y: f32, r: f32, t: f32, c: Color) {
    mq::draw_circle_lines(x, y, r, t, fade(c));
}

pub fn draw_line(x1: f32, y1: f32, x2: f32, y2: f32, t: f32, c: Color) {
    mq::draw_line(x1, y1, x2, y2, t, fade(c));
}

pub fn draw_triangle(a: Vec2, b: Vec2, c: Vec2, color: Color) {
    mq::draw_triangle(a, b, c, fade(color));
}
