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

/// The colours of a ball set (see `snook_proto::shop`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub colors: [[u8; 3]; 7],
    pub eight: [u8; 3],
    pub ivory: [u8; 3],
}

impl Palette {
    /// The palette of a shop ball set (the classic one if unknown).
    pub fn of(code: &str) -> Palette {
        let item =
            snook_proto::shop::item(code).or_else(|| snook_proto::shop::item("balls_classic"));
        match item.map(|i| i.look) {
            Some(snook_proto::shop::Look::Balls { colors, eight, ivory }) => {
                Palette { colors, eight, ivory }
            }
            _ => Palette { colors: [[255, 204, 10]; 7], eight: [22, 22, 24], ivory: IVORY },
        }
    }
}

/// Ball colour in sRGB (classic set).
pub fn ball_color(id: BallId) -> [u8; 3] {
    ball_color_in(id, &Palette::of("balls_classic"))
}

pub fn ball_color_in(id: BallId, p: &Palette) -> [u8; 3] {
    match id.0 {
        0 => p.ivory,
        8 => p.eight,
        n => p.colors[((n - 1) % 8).min(6) as usize],
    }
}

const IVORY: [u8; 3] = [246, 242, 228];
const INK: [u8; 3] = [20, 20, 20];

/// A ball's number, rasterised from the game's bold font with real
/// anti-aliasing (a 5x7 bitmap font looked blocky up close): coverage 0..1
/// on a canvas centred on the number circle.
struct Label {
    w: usize,
    h: usize,
    cov: Vec<f32>,
    /// Size of a canvas pixel in spot-radius units.
    scale: f32,
}

impl Label {
    fn new(number: u8) -> Label {
        const PX: f32 = 192.0;
        let font = fontdue::Font::from_bytes(
            &include_bytes!("../assets/DejaVuSans-Bold.ttf")[..],
            fontdue::FontSettings::default(),
        )
        .expect("bundled font");
        let text = number.to_string();
        // Lay the digits out on a baseline, a little tighter than the font.
        let mut glyphs = Vec::new();
        let mut pen = 0.0f32;
        for c in text.chars() {
            let (m, bitmap) = font.rasterize(c, PX);
            glyphs.push((pen.round() as i32 + m.xmin, m, bitmap));
            pen += m.advance_width * 0.94;
        }
        let x0 = glyphs.iter().map(|(x, _, _)| *x).min().unwrap_or(0);
        let x1 = glyphs.iter().map(|(x, m, _)| x + m.width as i32).max().unwrap_or(1);
        let top = glyphs.iter().map(|(_, m, _)| m.ymin + m.height as i32).max().unwrap_or(1);
        let bottom = glyphs.iter().map(|(_, m, _)| m.ymin).min().unwrap_or(0);
        let (gw, gh) = ((x1 - x0) as usize, (top - bottom) as usize);
        // 6 and 9 are underlined, as on real balls, so they can't be confused.
        let underline = matches!(number, 6 | 9);
        let (bar_gap, bar_h) = ((gh as f32 * 0.07) as usize, (gh as f32 * 0.10).max(2.0) as usize);
        let extra = if underline { bar_gap + bar_h } else { 0 };
        let pad = 4;
        let (w, h) = (gw + pad * 2, gh + extra + pad * 2);
        let mut cov = vec![0.0f32; w * h];
        for (x, m, bitmap) in &glyphs {
            let ox = (x - x0) as usize + pad;
            let oy = (top - (m.ymin + m.height as i32)) as usize + pad;
            for r in 0..m.height {
                for c in 0..m.width {
                    let v = bitmap[r * m.width + c] as f32 / 255.0;
                    let cell = &mut cov[(oy + r) * w + ox + c];
                    *cell = cell.max(v);
                }
            }
        }
        if underline {
            let bw = (gw as f32 * 0.8) as usize;
            let bx = pad + (gw - bw) / 2;
            for r in 0..bar_h {
                for c in 0..bw {
                    cov[(pad + gh + bar_gap + r) * w + bx + c] = 1.0;
                }
            }
        }
        // The digits fill about 60% of the circle's height (narrower for two).
        let height = (gh + extra) as f32;
        let scale = (1.16 / height).min(1.52 / gw as f32);
        Label { w, h, cov, scale }
    }

