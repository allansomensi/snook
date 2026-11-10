//! The window icon: an 8-ball, drawn procedurally.

use macroquad::miniquad::conf::Icon;

fn render<const N: usize>(size: usize) -> [u8; N] {
    let mut out = [0u8; N];
    let ss = 4;
    let r = size as f32 / 2.0 - 0.5;
    let c = size as f32 / 2.0;
    for y in 0..size {
        for x in 0..size {
            let mut acc = [0f32; 4];
            for sy in 0..ss {
                for sx in 0..ss {
                    let px = x as f32 + (sx as f32 + 0.5) / ss as f32 - c;
                    let py = y as f32 + (sy as f32 + 0.5) / ss as f32 - c;
                    let d = (px * px + py * py).sqrt() / r;
                    if d > 1.0 {
                        continue;
                    }
                    // White number circle, slightly up and left.
                    let (wx, wy) = (px / r + 0.12, py / r + 0.12);
                    let wd = (wx * wx + wy * wy).sqrt();
                    let mut col = if wd < 0.46 { [0.96, 0.95, 0.9] } else { [0.06, 0.06, 0.07] };
                    // An "8" as two small rings.
                    if wd < 0.46 {
                        for (oy, rr) in [(-0.13f32, 0.11f32), (0.12, 0.13)] {
                            let e = (wx * wx + (wy - oy) * (wy - oy)).sqrt();
                            if (e - rr).abs() < 0.045 {
                                col = [0.05, 0.05, 0.06];
                            }
                        }
                    }
                    // Soft highlight.
                    let (hx, hy) = (px / r + 0.4, py / r + 0.45);
                    let h = (1.0 - (hx * hx + hy * hy).sqrt() / 0.5).max(0.0) * 0.5;
                    for k in 0..3 {
                        acc[k] += (col[k] + h).min(1.0);
                    }
                    acc[3] += 1.0;
                }
            }
            let n = (ss * ss) as f32;
            let i = (y * size + x) * 4;
            let a = acc[3] / n;
            for k in 0..3 {
                out[i + k] = if acc[3] > 0.0 { (acc[k] / acc[3] * 255.0) as u8 } else { 0 };
            }
            out[i + 3] = (a * 255.0) as u8;
        }
    }
    out
}

pub fn window_icon() -> Icon {
    Icon { small: render::<1024>(16), medium: render::<4096>(32), big: render::<16384>(64) }
}

#[cfg(test)]
mod tests {
    #[test]
    fn icon_is_round_and_opaque_in_the_middle() {
        let px = super::render::<4096>(32);
        assert_eq!(px[3], 0, "corner is transparent");
        let mid = (16 * 32 + 4) * 4;
        assert_eq!(px[mid + 3], 255);
    }
}
