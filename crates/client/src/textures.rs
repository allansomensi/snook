//! Procedurally generated textures (CPU side, RGBA8).
//!
//! The game ships without image files. Patterns are kept smooth (low
//! frequency) and the GPU textures are mipmapped, which avoids the shimmering
//! "grain" that fine noise produces when the camera moves.

use macroquad::math::Vec3;
use snook_core::{BallId, BallKind};

/// RGBA8 image data.
pub struct Pixels {
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
}

/// Maps texture coordinates to a direction on the unit sphere (y-up).
pub fn uv_to_dir(u: f32, v: f32) -> Vec3 {
    let phi = u * std::f32::consts::TAU;
    let theta = v * std::f32::consts::PI;
    Vec3::new(theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin())
}

/// Ball colour in sRGB.
pub fn ball_color(id: BallId) -> [u8; 3] {
    match id.0 {
        0 => [246, 242, 228],
        8 => [22, 22, 24],
        n => match (n - 1) % 8 {
            0 => [255, 204, 10],
            1 => [18, 58, 205],
            2 => [228, 28, 24],
            3 => [108, 36, 168],
            4 => [255, 118, 12],
            5 => [12, 142, 58],
            _ => [142, 26, 28],
        },
    }
}

const IVORY: [u8; 3] = [246, 242, 228];
const INK: [u8; 3] = [20, 20, 20];