    /// Ink coverage at (`a`, `b`) (right and up, in spot-radius units), bilinear.
    fn sample(&self, a: f32, b: f32) -> f32 {
        let x = a / self.scale + self.w as f32 / 2.0 - 0.5;
        let y = -b / self.scale + self.h as f32 / 2.0 - 0.5;
        let (xf, yf) = (x.floor(), y.floor());
        let (fx, fy) = (x - xf, y - yf);
        let at = |cx: f32, cy: f32| -> f32 {
            if cx < 0.0 || cy < 0.0 || cx >= self.w as f32 || cy >= self.h as f32 {
                0.0
            } else {
                self.cov[cy as usize * self.w + cx as usize]
            }
        };
        let top = at(xf, yf) + (at(xf + 1.0, yf) - at(xf, yf)) * fx;
        let bottom = at(xf, yf + 1.0) + (at(xf + 1.0, yf + 1.0) - at(xf, yf + 1.0)) * fx;
        top + (bottom - top) * fy
    }
}

/// How much of a pixel `px` wide lies on the inside of an edge `d` away
/// (positive inside): a smooth, alias-free boundary.
fn coverage(d: f32, px: f32) -> f32 {
    (d / px.max(1e-6) + 0.5).clamp(0.0, 1.0)
}

fn mix(a: [f32; 3], b: [u8; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] as f32 - a[i]) * t)
}

/// Colour of a ball's surface in direction `p` (mesh-local, unit length),
/// for a texel covering `px` radians. Numbers sit on local ±Z; stripes run
/// around the band |x| < 0.5.
fn ball_surface_f(id: BallId, p: Vec3, pal: &Palette, label: Option<&Label>, px: f32) -> [f32; 3] {
    let color = ball_color_in(id, pal);
    let ivory = pal.ivory;
    let f = |c: [u8; 3]| c.map(|v| v as f32);
    if id == BallId::CUE {
        // Six small red dots (they show the spin).
        let dots = [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z];
        let near = dots.iter().map(|d| p.dot(*d).clamp(-1.0, 1.0).acos()).fold(9.0, f32::min);
        return mix(f(color), [190, 30, 30], coverage(0.0895 - near, px));
    }
    // The body: the colour, or ivory outside a stripe's band.
    let band = match id.kind() {
        BallKind::Stripe => coverage(0.5 - p.x.abs(), px * (1.0 - p.x * p.x).sqrt().max(0.2)),
        _ => 1.0,
    };
    let mut c = mix(f(ivory), color, band);
    // A large number circle with big, bold digits: readable from across the table.
    let spot_angle: f32 = 0.56;
    for (s, e1) in [(Vec3::Z, Vec3::X), (-Vec3::Z, -Vec3::X)] {
        let angle = p.dot(s).clamp(-1.0, 1.0).acos();
        if angle < spot_angle + px {
            let inside = coverage(spot_angle - angle, px);
            let scale = spot_angle.sin();
            let a = p.dot(e1) / scale;
            let b = p.dot(Vec3::Y) / scale;
            let ink = label.map_or(0.0, |l| l.sample(a, b));
            let face = mix(f(ivory), INK, ink);
            c = [0, 1, 2].map(|i| c[i] + (face[i] - c[i]) * inside);
        }
    }
    c
}

/// [`ball_surface_f`] at the centre of a texel of a 1024-wide texture.
#[cfg(test)]
fn ball_surface(id: BallId, p: Vec3, pal: &Palette) -> [u8; 3] {
    let label = (id.0 > 0).then(|| Label::new(id.0));
    ball_surface_f(id, p, pal, label.as_ref(), std::f32::consts::TAU / 1024.0)
        .map(|v| v.round().clamp(0.0, 255.0) as u8)
}

