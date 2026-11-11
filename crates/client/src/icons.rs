//! Small vector icons drawn with shapes (no image files), so they stay
//! sharp at any interface size and fade with the HUD.

use macroquad::prelude::*;

#[allow(unused_imports)]
use crate::paint::{draw_circle, draw_circle_lines, draw_line, draw_rectangle, draw_triangle};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Trophy,
    Person,
    People,
    Gear,
    Power,
    /// Back to the default camera.
    CameraReset,
    Replay,
    Plus,
    Minus,
    Close,
    Back,
    Lock,
    Globe,
    Network,
    Robot,
    Medal,
    Star,
    Refresh,
    Table,
    Play,
}

fn arc(c: Vec2, r: f32, from: f32, to: f32, w: f32, color: Color) {
    let steps = 14;
    let mut prev = c + Vec2::from_angle(from) * r;
    for k in 1..=steps {
        let a = from + (to - from) * k as f32 / steps as f32;
        let p = c + Vec2::from_angle(a) * r;
        draw_line(prev.x, prev.y, p.x, p.y, w, color);
        prev = p;
    }
}

/// The upper half of a disc centred on `c`.
fn half_disc(c: Vec2, r: f32, color: Color) {
    let steps = 12;
    for k in 0..steps {
        let a0 = std::f32::consts::PI * (1.0 + k as f32 / steps as f32);
        let a1 = std::f32::consts::PI * (1.0 + (k + 1) as f32 / steps as f32);
        draw_triangle(c, c + Vec2::from_angle(a0) * r, c + Vec2::from_angle(a1) * r, color);
    }
}

fn line(a: Vec2, b: Vec2, w: f32, color: Color) {
    draw_line(a.x, a.y, b.x, b.y, w, color);
}

