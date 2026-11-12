//! Winning effects (sold in the shop): what happens over the pocket when a
//! player wins the game on the 8. Drawn in 2D over the table, from a seed and
//! the age of the effect (no state per particle).

use macroquad::prelude::*;
use snook_proto::shop::{EffectKind, Look};

/// How long an effect plays, in seconds.
pub const LENGTH: f32 = 2.6;

fn hash(i: u32, k: u32) -> f32 {
    let mut h = i.wrapping_mul(0x9E37_79B9) ^ k.wrapping_mul(0x85EB_CA6B);
    h = (h ^ (h >> 15)).wrapping_mul(0x2C1B_3C6D);
    h = (h ^ (h >> 12)).wrapping_mul(0x297A_2D39);
    (h ^ (h >> 15)) as f32 / u32::MAX as f32
}

fn rgb(c: [u8; 3], a: f32) -> Color {
    Color::from_rgba(c[0], c[1], c[2], (a.clamp(0.0, 1.0) * 255.0) as u8)
}

/// Draws an effect started `age` seconds ago at `origin` (the pocket on
/// screen), `scale` pixels per unit (bigger on a bigger screen).
pub fn draw_at(kind: EffectKind, color: [u8; 3], origin: Vec2, age: f32, scale: f32) {
    draw_clipped(kind, color, origin, age, scale, None)
}

