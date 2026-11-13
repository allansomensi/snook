//! The Snookered emblem, drawn procedurally: a pool table seen from above
//! (walnut rails, brass pocket castings, green cloth) with a black ball
//! whose number circle carries an "S".
//!
//! The same picture is the window icon, the menu's emblem and, exported to
//! files, the executable's and the installer's icon (`assets/icon/`; see
//! the `export_icons` test to regenerate them after a change here).

use macroquad::miniquad::conf::Icon;

#[derive(Clone, Copy)]
struct Rgba([f32; 4]);

impl Rgba {
    const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Rgba([r, g, b, 1.0])
    }

    /// Paints `top` over `self` with coverage `a` (0..1).
    fn over(self, top: Rgba, a: f32) -> Rgba {
        let a = a.clamp(0.0, 1.0) * top.0[3];
        let mut o = self.0;
        for (k, v) in o.iter_mut().enumerate().take(3) {
            *v += (top.0[k] - *v) * a;
        }
        o[3] += (1.0 - o[3]) * a;
        Rgba(o)
    }

    fn scale(self, k: f32) -> Rgba {
        let c = self.0;
        Rgba([c[0] * k, c[1] * k, c[2] * k, c[3]])
    }

    fn mix(self, other: Rgba, t: f32) -> Rgba {
        let t = t.clamp(0.0, 1.0);
        let (a, b) = (self.0, other.0);
        Rgba([
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
            a[3] + (b[3] - a[3]) * t,
        ])
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn len(x: f32, y: f32) -> f32 {
    (x * x + y * y).sqrt()
}

/// Signed distance to a rounded square of half size `b` and corner radius `r`.
fn rounded_square(x: f32, y: f32, b: f32, r: f32) -> f32 {
    let (qx, qy) = (x.abs() - (b - r), y.abs() - (b - r));
    len(qx.max(0.0), qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

const WALNUT_LIGHT: Rgba = Rgba::rgb(0.47, 0.26, 0.12);
const WALNUT_DARK: Rgba = Rgba::rgb(0.20, 0.10, 0.045);
const BRASS_LIGHT: Rgba = Rgba::rgb(1.0, 0.88, 0.58);
const BRASS_DARK: Rgba = Rgba::rgb(0.62, 0.42, 0.16);
const CLOTH_LIGHT: Rgba = Rgba::rgb(0.16, 0.56, 0.33);
const CLOTH_DARK: Rgba = Rgba::rgb(0.035, 0.22, 0.12);
const IVORY: Rgba = Rgba::rgb(0.97, 0.94, 0.86);
const INK: Rgba = Rgba::rgb(0.06, 0.05, 0.05);

/// Half size of the table (the picture spans -1..1).
const OUTER: f32 = 0.94;
/// Where the cloth starts (the rails are between this and `OUTER`).
const CLOTH: f32 = 0.70;
/// The ball: centre and radius.
const BALL: (f32, f32, f32) = (0.0, 0.0, 0.47);

/// The colour of the emblem at `(x, y)` (-1..1, y down), with alpha.
fn shade(x: f32, y: f32, px: f32) -> Rgba {
    let mut c = Rgba([0.0; 4]);
    let d = rounded_square(x, y, OUTER, 0.30);
    if d > px {
        return c;
    }
    // Walnut rails: lighter at the top, a soft bevel along the edge.
    let grain = 0.5 + 0.5 * ((x * 9.0 + y * 2.0).sin() * 0.5 + (x * 23.0 - y * 5.0).sin() * 0.5);
    let wood = WALNUT_LIGHT.mix(WALNUT_DARK, 0.35 + 0.45 * (y + 1.0) / 2.0 + 0.12 * grain);
    let bevel = smoothstep(-0.10, 0.0, d);
    c = c.over(wood.mix(Rgba::rgb(0.10, 0.05, 0.02), bevel * 0.6), smoothstep(px, -px, d));
    // A thin brass inlay around the cloth.
    let inner = rounded_square(x, y, CLOTH + 0.045, 0.16);
    let brass = BRASS_LIGHT.mix(BRASS_DARK, (y + x * 0.4 + 1.0) / 2.0);
    let ring = smoothstep(px, -px, inner) * smoothstep(-px, px, inner + 0.035);
    c = c.over(brass, ring);
    // The cloth: lit from the upper left, darker towards the cushions.
    let cloth_d = rounded_square(x, y, CLOTH, 0.12);
    let lit = len(x + 0.30, y + 0.35) / 1.25;
    let mut cloth = CLOTH_LIGHT.mix(CLOTH_DARK, smoothstep(0.0, 1.0, lit));
    // Cushion shade along the edge.
    cloth = cloth.scale(0.75 + 0.25 * smoothstep(0.0, -0.10, cloth_d));
    // The ball's shadow, down and to the right.
    let (bx, by, br) = BALL;
    let s = len((x - bx - 0.09) / 1.05, (y - by - 0.12) / 0.95) / br;
    cloth = cloth.scale(1.0 - 0.55 * (1.0 - smoothstep(0.75, 1.12, s)));
    c = c.over(cloth, smoothstep(px, -px, cloth_d));
    // Brass pocket castings in the corners, with the dark pocket inside.
    for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let (cx, cy) = (sx * 0.675, sy * 0.675);
        let pd = len(x - cx, y - cy);
        let shine = 0.5 + 0.5 * (-(x - cx) - (y - cy)) / pd.max(1e-4);
        c = c.over(BRASS_DARK.mix(BRASS_LIGHT, shine), smoothstep(0.165 + px, 0.165 - px, pd));
        let hole = Rgba::rgb(0.02, 0.015, 0.01).mix(Rgba::rgb(0.10, 0.07, 0.05), pd / 0.115);
        c = c.over(hole, smoothstep(0.115 + px, 0.115 - px, pd));
    }
    // The ball.
    let (u, v) = ((x - bx) / br, (y - by) / br);
    let r2 = u * u + v * v;
    let edge = smoothstep(1.0 + px / br, 1.0 - px / br, r2.sqrt());
    if edge > 0.0 {
        let n = [u, v, (1.0 - r2.min(1.0)).sqrt()];
        let light = norm([-0.80, -0.95, 0.55]);
        let diffuse = dot(n, light).max(0.0);
        // The number circle faces the viewer, a little up and to the left.
        let face = norm([-0.10, -0.13, 0.98]);
        let t1 = norm(cross([0.0, 1.0, 0.0], face));
        let t2 = cross(face, t1);
        let (lu, lv) = (dot(n, t1), dot(n, t2));
        let on_face = dot(n, face);
        let radius = 0.52;
        let fpx = px / br / radius * 1.4;
        let face_a = if on_face > 0.0 {
            smoothstep(radius + fpx * radius, radius - fpx * radius, len(lu, lv))
        } else {
            0.0
        };
        let mut base = INK.over(IVORY, face_a);
        let s = letter_s(lu / radius, -lv / radius, fpx);
        base = base.over(INK, s * face_a);
        let mut col = base.scale(0.30 + 0.80 * diffuse);
        // Rim light from the cloth and a sharp highlight.
        col = col.over(
            Rgba::rgb(0.20, 0.55, 0.35),
            0.25 * smoothstep(0.7, 1.0, r2.sqrt()) * (v.max(0.0)),
        );
        let half = norm([light[0], light[1], light[2] + 1.0]);
        let spec = dot(n, half).max(0.0).powf(90.0);
        col = col.over(Rgba::rgb(1.0, 1.0, 1.0), spec * 0.95);
        let soft = dot(n, half).max(0.0).powf(8.0);
        col = col.over(Rgba::rgb(1.0, 1.0, 1.0), soft * 0.10);
        c = c.over(col, edge);
    }
    c
}

/// Coverage of an "S" in a unit circle (x right, y up), `px` the pixel size.
fn letter_s(x: f32, y: f32, px: f32) -> f32 {
    let r = 0.29;
    let w = 0.12;
    let arc = |cx: f32, cy: f32, keep: &dyn Fn(f32) -> bool| -> f32 {
        let (dx, dy) = (x - cx, y - cy);
        let a = dy.atan2(dx).to_degrees();
        if !keep(a) {
            return 0.0;
        }
        let d = (len(dx, dy) - r).abs() - w;
        smoothstep(px, -px, d)
    };
    // Upper bowl: open on the lower right (angles counter-clockwise, y up).
    let top = arc(0.0, r * 0.95, &|a| !(-90.0..38.0).contains(&a));
    // Lower bowl: open on the upper left.
    let bottom = arc(0.0, -r * 0.95, &|a| !(90.0..180.1).contains(&a) && a > -142.0);
    top.max(bottom)
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt();
    [a[0] / l, a[1] / l, a[2] / l]
}

/// The emblem as `size`×`size` RGBA pixels (straight alpha).
pub fn render(size: usize) -> Vec<u8> {
    let ss = if size <= 64 { 5 } else { 3 };
    let mut out = vec![0u8; size * size * 4];
    let px = 2.0 / size as f32 * 0.5;
    for y in 0..size {
        for x in 0..size {
            let mut acc = [0f32; 4];
            for sy in 0..ss {
                for sx in 0..ss {
                    let fx = (x as f32 + (sx as f32 + 0.5) / ss as f32) / size as f32 * 2.0 - 1.0;
                    let fy = (y as f32 + (sy as f32 + 0.5) / ss as f32) / size as f32 * 2.0 - 1.0;
                    let c = shade(fx, fy, px).0;
                    // Premultiplied while averaging.
                    for k in 0..3 {
                        acc[k] += c[k] * c[3];
                    }
                    acc[3] += c[3];
                }
            }
            let n = (ss * ss) as f32;
            let i = (y * size + x) * 4;
            let a = acc[3] / n;
            for k in 0..3 {
                let v = if acc[3] > 0.0 { acc[k] / acc[3] } else { 0.0 };
                out[i + k] = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
            out[i + 3] = (a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
    }
    out
}

pub fn window_icon() -> Icon {
    let mut icon = Icon { small: [0; 1024], medium: [0; 4096], big: [0; 16384] };
    icon.small.copy_from_slice(&render(16));
    icon.medium.copy_from_slice(&render(32));
    icon.big.copy_from_slice(&render(64));
    icon
}

/// Draws the emblem (menu, lounge, loading screen) at `(x, y)`, `size` wide.
pub fn draw(x: f32, y: f32, size: f32) {
    use macroquad::prelude::*;
    thread_local! {
        static EMBLEM: std::cell::OnceCell<Texture2D> = const { std::cell::OnceCell::new() };
    }
    let tex = EMBLEM.with(|e| {
        e.get_or_init(|| {
            let tex = Texture2D::from_rgba8(256, 256, &render(256));
            tex.set_filter(FilterMode::Linear);
            tex
        })
        .clone()
    });
    let alpha = crate::paint::opacity();
    draw_texture_ex(
        &tex,
        x,
        y,
        Color::new(1.0, 1.0, 1.0, alpha),
        DrawTextureParams { dest_size: Some(vec2(size, size)), ..Default::default() },
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn icon_has_transparent_corners_and_opaque_middle() {
        let px = super::render(32);
        assert_eq!(px[3], 0, "corner is transparent");
        let mid = (16 * 32 + 16) * 4;
        assert_eq!(px[mid + 3], 255);
    }

    /// Writes the emblem as PNG files and a Windows .ico into
    /// `crates/client/assets/icon/`:
    /// `cargo test --release -p snookered export_icons -- --ignored`.
    #[test]
    #[ignore]
    fn export_icons() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icon");
        std::fs::create_dir_all(&dir).unwrap();
        let png = |size: usize| -> Vec<u8> {
            let mut bytes = Vec::new();
            {
                let mut enc = png::Encoder::new(&mut bytes, size as u32, size as u32);
                enc.set_color(png::ColorType::Rgba);
                enc.set_depth(png::BitDepth::Eight);
                enc.set_compression(png::Compression::Best);
                let mut w = enc.write_header().unwrap();
                w.write_image_data(&super::render(size)).unwrap();
            }
            bytes
        };
        for size in [64, 128, 256, 512, 1024] {
            std::fs::write(dir.join(format!("snookered-{size}.png")), png(size)).unwrap();
        }
        // A .ico with PNG entries (Windows Vista and later).
        let sizes = [16usize, 20, 24, 32, 40, 48, 64, 128, 256];
        let images: Vec<Vec<u8>> = sizes.iter().map(|&s| png(s)).collect();
        let mut ico = Vec::new();
        ico.extend_from_slice(&[0, 0, 1, 0]);
        ico.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
        let mut offset = 6 + 16 * sizes.len();
        for (&s, img) in sizes.iter().zip(&images) {
            let b = if s >= 256 { 0 } else { s as u8 };
            ico.extend_from_slice(&[b, b, 0, 0]);
            ico.extend_from_slice(&1u16.to_le_bytes());
            ico.extend_from_slice(&32u16.to_le_bytes());
            ico.extend_from_slice(&(img.len() as u32).to_le_bytes());
            ico.extend_from_slice(&(offset as u32).to_le_bytes());
            offset += img.len();
        }
        for img in &images {
            ico.extend_from_slice(img);
        }
        std::fs::write(dir.join("snookered.ico"), ico).unwrap();

        // The Windows installer's pictures (WiX: 493x58 banner, 493x312 welcome page).
        let banner = picture(493, 58, |_, _| [1.0, 1.0, 1.0], &[(46, 493 - 52, 6)]);
        std::fs::write(dir.join("wix-banner.bmp"), bmp(493, 58, &banner)).unwrap();
        // The welcome page: the emblem on cloth on the left (the text goes on the right).
        let dialog = picture(
            493,
            312,
            |x, y| {
                if x >= 164 {
                    return [1.0, 1.0, 1.0];
                }
                let (u, v) = (x as f32 / 164.0 - 0.5, y as f32 / 312.0 - 0.35);
                let k = (1.0 - (u * u + v * v).sqrt() * 1.1).clamp(0.0, 1.0);
                let felt = [0.03 + 0.10 * k, 0.17 + 0.30 * k, 0.09 + 0.16 * k];
                // A brass line along the edge of the cloth.
                if x >= 160 { [0.80, 0.62, 0.30] } else { felt }
            },
            &[(120, 20, 64)],
        );
        std::fs::write(dir.join("wix-dialog.bmp"), bmp(493, 312, &dialog)).unwrap();
    }

    /// Writes the Microsoft Store package's pictures into
    /// `crates/client/msix/Assets/`, at the scales and target sizes Windows
    /// asks for: `cargo test -p snookered export_msix_assets -- --ignored`.
    #[test]
    #[ignore]
    fn export_msix_assets() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("msix/Assets");
        if dir.exists() {
            std::fs::remove_dir_all(&dir).unwrap();
        }
        std::fs::create_dir_all(&dir).unwrap();
        // RGBA, transparent around the emblem: (file, width, height, emblem size).
        // Names carry resource qualifiers (scale, target size): the package's
        // resources.pri (made by tools/msix/make-msix.ps1) lets Windows pick
        // the right one. The "unplated" small icons are what keep Windows
        // from drawing them on a plate in the accent colour (the taskbar and
        // the Start menu showed the emblem with blue corners).
        let mut files: Vec<(String, usize, usize, usize)> = Vec::new();
        for (scale, k) in [(100, 1.0), (200, 2.0), (400, 4.0)] {
            let px = |v: f32| (v * k) as usize;
            files.push((
                format!("Square44x44Logo.scale-{scale}.png"),
                px(44.0),
                px(44.0),
                px(44.0),
            ));
            files.push((
                format!("Square150x150Logo.scale-{scale}.png"),
                px(150.0),
                px(150.0),
                px(108.0),
            ));
            files.push((
                format!("Wide310x150Logo.scale-{scale}.png"),
                px(310.0),
                px(150.0),
                px(108.0),
            ));
            files.push((format!("StoreLogo.scale-{scale}.png"), px(50.0), px(50.0), px(50.0)));
        }
        for size in [16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 256] {
            for form in ["", "_altform-unplated", "_altform-lightunplated"] {
                files.push((
                    format!("Square44x44Logo.targetsize-{size}{form}.png"),
                    size,
                    size,
                    size,
                ));
            }
        }
        for (name, w, h, e) in files {
            let emblem = super::render(e);
            let mut px = vec![0u8; w * h * 4];
            let (ox, oy) = ((w - e) / 2, (h - e) / 2);
            for y in 0..e {
                let src = &emblem[y * e * 4..(y + 1) * e * 4];
                let at = ((oy + y) * w + ox) * 4;
                px[at..at + e * 4].copy_from_slice(src);
            }
            let mut bytes = Vec::new();
            {
                let mut enc = png::Encoder::new(&mut bytes, w as u32, h as u32);
                enc.set_color(png::ColorType::Rgba);
                enc.set_depth(png::BitDepth::Eight);
                enc.set_compression(png::Compression::Best);
                enc.write_header().unwrap().write_image_data(&px).unwrap();
            }
            std::fs::write(dir.join(name), bytes).unwrap();
        }
    }

    /// An RGB picture: `background(x, y)` with emblems `(size, x, y)` on top.
    fn picture(
        w: usize,
        h: usize,
        background: impl Fn(usize, usize) -> [f32; 3],
        emblems: &[(usize, usize, usize)],
    ) -> Vec<[f32; 3]> {
        let mut out: Vec<[f32; 3]> = (0..w * h).map(|i| background(i % w, i / w)).collect();
        for &(size, ex, ey) in emblems {
            let px = super::render(size);
            for y in 0..size {
                for x in 0..size {
                    let s = &px[(y * size + x) * 4..][..4];
                    let a = s[3] as f32 / 255.0;
                    let d = &mut out[(ey + y) * w + ex + x];
                    for k in 0..3 {
                        d[k] += (s[k] as f32 / 255.0 - d[k]) * a;
                    }
                }
            }
        }
        out
    }

    /// A 24-bit Windows bitmap (bottom-up rows, padded to 4 bytes).
    fn bmp(w: usize, h: usize, px: &[[f32; 3]]) -> Vec<u8> {
        let row = (w * 3).div_ceil(4) * 4;
        let size = 54 + row * h;
        let mut out = Vec::with_capacity(size);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&(size as u32).to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&(w as i32).to_le_bytes());
        out.extend_from_slice(&(h as i32).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&24u16.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&((row * h) as u32).to_le_bytes());
        out.extend_from_slice(&2835u32.to_le_bytes());
        out.extend_from_slice(&2835u32.to_le_bytes());
        out.extend_from_slice(&[0; 8]);
        for y in (0..h).rev() {
            let start = out.len();
            for x in 0..w {
                let c = px[y * w + x];
                for k in [2, 1, 0] {
                    out.push((c[k].clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
                }
            }
            out.resize(start + row, 0);
        }
        out
    }
}