/// Draws `icon` centred on `c`, about `s` units across.
pub fn draw(icon: Icon, c: Vec2, s: f32, color: Color) {
    let h = s / 2.0;
    let w = (s * 0.1).max(1.4);
    let p = |x: f32, y: f32| c + vec2(x, y) * h;
    match icon {
        Icon::Trophy => {
            // Cup, handles, stem and base.
            draw_triangle(p(-0.55, -0.75), p(0.55, -0.75), p(0.0, 0.2), color);
            draw_rectangle(c.x - 0.55 * h, c.y - 0.85 * h, 1.1 * h, 0.35 * h, color);
            arc(p(-0.55, -0.45), 0.28 * h, 1.57, 4.71, w * 0.8, color);
            arc(p(0.55, -0.45), 0.28 * h, -1.57, 1.57, w * 0.8, color);
            line(p(0.0, 0.1), p(0.0, 0.55), w * 1.4, color);
            draw_rectangle(c.x - 0.45 * h, c.y + 0.55 * h, 0.9 * h, 0.25 * h, color);
        }
        Icon::Person => {
            draw_circle(c.x, c.y - 0.38 * h, 0.34 * h, color);
            half_disc(c + vec2(0.0, 0.85 * h), 0.68 * h, color);
        }
        Icon::People => {
            let dim = Color::new(color.r, color.g, color.b, color.a * 0.6);
            draw_circle(c.x + 0.38 * h, c.y - 0.4 * h, 0.27 * h, dim);
            half_disc(c + vec2(0.38 * h, 0.6 * h), 0.5 * h, dim);
            draw_circle(c.x - 0.25 * h, c.y - 0.3 * h, 0.31 * h, color);
            half_disc(c + vec2(-0.25 * h, 0.85 * h), 0.6 * h, color);
        }
        Icon::Gear => {
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::TAU / 8.0;
                let d = Vec2::from_angle(a);
                line(c + d * 0.45 * h, c + d * 0.85 * h, w * 2.2, color);
            }
            draw_circle(c.x, c.y, 0.6 * h, color);
            draw_circle(c.x, c.y, 0.25 * h, Color::new(0.0, 0.0, 0.0, 0.55));
        }
        Icon::Power => {
            arc(c, 0.65 * h, -1.0, 4.14, w, color);
            line(p(0.0, -0.95), p(0.0, -0.15), w, color);
        }
        Icon::Replay => {
            arc(c, 0.62 * h, -2.6, 2.2, w, color);
            let tip = c + Vec2::from_angle(-2.6) * 0.62 * h;
            draw_triangle(
                tip + vec2(-0.35, -0.05) * h,
                tip + vec2(0.25, -0.2) * h,
                tip + vec2(0.05, 0.38) * h,
                color,
            );
            draw_triangle(p(-0.12, -0.25), p(-0.12, 0.25), p(0.28, 0.0), color);
        }
        Icon::CameraReset => {
            // A camera body with its lens…
            let body = Rect::new(c.x - 0.78 * h, c.y - 0.42 * h, 1.2 * h, 0.95 * h);
            crate::ui::rrect(body, 0.16 * h, color);
            draw_rectangle(c.x - 0.45 * h, c.y - 0.6 * h, 0.4 * h, 0.2 * h, color);
            let lens = vec2(body.x + body.w / 2.0, body.y + body.h / 2.0);
            draw_circle(lens.x, lens.y, 0.28 * h, Color::new(0.0, 0.0, 0.0, 0.7));
            draw_circle(lens.x, lens.y, 0.15 * h, color);
            // …and a "go back" arrow curling round its corner.
            let ac = c + vec2(0.55 * h, 0.45 * h);
            arc(ac, 0.36 * h, -1.2, 2.2, w, color);
            let tip = ac + Vec2::from_angle(-1.2) * 0.36 * h;
            draw_triangle(
                tip + vec2(-0.26, -0.12) * h,
                tip + vec2(0.18, -0.2) * h,
                tip + vec2(0.0, 0.24) * h,
                color,
            );
        }
        Icon::Plus => {
            line(p(-0.65, 0.0), p(0.65, 0.0), w * 1.3, color);
            line(p(0.0, -0.65), p(0.0, 0.65), w * 1.3, color);
        }
        Icon::Minus => line(p(-0.65, 0.0), p(0.65, 0.0), w * 1.3, color),
        Icon::Close => {
            line(p(-0.55, -0.55), p(0.55, 0.55), w * 1.2, color);
            line(p(-0.55, 0.55), p(0.55, -0.55), w * 1.2, color);
        }
        Icon::Back => {
            line(p(-0.65, 0.0), p(0.65, 0.0), w * 1.2, color);
            line(p(-0.65, 0.0), p(-0.1, -0.55), w * 1.2, color);
            line(p(-0.65, 0.0), p(-0.1, 0.55), w * 1.2, color);
        }
        Icon::Lock => {
            arc(
                p(0.0, -0.25),
                0.38 * h,
                std::f32::consts::PI,
                std::f32::consts::TAU,
                w * 1.1,
                color,
            );
            line(p(-0.38, -0.25), p(-0.38, 0.05), w * 1.1, color);
            line(p(0.38, -0.25), p(0.38, 0.05), w * 1.1, color);
            crate::ui::rrect(Rect::new(c.x - 0.6 * h, c.y, 1.2 * h, 0.85 * h), 0.15 * h, color);
        }
        Icon::Globe => {
            draw_circle_lines(c.x, c.y, 0.8 * h, w, color);
            line(p(-0.8, 0.0), p(0.8, 0.0), w * 0.8, color);
            arc(c + vec2(0.9 * h, 0.0), 1.2 * h, 2.43, 3.85, w * 0.8, color);
            arc(c - vec2(0.9 * h, 0.0), 1.2 * h, -0.71, 0.71, w * 0.8, color);
        }
        Icon::Network => {
            crate::ui::rrect(
                Rect::new(c.x - 0.3 * h, c.y - 0.85 * h, 0.6 * h, 0.45 * h),
                0.08 * h,
                color,
            );
            for x in [-0.6f32, 0.0, 0.6] {
                crate::ui::rrect(
                    Rect::new(c.x + (x - 0.25) * h, c.y + 0.4 * h, 0.5 * h, 0.42 * h),
                    0.08 * h,
                    color,
                );
                line(p(x, 0.4), p(x, 0.05), w * 0.9, color);
            }
            line(p(-0.6, 0.05), p(0.6, 0.05), w * 0.9, color);
            line(p(0.0, -0.4), p(0.0, 0.05), w * 0.9, color);
        }
        Icon::Robot => {
            crate::ui::rrect(
                Rect::new(c.x - 0.7 * h, c.y - 0.45 * h, 1.4 * h, 1.1 * h),
                0.25 * h,
                color,
            );
            let hole = Color::new(0.0, 0.0, 0.0, 0.6);
            draw_circle(c.x - 0.3 * h, c.y + 0.0, 0.16 * h, hole);
            draw_circle(c.x + 0.3 * h, c.y + 0.0, 0.16 * h, hole);
            line(p(0.0, -0.45), p(0.0, -0.8), w, color);
            draw_circle(c.x, c.y - 0.85 * h, 0.12 * h, color);
        }
        Icon::Medal => {
            draw_triangle(p(-0.45, -0.95), p(-0.05, -0.95), p(0.0, -0.1), color);
            draw_triangle(p(0.45, -0.95), p(0.05, -0.95), p(0.0, -0.1), color);
            draw_circle(c.x, c.y + 0.35 * h, 0.5 * h, color);
            draw_circle_lines(
                c.x,
                c.y + 0.35 * h,
                0.3 * h,
                w * 0.7,
                Color::new(0.0, 0.0, 0.0, 0.35),
            );
        }
        Icon::Star => {
            let pts: Vec<Vec2> = (0..10)
                .map(|k| {
                    let r = if k % 2 == 0 { 0.95 } else { 0.42 };
                    c + Vec2::from_angle(
                        -std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::PI / 5.0,
                    ) * r
                        * h
                })
                .collect();
            for k in 0..10 {
                draw_triangle(c, pts[k], pts[(k + 1) % 10], color);
            }
        }
        Icon::Refresh => {
            arc(c, 0.62 * h, -0.4, 4.6, w, color);
            let tip = c + Vec2::from_angle(-0.4) * 0.62 * h;
            draw_triangle(
                tip + vec2(-0.3, -0.25) * h,
                tip + vec2(0.32, -0.08) * h,
                tip + vec2(0.02, 0.35) * h,
                color,
            );
        }
        Icon::Table => {
            crate::ui::rrect(
                Rect::new(c.x - 0.9 * h, c.y - 0.55 * h, 1.8 * h, 1.1 * h),
                0.18 * h,
                color,
            );
            draw_rectangle(
                c.x - 0.7 * h,
                c.y - 0.35 * h,
                1.4 * h,
                0.7 * h,
                Color::new(0.1, 0.45, 0.25, color.a),
            );
            for (x, y) in [
                (-0.7f32, -0.35f32),
                (0.0, -0.35),
                (0.7, -0.35),
                (-0.7, 0.35),
                (0.0, 0.35),
                (0.7, 0.35),
            ] {
                draw_circle(c.x + x * h, c.y + y * h, 0.1 * h, Color::new(0.0, 0.0, 0.0, color.a));
            }
        }
        Icon::Play => draw_triangle(p(-0.45, -0.7), p(-0.45, 0.7), p(0.7, 0.0), color),
    }
}