/// [`draw_at`], keeping every particle inside `clip`.
fn draw_clipped(
    kind: EffectKind,
    color: [u8; 3],
    origin: Vec2,
    age: f32,
    scale: f32,
    clip: Option<Rect>,
) {
    let inside = |p: Vec2| clip.is_none_or(|r| r.contains(p));
    let draw_circle = |x: f32, y: f32, r: f32, c: Color| {
        if inside(vec2(x, y)) {
            crate::paint::draw_circle(x, y, r, c)
        }
    };
    let draw_line = |x1: f32, y1: f32, x2: f32, y2: f32, t: f32, c: Color| {
        if inside(vec2(x1, y1)) && inside(vec2(x2, y2)) {
            crate::paint::draw_line(x1, y1, x2, y2, t, c)
        }
    };
    let draw_rectangle = |x: f32, y: f32, w: f32, h: f32, c: Color| {
        if inside(vec2(x, y)) && inside(vec2(x + w, y + h)) {
            crate::paint::draw_rectangle(x, y, w, h, c)
        }
    };
    let draw_circle_lines = |x: f32, y: f32, r: f32, t: f32, c: Color| {
        let ok = clip.is_none_or(|cr| {
            x - r >= cr.x && x + r <= cr.x + cr.w && y - r >= cr.y && y + r <= cr.y + cr.h
        });
        if ok {
            crate::paint::draw_circle_lines(x, y, r, t, c)
        }
    };
    let draw_ellipse = |x: f32, y: f32, w: f32, h: f32, c: Color| {
        if inside(vec2(x, y)) {
            crate::paint::draw_ellipse(x, y, w, h, c)
        }
    };
    let t = age / LENGTH;
    if !(0.0..1.0).contains(&t) {
        return;
    }
    let fade = 1.0 - smooth(0.7, 1.0, t);
    match kind {
        EffectKind::None => {}
        EffectKind::Sparks => {
            for i in 0..48 {
                let a = hash(i, 1) * std::f32::consts::TAU;
                let speed = (0.6 + hash(i, 2) * 1.2) * scale;
                let life = 0.5 + hash(i, 3) * 0.5;
                if t > life {
                    continue;
                }
                let k = t / life;
                let d = vec2(a.cos(), a.sin() - 0.6) * speed * k + vec2(0.0, 0.9 * scale * k * k);
                let p = origin + d;
                let tail = p - vec2(a.cos(), a.sin() - 0.6) * scale * 0.08;
                let c = rgb(color, (1.0 - k) * 1.2);
                draw_line(tail.x, tail.y, p.x, p.y, 2.0, c);
                draw_circle(p.x, p.y, 1.8, Color::new(1.0, 1.0, 1.0, (1.0 - k) * 0.9));
            }
        }
        EffectKind::Confetti => {
            let palette = [
                [255, 90, 90],
                [255, 210, 80],
                [90, 200, 255],
                [120, 230, 140],
                [220, 120, 255],
                color,
            ];
            for i in 0..70 {
                let x0 = (hash(i, 1) - 0.5) * 3.0 * scale;
                let fall = (0.4 + hash(i, 2) * 0.8) * scale;
                let sway = (age * (2.0 + hash(i, 3) * 3.0) + hash(i, 4) * 6.0).sin() * 0.12 * scale;
                let p = origin + vec2(x0 + sway, -1.4 * scale + fall * t * 2.2);
                let spin = (age * 6.0 + hash(i, 5) * 6.0).sin();
                let c = rgb(palette[i as usize % palette.len()], fade);
                let (w, h) = (0.05 * scale * spin.abs().max(0.2), 0.03 * scale);
                draw_rectangle(p.x - w / 2.0, p.y - h / 2.0, w, h, c);
            }
        }
        EffectKind::Shockwave => {
            for k in 0..3 {
                let tk = t * 1.4 - k as f32 * 0.18;
                if !(0.0..1.0).contains(&tk) {
                    continue;
                }
                let r = tk * 1.1 * scale;
                draw_circle_lines(
                    origin.x,
                    origin.y,
                    r,
                    3.0 * (1.0 - tk) + 1.0,
                    rgb(color, 1.0 - tk),
                );
                draw_circle_lines(
                    origin.x,
                    origin.y,
                    r * 0.96,
                    1.5,
                    Color::new(1.0, 1.0, 1.0, (1.0 - tk) * 0.6),
                );
            }
        }
        EffectKind::Flames => {
            for i in 0..60 {
                let cycle = 0.6 + hash(i, 1) * 0.5;
                let k = ((age + hash(i, 2) * cycle) / cycle).fract();
                let x =
                    (hash(i, 3) - 0.5) * 0.35 * scale + (age * 9.0 + i as f32).sin() * 0.03 * scale;
                let p = origin + vec2(x * (1.0 - k * 0.6), -k * 0.75 * scale);
                let size = (1.0 - k) * 0.07 * scale + 1.0;
                let hot = [255, (230.0 - k * 160.0) as u8, (120.0 - k * 120.0).max(0.0) as u8];
                let c = rgb(if k < 0.3 { [255, 240, 180] } else { hot }, (1.0 - k) * fade);
                draw_circle(p.x, p.y, size, c);
            }
            let _ = color;
        }
        EffectKind::Coins => {
            for i in 0..26 {
                let a = (hash(i, 1) - 0.5) * 1.6;
                let speed = (1.3 + hash(i, 2) * 0.9) * scale;
                let k = t * 1.2;
                let p =
                    origin + vec2(a.sin() * speed * k, -a.cos() * speed * k + 1.8 * scale * k * k);
                let squash = (age * 8.0 + hash(i, 3) * 6.0).cos().abs().max(0.15);
                let r = 0.05 * scale;
                let c = rgb(color, fade);
                draw_ellipse(p.x, p.y, r * squash, r, c);
                draw_ellipse(p.x, p.y, r * squash * 0.6, r * 0.6, rgb([255, 240, 160], fade));
            }
        }
        EffectKind::Fireworks => {
            for burst in 0..3u32 {
                let start = burst as f32 * 0.35;
                let local = age - start;
                if local < 0.0 {
                    continue;
                }
                let target = origin
                    + vec2(
                        (hash(burst, 9) - 0.5) * 1.6 * scale,
                        -(0.9 + hash(burst, 8) * 0.5) * scale,
                    );
                let rise = 0.45;
                if local < rise {
                    let k = local / rise;
                    let p = origin.lerp(target, k);
                    draw_circle(p.x, p.y, 2.2, Color::new(1.0, 0.95, 0.8, 1.0));
                    continue;
                }
                let k = ((local - rise) / 1.3).min(1.0);
                let hue = [[255, 120, 120], [130, 220, 255], color][burst as usize];
                for i in 0..36 {
                    let a = i as f32 / 36.0 * std::f32::consts::TAU + hash(i, burst) * 0.2;
                    let d =
                        vec2(a.cos(), a.sin()) * k * 0.45 * scale + vec2(0.0, k * k * 0.25 * scale);
                    let p = target + d;
                    draw_circle(p.x, p.y, 2.0 * (1.0 - k) + 0.6, rgb(hue, (1.0 - k) * 1.3));
                }
            }
        }
    }
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A looping preview in the shop.
pub fn draw_preview(kind: EffectKind, color: [u8; 3], r: Rect, time: f32) {
    let age = time % (LENGTH + 0.4);
    let origin = vec2(r.x + r.w / 2.0, r.y + r.h * 0.82);
    // A pocket for the effect to come out of.
    crate::paint::draw_circle(origin.x, origin.y, r.h * 0.11, Color::new(0.02, 0.02, 0.02, 1.0));
    let scale = if kind == EffectKind::Confetti { r.h * 0.55 } else { r.h * 0.75 };
    draw_clipped(kind, color, origin, age, scale, Some(r));
}

/// An effect playing over the table.
pub struct WinEffect {
    kind: EffectKind,
    color: [u8; 3],
    /// The pocket, in the 3D world.
    pub at: Vec3,
    age: f32,
}

impl WinEffect {
    /// The effect of a shop item (`None` for no effect).
    pub fn of(code: &str, at: Vec3) -> Option<WinEffect> {
        match snook_proto::shop::item(code)?.look {
            Look::Effect { kind, color } if kind != EffectKind::None => {
                Some(WinEffect { kind, color, at, age: 0.0 })
            }
            _ => None,
        }
    }

    /// Draws at the pocket's place on screen; false once it is over.
    pub fn draw(&mut self, dt: f32, screen: Option<Vec2>, scale: f32) -> bool {
        self.age += dt;
        if let Some(p) = screen {
            draw_at(self.kind, self.color, p, self.age, scale);
        }
        self.age < LENGTH
    }
}