const DIGITS: [[u8; 7]; 10] = [
    [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
    [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
    [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
    [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110],
    [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
    [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
    [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
    [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
    [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
    [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
];

/// True if the glyph cell at (`a`, `b`) (tangent-plane coordinates) is inked.
fn number_ink(number: u8, a: f32, b: f32) -> bool {
    let (digits, count) = if number >= 10 {
        ([(number / 10) as usize, (number % 10) as usize], 2)
    } else {
        ([number as usize, 0], 1)
    };
    let cols = count * 5 + (count - 1);
    let cell = 1.05 / 7.0;
    let width = cols as f32 * cell;
    let col = ((a + width / 2.0) / cell).floor();
    let row = ((0.525 - b) / cell).floor();
    if col < 0.0 || row < 0.0 || col >= cols as f32 || row >= 7.0 {
        return false;
    }
    let (col, row) = (col as usize, row as usize);
    let (digit, x) = (col / 6, col % 6);
    x != 5 && DIGITS[digits[digit]][row] & (1 << (4 - x)) != 0
}

/// Colour of a ball's surface in direction `p` (mesh-local, unit length).
/// Numbers sit on local ±Z; stripes run around the band |x| < 0.5.
fn ball_surface(id: BallId, p: Vec3) -> [u8; 3] {
    let color = ball_color(id);
    if id == BallId::CUE {
        let dots = [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z];
        return if dots.iter().any(|d| p.dot(*d) > 0.996) { [190, 30, 30] } else { color };
    }
    let spot_angle: f32 = 0.43;
    for (s, e1) in [(Vec3::Z, Vec3::X), (-Vec3::Z, -Vec3::X)] {
        if p.dot(s) > spot_angle.cos() {
            let scale = spot_angle.sin();
            let a = p.dot(e1) / scale;
            let b = p.dot(Vec3::Y) / scale;
            return if number_ink(id.0, a * 1.15, b * 1.15) { INK } else { IVORY };
        }
    }
    match id.kind() {
        BallKind::Stripe if p.x.abs() > 0.5 => IVORY,
        _ => color,
    }
}

/// Equirectangular texture for a numbered ball, `width` pixels wide.
pub fn ball(id: BallId, width: u16) -> Pixels {
    let w_px = width;
    let h_px = width / 2;
    const SS: u32 = 2;
    let mut rgba = Vec::with_capacity(w_px as usize * h_px as usize * 4);
    for y in 0..h_px as u32 {
        for x in 0..w_px as u32 {
            let mut acc = [0u32; 3];
            for sy in 0..SS {
                for sx in 0..SS {
                    let u = (x as f32 + (sx as f32 + 0.5) / SS as f32) / w_px as f32;
                    let v = (y as f32 + (sy as f32 + 0.5) / SS as f32) / h_px as f32;
                    let c = ball_surface(id, uv_to_dir(u, v));
                    for k in 0..3 {
                        acc[k] += c[k] as u32;
                    }
                }
            }
            let n = SS * SS;
            rgba.extend_from_slice(&[
                (acc[0] / n) as u8,
                (acc[1] / n) as u8,
                (acc[2] / n) as u8,
                255,
            ]);
        }
    }
    Pixels { width: w_px, height: h_px, rgba }
}

fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(374_761_393)
        ^ (y as u32).wrapping_mul(668_265_263)
        ^ seed.wrapping_mul(2_246_822_519);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    (h ^ (h >> 16)) as f32 / u32::MAX as f32
}

/// Smooth value noise, tileable with period `period` cells.
fn noise(x: f32, y: f32, period: i32, seed: u32) -> f32 {
    let (x0, y0) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let w = |v: i32| v.rem_euclid(period);
    let h = |xx: i32, yy: i32| hash(w(xx), w(yy), seed);
    let top = h(x0, y0) + (h(x0 + 1, y0) - h(x0, y0)) * s(fx);
    let bottom = h(x0, y0 + 1) + (h(x0 + 1, y0 + 1) - h(x0, y0 + 1)) * s(fx);
    top + (bottom - top) * s(fy)
}

fn fbm(x: f32, y: f32, period: i32, seed: u32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 0.5, 1.0, 0.0);
    for o in 0..octaves {
        sum += amp * noise(x * freq, y * freq, period * freq as i32, seed + o);
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    sum / norm
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    [0, 1, 2].map(|i| ((a[i] + (b[i] - a[i]) * t) * 255.0).round().clamp(0.0, 255.0) as u8)
}

fn image(size: u16, f: impl Fn(f32, f32) -> [u8; 3]) -> Pixels {
    let mut rgba = Vec::with_capacity(size as usize * size as usize * 4);
    for y in 0..size {
        for x in 0..size {
            let c = f(x as f32 / size as f32, y as f32 / size as f32);
            rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    Pixels { width: size, height: size, rgba }
}

/// Billiard cloth: almost uniform, with very gentle large-scale variation.
pub fn felt() -> Pixels {
    image(256, |u, v| {
        let n = fbm(u * 4.0, v * 4.0, 4, 3, 3);
        lerp3([0.16, 0.42, 0.10], [0.18, 0.46, 0.11], n)
    })
}

/// Fine, realistic wood: long thin grain lines running along `u`, gentle
/// figure, small dark pores, low contrast. Tileable.
pub fn fine_wood(light: [f32; 3], dark: [f32; 3], seed: u32) -> Pixels {
    const N: u16 = 512;
    let mut rgba = Vec::with_capacity(N as usize * N as usize * 4);
    for y in 0..N {
        for x in 0..N {
            let (u, v) = (x as f32 / N as f32, y as f32 / N as f32);
            // Slow, wide figure that bends the grain.
            let warp = fbm(u * 4.0, v * 4.0, 4, seed, 3);
            // Growth rings seen from the side: thin lines, many of them.
            let ring = ((v * 22.0 + warp * 1.6) * std::f32::consts::TAU).sin() * 0.5 + 0.5;
            let ring = ring.powf(6.0);
            // Very fine streaks that run the whole length.
            let streak = noise(0.0, v * 160.0 + warp * 6.0, 160, seed + 5) * 0.7
                + noise(0.0, v * 48.0, 48, seed + 11) * 0.3;
            // Broad, soft colour variation.
            let tone = fbm(u * 2.0, v * 2.0, 2, seed + 21, 2);
            let mut t = 0.30 + 0.28 * ring + 0.30 * (streak - 0.5) + 0.25 * (tone - 0.5);
            // Pores: tiny dark specks stretched along the grain.
            if hash(x as i32 / 3, y as i32, seed + 31) > 0.985 {
                t += 0.25;
            }
            let c = lerp3(light, dark, t);
            rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    Pixels { width: N, height: N, rgba }
}

/// Plank floor.
pub fn floor() -> Pixels {
    image(256, |u, v| {
        let planks = 4.0;
        let plank = (v * planks).floor();
        let within = v * planks - plank;
        let tone = hash(plank as i32, 0, 7) * 0.3;
        let grain = fbm(u * 2.0 + plank * 0.37, within * 3.0, 4, 11, 3) * 0.4;
        let seam = if within < 0.025 { 0.45 } else { 0.0 };
        lerp3([0.34, 0.21, 0.12], [0.13, 0.08, 0.05], tone + grain + seam)
    })
}

/// Soft round shadow: black with alpha falling off towards the edge.
pub fn shadow() -> Pixels {
    let size: u16 = 64;
    let mut rgba = Vec::with_capacity(size as usize * size as usize * 4);
    for y in 0..size {
        for x in 0..size {
            let dx = (x as f32 + 0.5) / size as f32 * 2.0 - 1.0;
            let dy = (y as f32 + 0.5) / size as f32 * 2.0 - 1.0;
            let d = (dx * dx + dy * dy).sqrt();
            let a = (1.0 - d).clamp(0.0, 1.0).powf(1.6);
            rgba.extend_from_slice(&[0, 0, 0, (a * 255.0) as u8]);
        }
    }
    Pixels { width: size, height: size, rgba }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_render() {
        assert!(number_ink(1, 0.0, 0.0));
        assert!(!number_ink(1, -0.45, 0.0));
        assert!(!number_ink(15, 0.0, 0.9));
    }

    #[test]
    fn ball_layout() {
        assert_eq!(ball_surface(BallId(3), Vec3::new(0.0, 0.3, 0.95).normalize()), IVORY);
        assert_eq!(ball_surface(BallId(11), Vec3::X), IVORY);
        assert_eq!(ball_surface(BallId(11), Vec3::Y), ball_color(BallId(11)));
        assert_eq!(ball_surface(BallId(3), Vec3::X), ball_color(BallId(3)));
    }

    #[test]
    fn textures_have_expected_size_and_tile() {
        let f = felt();
        assert_eq!(f.rgba.len(), 256 * 256 * 4);
        // Tileable noise: first and last columns are close.
        let px = |x: usize, y: usize| f.rgba[(y * 256 + x) * 4 + 1] as i32;
        assert!((px(0, 10) - px(255, 10)).abs() <= 3);
        let s = shadow();
        assert_eq!(s.rgba[3], 0, "corner is transparent");
        assert!(s.rgba[(32 * 64 + 32) * 4 + 3] > 200, "centre is dark");
    }
}