/// Equirectangular texture for a numbered ball, `width` pixels wide.
pub fn ball(id: BallId, width: u16, palette: &Palette) -> Pixels {
    let w_px = width;
    let h_px = width / 2;
    let label = (id.0 > 0).then(|| Label::new(id.0));
    // Edges are anti-aliased analytically; two samples a side smooth the
    // digits further and the poles, where texels are narrow.
    const SS: u32 = 2;
    let px = std::f32::consts::TAU / w_px as f32;
    let mut rgba = Vec::with_capacity(w_px as usize * h_px as usize * 4);
    for y in 0..h_px as u32 {
        for x in 0..w_px as u32 {
            let mut acc = [0f32; 3];
            for sy in 0..SS {
                for sx in 0..SS {
                    let u = (x as f32 + (sx as f32 + 0.5) / SS as f32) / w_px as f32;
                    let v = (y as f32 + (sy as f32 + 0.5) / SS as f32) / h_px as f32;
                    let c = ball_surface_f(id, uv_to_dir(u, v), palette, label.as_ref(), px);
                    for k in 0..3 {
                        acc[k] += c[k];
                    }
                }
            }
            let n = (SS * SS) as f32;
            let px8 = acc.map(|v| (v / n).round().clamp(0.0, 255.0) as u8);
            rgba.extend_from_slice(&[px8[0], px8[1], px8[2], 255]);
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

/// Billiard cloth: a deep tournament green, almost uniform, with very gentle
/// large-scale variation and a fine nap that shows up close.
/// Cloth of the given base colour (sRGB): soft mottling and a fine nap.
pub fn felt(base: [u8; 3]) -> Pixels {
    // The classic green [24, 96, 48] maps to the original tones.
    let b = base.map(|c| c as f32 / 255.0);
    let dark = [b[0] * 0.90, b[1] * 0.95, b[2] * 0.90];
    let light = [b[0] * 1.06, b[1] * 1.06, b[2] * 1.02];
    image(512, move |u, v| {
        let n = fbm(u * 4.0, v * 4.0, 4, 3, 3);
        let nap = fbm(u * 96.0, v * 96.0, 96, 7, 2) - 0.5;
        let c = lerp3(dark, light, n);
        let k = 1.0 + nap * 0.07;
        c.map(|x| (x as f32 * k).round().clamp(0.0, 255.0) as u8)
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
/// A soft white disc fading out (light on a wall).
pub fn glow() -> Pixels {
    let size: u16 = 64;
    let mut rgba = Vec::with_capacity(size as usize * size as usize * 4);
    for y in 0..size {
        for x in 0..size {
            let dx = (x as f32 + 0.5) / size as f32 * 2.0 - 1.0;
            let dy = (y as f32 + 0.5) / size as f32 * 2.0 - 1.0;
            let d = (dx * dx + dy * dy).sqrt();
            let a = (1.0 - d).clamp(0.0, 1.0).powf(2.0);
            rgba.extend_from_slice(&[255, 255, 255, (a * 255.0) as u8]);
        }
    }
    Pixels { width: size, height: size, rgba }
}

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
        let one = Label::new(1);
        assert!(one.sample(0.0, 0.0) > 0.9, "the 1's stem is in the middle");
        assert!(one.sample(-0.45, -0.1) < 0.1);
        assert!(Label::new(15).sample(0.0, 0.9) < 0.01, "nothing above the digits");
        // 6 and 9 are underlined.
        assert!(Label::new(6).h > Label::new(8).h);
    }

    #[test]
    fn ball_layout() {
        let classic = Palette::of("balls_classic");
        assert_eq!(
            ball_surface(BallId(3), Vec3::new(0.33, 0.33, 0.88).normalize(), &classic),
            IVORY
        );
        assert_eq!(
            ball_surface(BallId(3), Vec3::Z, &Palette::of("balls_classic")),
            INK,
            "the number is in the middle of the spot"
        );
        assert_eq!(ball_surface(BallId(11), Vec3::X, &Palette::of("balls_classic")), IVORY);
        assert_eq!(
            ball_surface(BallId(11), Vec3::Y, &Palette::of("balls_classic")),
            ball_color(BallId(11))
        );
        assert_eq!(
            ball_surface(BallId(3), Vec3::X, &Palette::of("balls_classic")),
            ball_color(BallId(3))
        );
    }

    #[test]
    fn textures_have_expected_size_and_tile() {
        let f = felt([24, 96, 48]);
        assert_eq!(f.rgba.len(), 512 * 512 * 4);
        // Tileable noise: first and last columns are close.
        let px = |x: usize, y: usize| f.rgba[(y * 512 + x) * 4 + 1] as i32;
        assert!((px(0, 10) - px(511, 10)).abs() <= 6);
        let s = shadow();
        assert_eq!(s.rgba[3], 0, "corner is transparent");
        assert!(s.rgba[(32 * 64 + 32) * 4 + 3] > 200, "centre is dark");
    }
}
