//! The pool hall around the table (when the dark stage is off): panelled
//! walls with damask paper, a varnished plank floor and a rug, a pressed-tin
//! ceiling with beams, the bar with its bottles and stools, cue racks, a
//! scoreboard, a chalkboard, a dartboard, posters, a jukebox and a neon sign.
//!
//! Everything is built once from code (no image files). The room is lit
//! once too: the light of the lamps is baked into the vertices and the room
//! is drawn unlit, so it costs almost nothing per frame.

use macroquad::prelude::*;

use crate::config::Quality;
use crate::coords::TABLE_HEIGHT;
use crate::gfx::{self, MeshBuilder, Surface};
use crate::textures::{self, Pixels};

/// Half sizes of the room and its height.
const HX: f32 = 5.5;
const HZ: f32 = 4.0;
const HEIGHT: f32 = 3.2;
/// Top of the wood panelling, and of the paper (the crown molding above).
const WAINSCOT: f32 = 1.0;
const CROWN: f32 = 3.0;

pub struct RoomPart {
    pub mesh: gfx::Model,
    pub surface: Surface,
    /// Drawn see-through (pools of light on the walls).
    pub blended: bool,
}

struct Light {
    pos: Vec3,
    color: Vec3,
    power: f32,
    /// A shade that only lights downwards.
    down: bool,
}

/// The lights of the room (for the baked lighting): the lamp over the table,
/// the wall sconces, the pendants over the bar, the jukebox and the neon sign.
fn lights() -> Vec<Light> {
    let mut l = Vec::new();
    for x in [-0.45f32, 0.0, 0.45] {
        l.push(Light {
            pos: vec3(x, TABLE_HEIGHT + 0.93, 0.0),
            color: vec3(1.0, 0.86, 0.66),
            power: 1.0,
            down: true,
        });
    }
    for side in [-1.0f32, 1.0] {
        for z in [-1.55f32, 1.55] {
            l.push(Light {
                pos: vec3(side * (HX - 0.16), 1.95, z),
                color: vec3(1.0, 0.72, 0.42),
                power: 0.8,
                down: false,
            });
        }
    }
    for x in [-1.3f32, 0.0, 1.3] {
        l.push(Light {
            pos: vec3(x, 2.25, -3.3),
            color: vec3(1.0, 0.78, 0.5),
            power: 0.9,
            down: true,
        });
    }
    l.push(Light {
        pos: vec3(2.6, 1.2, HZ - 0.5),
        color: vec3(1.0, 0.5, 0.25),
        power: 0.25,
        down: false,
    });
    l.push(Light {
        pos: vec3(-0.6, 2.1, HZ - 0.1),
        color: vec3(0.95, 0.25, 0.55),
        power: 0.22,
        down: false,
    });
    l
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The light reaching a point of the room (with soft corners), tone mapped.
fn bake_light(lights: &[Light], table: Vec2, p: Vec3, n: Vec3) -> Vec3 {
    let hemi = 0.72 + 0.28 * n.y;
    let mut c = vec3(0.30, 0.25, 0.21) * hemi;
    let under_table = p.y < 0.75 && p.x.abs() < table.x && p.z.abs() < table.y;
    for l in lights {
        let d = l.pos - p;
        let dist = d.length().max(0.05);
        let dir = d / dist;
        // A little wrap-around keeps faces turned away from going black.
        let ndl = (n.dot(dir) * 0.85 + 0.15).max(0.0);
        let cone = if l.down { 0.2 + 0.8 * smooth(0.15, 0.75, dir.y) } else { 1.0 };
        let mut att = l.power / (1.0 + dist * dist * 0.7);
        if under_table && l.down && l.pos.y < 2.0 {
            att *= 0.12;
        }
        c += l.color * ndl * att * cone;
    }
    // Corners and the edges of the floor and ceiling get less light.
    let to_wall = (HX - p.x.abs()).min(HZ - p.z.abs());
    let ao = 0.55
        + 0.45
            * smooth(0.0, 0.45, p.y).min(smooth(0.0, 0.5, to_wall)).min(smooth(
                0.0,
                0.35,
                HEIGHT - p.y,
            ));
    let c = c * ao * 2.6;
    vec3(1.0 - (-c.x).exp(), 1.0 - (-c.y).exp(), 1.0 - (-c.z).exp())
}

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgba(r, g, b, 255)
}

fn upload(p: Pixels, repeat: bool) -> Texture2D {
    gfx::texture(p.width, p.height, &p.rgba, repeat)
}

/// A cylinder between two points.
fn tube(b: &mut MeshBuilder, a: Vec3, c: Vec3, r: f32, segments: u32, color: Color) {
    let axis = (c - a).normalize_or_zero();
    let side = if axis.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
    let u = axis.cross(side).normalize_or_zero();
    let v = axis.cross(u);
    for k in 0..segments {
        let t0 = k as f32 / segments as f32 * std::f32::consts::TAU;
        let t1 = (k + 1) as f32 / segments as f32 * std::f32::consts::TAU;
        let d0 = u * t0.cos() + v * t0.sin();
        let d1 = u * t1.cos() + v * t1.sin();
        let i = [
            b.push(a + d0 * r, Vec2::ZERO, d0, color),
            b.push(a + d1 * r, Vec2::ZERO, d1, color),
            b.push(c + d1 * r, Vec2::ZERO, d1, color),
            b.push(c + d0 * r, Vec2::ZERO, d0, color),
        ];
        b.indices.extend_from_slice(&[i[0], i[1], i[2], i[0], i[2], i[3]]);
    }
}

/// A ring of tube (a torus) lying flat.
fn hoop(b: &mut MeshBuilder, center: Vec3, radius: f32, r: f32, segments: u32, color: Color) {
    for k in 0..segments {
        let a0 = k as f32 / segments as f32 * std::f32::consts::TAU;
        let a1 = (k + 1) as f32 / segments as f32 * std::f32::consts::TAU;
        let p0 = center + vec3(a0.cos(), 0.0, a0.sin()) * radius;
        let p1 = center + vec3(a1.cos(), 0.0, a1.sin()) * radius;
        tube(b, p0, p1, r, 8, color);
    }
}

/// A disc facing `normal`, with polar texture coordinates (0..1).
fn disc_facing(
    b: &mut MeshBuilder,
    center: Vec3,
    normal: Vec3,
    radius: f32,
    segments: u32,
    color: Color,
) {
    let side = if normal.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
    let u = side.cross(normal).normalize_or_zero();
    let v = normal.cross(u);
    let c = b.push(center, vec2(0.5, 0.5), normal, color);
    for k in 0..segments {
        let a0 = k as f32 / segments as f32 * std::f32::consts::TAU;
        let a1 = (k + 1) as f32 / segments as f32 * std::f32::consts::TAU;
        let p = |a: f32| center + (u * a.cos() + v * a.sin()) * radius;
        let uv = |a: f32| vec2(0.5 + a.cos() * 0.5, 0.5 - a.sin() * 0.5);
        let i0 = b.push(p(a0), uv(a0), normal, color);
        let i1 = b.push(p(a1), uv(a1), normal, color);
        b.indices.extend_from_slice(&[c, i0, i1]);
    }
}

/// A rectangle (a picture, a board) on a wall, with 0..1 texture coordinates.
fn picture(b: &mut MeshBuilder, center: Vec3, right: Vec3, up: Vec3, w: f32, h: f32, color: Color) {
    let (r, u) = (right * w / 2.0, up * h / 2.0);
    b.quad(
        [center - r - u, center + r - u, center + r + u, center - r + u],
        [vec2(0.0, 1.0), vec2(1.0, 1.0), vec2(1.0, 0.0), vec2(0.0, 0.0)],
        color,
    );
}

/// A wall: (point on its floor line, the direction along it, its length,
/// the direction into the room).
struct Wall {
    start: Vec3,
    along: Vec3,
    length: f32,
    inward: Vec3,
}

fn walls() -> [Wall; 4] {
    [
        // Back (the bar), front, left (cue racks), right (posters).
        Wall { start: vec3(-HX, 0.0, -HZ), along: Vec3::X, length: 2.0 * HX, inward: Vec3::Z },
        Wall { start: vec3(HX, 0.0, HZ), along: -Vec3::X, length: 2.0 * HX, inward: -Vec3::Z },
        Wall { start: vec3(-HX, 0.0, HZ), along: -Vec3::Z, length: 2.0 * HZ, inward: Vec3::X },
        Wall { start: vec3(HX, 0.0, -HZ), along: Vec3::Z, length: 2.0 * HZ, inward: -Vec3::X },
    ]
}

/// Everything in the room. `table`: the table's half length and width (for
/// its shadow on the floor light).
pub fn build(quality: Quality, table: Vec2, white: &Texture2D, wood: &Texture2D) -> Vec<RoomPart> {
    let detailed = quality >= Quality::High;
    let medium = quality >= Quality::Medium;
    let big = quality == Quality::Ultra;
    // Textures, made side by side.
    let (floor_px, paper_px, tin_px, rug_px, chalk_px, dart_px, posters_px, label_px) =
        std::thread::scope(|s| {
            let floor = s.spawn(move || textures::floor_sized(if big { 512 } else { 256 }));
            let paper = s.spawn(textures::wallpaper);
            let tin = s.spawn(textures::tin_ceiling);
            let rug = s.spawn(textures::rug);
            let chalk = s.spawn(textures::chalkboard);
            let dart = s.spawn(textures::dartboard);
            let posters = s.spawn(|| [0, 1, 2].map(textures::poster));
            let label = s.spawn(textures::label);
            (
                floor.join().expect("floor"),
                paper.join().expect("paper"),
                tin.join().expect("tin"),
                rug.join().expect("rug"),
                chalk.join().expect("chalk"),
                dart.join().expect("dart"),
                posters.join().expect("posters"),
                label.join().expect("label"),
            )
        });
    let floor_tex = upload(floor_px, true);
    let paper_tex = upload(paper_px, true);
    let tin_tex = upload(tin_px, true);
    let rug_tex = upload(rug_px, false);
    let chalk_tex = upload(chalk_px, false);
    let dart_tex = upload(dart_px, false);
    let poster_tex: Vec<Texture2D> = posters_px.into_iter().map(|p| upload(p, false)).collect();
    let label_tex = upload(label_px, false);

    let lights = lights();
    let table_shadow = table + vec2(0.12, 0.12);
    let mut parts = Vec::new();
    let mut add = |mut b: MeshBuilder, tex: &Texture2D, baked: bool| {
        if b.is_empty() {
            return;
        }
        if baked {
            b.bake(|p, n| bake_light(&lights, table_shadow, p, n));
        }
        parts.push(RoomPart { mesh: b.build(tex), surface: Surface::UNLIT, blended: false });
    };
    let mut night: Option<MeshBuilder> = None;
    let cell = if detailed { 0.2 } else { 0.4 };

    // --- Floor, rug -------------------------------------------------------
    let mut floor = MeshBuilder::default();
    let (nx, nz) = ((2.0 * HX / cell) as u32, (2.0 * HZ / cell) as u32);
    floor.grid(
        vec3(-HX, 0.0, HZ),
        vec3(2.0 * HX, 0.0, 0.0),
        vec3(0.0, 0.0, -2.0 * HZ),
        nx,
        nz,
        WHITE,
        0.32,
    );
    add(floor, &floor_tex, true);
    if medium {
        let mut rug = MeshBuilder::default();
        let (rx, rz) = (2.4f32, 1.75f32);
        let first = rug.len();
        rug.grid(
            vec3(-rx, 0.004, rz),
            vec3(2.0 * rx, 0.0, 0.0),
            vec3(0.0, 0.0, -2.0 * rz),
            24,
            18,
            WHITE,
            1.0,
        );
        for v in &mut rug.vertices[first..] {
            v.uv = vec2((v.position.x + rx) / (2.0 * rx), (v.position.z + rz) / (2.0 * rz));
        }
        add(rug, &rug_tex, true);
    }

    // --- Walls: panelling, paper, moldings ---------------------------------
    let mut paper = MeshBuilder::default();
    let mut panelling = MeshBuilder::default();
    let mut trim = MeshBuilder::default();
    let wood_color = rgb(255, 236, 222);
    let trim_color = rgb(210, 180, 160);
    for w in walls() {
        let up = Vec3::Y;
        let steps = (w.length / cell).ceil() as u32;
        // Paper from the panelling to the crown.
        paper.grid(
            w.start + up * WAINSCOT,
            w.along * w.length,
            up * (CROWN - WAINSCOT),
            steps,
            ((CROWN - WAINSCOT) / cell).ceil() as u32,
            WHITE,
            1.0 / 0.62,
        );
        // Wood behind the panels.
        panelling.grid(
            w.start,
            w.along * w.length,
            up * WAINSCOT,
            steps,
            (WAINSCOT / cell).ceil() as u32,
            wood_color,
            0.9,
        );
        if medium {
            // Raised panels: a frame of stiles and rails standing out of the
            // wall, and a bevelled panel inside each frame.
            let count = (w.length / 0.95).floor().max(1.0) as u32;
            let pw = w.length / count as f32;
            for k in 0..count {
                let c0 = w.start + w.along * (k as f32 * pw) + w.inward * 0.012;
                let frame = 0.07;
                let depth = 0.018;
                let box_at = |b: &mut MeshBuilder, along: f32, y: f32, len: f32, h: f32| {
                    let center = c0 + w.along * along + Vec3::Y * y;
                    let size = w.along.abs() * len + Vec3::Y * h + w.inward.abs() * depth;
                    b.cuboid(center, size, 0.0, wood_color, 0.9);
                };
                box_at(&mut panelling, frame / 2.0, 0.55, frame, 0.82);
                box_at(&mut panelling, pw / 2.0, 0.17, pw, frame);
                box_at(&mut panelling, pw / 2.0, 0.93, pw, frame);
                if detailed {
                    // The panel itself, slightly raised, with a lighter bevel.
                    let center = c0 + w.along * (pw / 2.0) + Vec3::Y * 0.55;
                    let size =
                        w.along.abs() * (pw - 2.6 * frame) + Vec3::Y * 0.56 + w.inward.abs() * 0.01;
                    panelling.cuboid(center, size, 0.0, rgb(255, 245, 236), 0.9);
                }
            }
            // Baseboard, chair rail and crown molding along the wall.
            let a = w.start;
            let b = w.start + w.along * w.length;
            let rail = [
                vec2(0.0, 0.0),
                vec2(0.022, 0.0),
                vec2(0.03, 0.012),
                vec2(0.03, 0.03),
                vec2(0.02, 0.045),
                vec2(0.0, 0.05),
            ];
            trim.extrude(
                a + Vec3::Y * (WAINSCOT - 0.02),
                b + Vec3::Y * (WAINSCOT - 0.02),
                w.inward,
                &rail,
                trim_color,
            );
            let base = [
                vec2(0.0, 0.0),
                vec2(0.025, 0.0),
                vec2(0.025, 0.11),
                vec2(0.015, 0.14),
                vec2(0.0, 0.15),
            ];
            trim.extrude(a, b, w.inward, &base, trim_color);
            let crown: Vec<Vec2> = (0..=6)
                .map(|k| {
                    let t = k as f32 / 6.0 * std::f32::consts::FRAC_PI_2;
                    vec2(0.14 * (1.0 - t.sin()), 0.14 * (1.0 - t.cos()))
                })
                .chain([vec2(0.0, 0.2), vec2(0.0, 0.0)])
                .collect();
            trim.extrude(a + Vec3::Y * CROWN, b + Vec3::Y * CROWN, w.inward, &crown, trim_color);
            trim.extrude(
                a + Vec3::Y * (CROWN + 0.14),
                b + Vec3::Y * (CROWN + 0.14),
                w.inward,
                &[vec2(0.0, 0.0), vec2(0.16, 0.0), vec2(0.16, 0.06), vec2(0.0, 0.06)],
                trim_color,
            );
        }
    }
    add(paper, &paper_tex, true);
    add(panelling, wood, true);
    add(trim, wood, true);

    // --- Ceiling and beams -------------------------------------------------
    let mut ceiling = MeshBuilder::default();
    ceiling.grid(
        vec3(-HX, HEIGHT, -HZ),
        vec3(2.0 * HX, 0.0, 0.0),
        vec3(0.0, 0.0, 2.0 * HZ),
        nx,
        nz,
        WHITE,
        1.0 / 0.6,
    );
    add(ceiling, &tin_tex, true);
    if medium {
        let mut beams = MeshBuilder::default();
        for k in -2..=2 {
            let x = k as f32 * 2.2;
            beams.cuboid(
                vec3(x, HEIGHT - 0.11, 0.0),
                vec3(0.2, 0.22, 2.0 * HZ),
                0.0,
                rgb(230, 200, 180),
                0.8,
            );
        }
        add(beams, wood, true);
    }

    // --- The bar -------------------------------------------------------------
    let bar_z = -3.5;
    let mut bar_wood = MeshBuilder::default();
    let mut brass = MeshBuilder::default();
    let mut dark = MeshBuilder::default();
    let mut glow = MeshBuilder::default();
    let mut glass = MeshBuilder::default();
    let mut labels = MeshBuilder::default();
    let counter_len = 4.2;
    // Front panel with raised panels.
    bar_wood.cuboid(vec3(0.0, 0.52, bar_z), vec3(counter_len, 1.04, 0.56), 0.0, wood_color, 0.8);
    if medium {
        for k in 0..6 {
            let x = -counter_len / 2.0 + (k as f32 + 0.5) * counter_len / 6.0;
            bar_wood.cuboid(
                vec3(x, 0.55, bar_z + 0.29),
                vec3(counter_len / 6.0 - 0.14, 0.62, 0.02),
                0.0,
                rgb(255, 245, 236),
                0.8,
            );
        }
    }
    // The counter top with a rounded front edge.
    let top: Vec<Vec2> = (0..=6)
        .map(|k| {
            let t = k as f32 / 6.0 * std::f32::consts::PI;
            vec2(0.025 * t.sin(), 0.025 - 0.025 * t.cos())
        })
        .chain([vec2(-0.66, 0.05), vec2(-0.66, 0.0)])
        .collect();
    bar_wood.extrude(
        vec3(-counter_len / 2.0 - 0.05, 1.04, bar_z + 0.33),
        vec3(counter_len / 2.0 + 0.05, 1.04, bar_z + 0.33),
        Vec3::Z,
        &top,
        rgb(255, 210, 180),
    );
    // Brass foot rail on posts.
    tube(
        &mut brass,
        vec3(-counter_len / 2.0, 0.18, bar_z + 0.45),
        vec3(counter_len / 2.0, 0.18, bar_z + 0.45),
        0.022,
        14,
        rgb(200, 160, 80),
    );
    for k in 0..5 {
        let x = -counter_len / 2.0 + 0.2 + k as f32 * (counter_len - 0.4) / 4.0;
        tube(
            &mut brass,
            vec3(x, 0.18, bar_z + 0.45),
            vec3(x, 0.18, bar_z + 0.3),
            0.014,
            8,
            rgb(200, 160, 80),
        );
    }
    // Back bar: a mirror, shelves and bottles.
    dark.cuboid(vec3(0.0, 1.85, -HZ + 0.03), vec3(3.8, 0.9, 0.02), 0.0, rgb(40, 46, 52), 1.0);
    bar_wood.cuboid(vec3(0.0, 0.5, -HZ + 0.25), vec3(4.0, 1.0, 0.45), 0.0, wood_color, 0.8);
    for shelf in 0..2 {
        bar_wood.cuboid(
            vec3(0.0, 1.55 + shelf as f32 * 0.45, -HZ + 0.14),
            vec3(3.8, 0.03, 0.24),
            0.0,
            trim_color,
            0.8,
        );
    }
    let bottle_colors = [
        rgb(40, 110, 40),
        rgb(150, 85, 25),
        rgb(210, 200, 170),
        rgb(120, 25, 25),
        rgb(30, 60, 130),
        rgb(90, 60, 30),
    ];
    let segs = if detailed { 16 } else { 10 };
    for shelf in 0..2 {
        for k in 0..14 {
            let h = 0.20 + (k % 3) as f32 * 0.04;
            let y = 1.567 + shelf as f32 * 0.45;
            let at = vec3(-1.6 + k as f32 * 0.245, y, -HZ + 0.16);
            let color = bottle_colors[(k + shelf * 2) % bottle_colors.len()];
            let r = 0.034 - (k % 2) as f32 * 0.004;
            // Body, shoulder, neck, lip.
            let profile = [
                (0.0, r * 0.92),
                (0.005, r),
                (h * 0.62, r),
                (h * 0.78, r * 0.55),
                (h * 0.84, r * 0.36),
                (h + 0.06, r * 0.34),
                (h + 0.065, r * 0.42),
                (h + 0.075, r * 0.38),
            ];
            glass.lathe(at, &profile, segs, color);
            if detailed {
                // A paper label around the body.
                let first = labels.len();
                labels.lathe(at, &[(h * 0.18, r + 0.0012), (h * 0.5, r + 0.0012)], segs, WHITE);
                for v in &mut labels.vertices[first..] {
                    v.uv.y = (v.position.y - at.y - h * 0.18) / (h * 0.32);
                }
            }
        }
    }
    // Pendant lamps over the bar.
    for x in [-1.3f32, 0.0, 1.3] {
        let at = vec3(x, 2.25, -3.3);
        tube(&mut dark, at + Vec3::Y * 0.12, vec3(x, HEIGHT, -3.3), 0.006, 6, rgb(30, 30, 30));
        let shade = [(0.0, 0.17), (0.03, 0.16), (0.08, 0.11), (0.12, 0.05), (0.13, 0.02)];
        brass.lathe(at, &shade, 24, rgb(40, 70, 50));
        glow.disc(at + Vec3::Y * 0.002, 0.15, 24, rgb(255, 222, 170));
    }
    // Stools along the bar.
    for k in 0..5 {
        let c = vec3(-1.6 + k as f32 * 0.8, 0.0, -2.75);
        dark.lathe(
            c,
            &[(0.0, 0.2), (0.02, 0.2), (0.035, 0.05), (0.66, 0.035), (0.68, 0.12)],
            20,
            rgb(50, 46, 42),
        );
        hoop(&mut brass, c + Vec3::Y * 0.3, 0.17, 0.011, 18, rgb(190, 150, 75));
        for a in 0..3 {
            let t = a as f32 / 3.0 * std::f32::consts::TAU;
            tube(
                &mut brass,
                c + Vec3::Y * 0.3,
                c + vec3(t.cos() * 0.17, 0.3, t.sin() * 0.17),
                0.008,
                6,
                rgb(190, 150, 75),
            );
        }
        // A tufted leather seat.
        let seat = [
            (0.68, 0.0),
            (0.68, 0.19),
            (0.70, 0.205),
            (0.74, 0.205),
            (0.765, 0.18),
            (0.775, 0.1),
            (0.778, 0.0),
        ];
        dark.lathe(c, &seat, 24, rgb(150, 38, 32));
    }

    // --- Sconces with their pools of light --------------------------------
    let mut pools = MeshBuilder::default();
    for side in [-1.0f32, 1.0] {
        for z in [-1.55f32, 1.55] {
            let wall_x = side * HX;
            let at = vec3(wall_x - side * 0.04, 1.85, z);
            brass.cuboid(at, vec3(0.03, 0.18, 0.1), 0.0, rgb(180, 140, 70), 1.0);
            tube(&mut brass, at, at + vec3(-side * 0.12, 0.05, 0.0), 0.008, 8, rgb(180, 140, 70));
            let shade = [(0.0, 0.02), (0.02, 0.05), (0.09, 0.07), (0.12, 0.072)];
            glow.lathe(at + vec3(-side * 0.13, 0.04, 0.0), &shade, 18, rgb(255, 214, 160));
            let c = vec3(wall_x - side * 0.006, 1.95, z);
            let (h, w) = (1.1, 0.9);
            let p = [
                c + vec3(0.0, -h, -w),
                c + vec3(0.0, -h, w),
                c + vec3(0.0, h, w),
                c + vec3(0.0, h, -w),
            ];
            pools.quad(
                p,
                [vec2(0.0, 1.0), vec2(1.0, 1.0), vec2(1.0, 0.0), vec2(0.0, 0.0)],
                Color::new(1.0, 0.72, 0.42, 0.16),
            );
        }
    }

    // --- Left wall: cue racks, the scoreboard, the chalkboard --------------
    let lx = -HX + 0.06;
    {
        let (z0, z1) = (-0.95f32, 0.95f32);
        bar_wood.cuboid(
            vec3(lx, 0.11, (z0 + z1) / 2.0),
            vec3(0.12, 0.07, z1 - z0),
            0.0,
            trim_color,
            0.8,
        );
        bar_wood.cuboid(
            vec3(lx, 1.32, (z0 + z1) / 2.0),
            vec3(0.1, 0.06, z1 - z0),
            0.0,
            trim_color,
            0.8,
        );
        bar_wood.cuboid(
            vec3(-HX + 0.015, 0.75, (z0 + z1) / 2.0),
            vec3(0.03, 1.4, z1 - z0 + 0.1),
            0.0,
            wood_color,
            0.8,
        );
    }
    let butts =
        [rgb(35, 20, 12), rgb(90, 30, 25), rgb(20, 20, 24), rgb(60, 35, 18), rgb(25, 45, 70)];
    for k in 0..9 {
        let z = -0.84 + k as f32 * 0.21;
        let at = vec3(lx, 0.14, z);
        let butt = butts[k % butts.len()];
        let cue = [
            (0.0, 0.0148),
            (0.12, 0.0145),
            (0.121, 0.0146),
            (0.37, 0.0135),
            (0.67, 0.0118),
            (1.43, 0.0064),
            (1.456, 0.006),
        ];
        dark.lathe(at, &cue, 10, butt);
        // The maple shaft.
        dark.lathe(at + Vec3::Y * 0.672, &[(0.0, 0.0117), (0.76, 0.0065)], 10, rgb(222, 190, 135));
        dark.lathe(at + Vec3::Y * 1.432, &[(0.0, 0.0064), (0.02, 0.0063)], 10, rgb(240, 238, 228));
    }
    // Bead scoreboard above the rack.
    {
        let x = -HX + 0.05;
        bar_wood.cuboid(vec3(x, 2.25, 0.0), vec3(0.05, 0.42, 1.7), 0.0, trim_color, 0.8);
        for (k, y) in [2.16f32, 2.34].into_iter().enumerate() {
            tube(
                &mut brass,
                vec3(x + 0.035, y, -0.75),
                vec3(x + 0.035, y, 0.75),
                0.004,
                6,
                rgb(170, 160, 140),
            );
            for b in 0..20 {
                let pushed = if k == 0 { 6 } else { 4 };
                let z =
                    if b < pushed { -0.72 + b as f32 * 0.062 } else { -0.20 + b as f32 * 0.046 };
                let color = if (b / 5) % 2 == 0 { rgb(235, 225, 200) } else { rgb(40, 28, 20) };
                dark.lathe(
                    vec3(x + 0.04, y - 0.022, z),
                    &[(0.0, 0.004), (0.01, 0.02), (0.034, 0.02), (0.044, 0.004)],
                    10,
                    color,
                );
            }
        }
    }
    let mut boards = MeshBuilder::default();
    let mut darts = MeshBuilder::default();
    let mut poster_meshes: Vec<MeshBuilder> = (0..3).map(|_| MeshBuilder::default()).collect();
    if medium {
        // A chalkboard with the night's scores.
        let c = vec3(-HX + 0.035, 1.75, -2.75);
        bar_wood.cuboid(c - Vec3::X * 0.01, vec3(0.03, 0.82, 1.12), 0.0, trim_color, 0.8);
        picture(&mut boards, c + Vec3::X * 0.01, -Vec3::Z, Vec3::Y, 1.0, 0.7, WHITE);
        bar_wood.cuboid(c + vec3(0.04, -0.37, 0.0), vec3(0.06, 0.02, 1.0), 0.0, trim_color, 0.8);
    }

    // --- Right wall: posters and the dartboard -----------------------------
    let rx = HX - 0.04;
    for (k, z) in [-0.75f32, 0.0, 0.75].into_iter().enumerate() {
        let c = vec3(rx, 2.1, z);
        bar_wood.cuboid(c, vec3(0.04, 0.66, 0.5), 0.0, rgb(60, 40, 28), 0.6);
        picture(&mut poster_meshes[k], c - Vec3::X * 0.022, Vec3::Z, Vec3::Y, 0.42, 0.58, WHITE);
    }
    if medium {
        // The dartboard in its cabinet, doors open.
        let c = vec3(rx - 0.02, 1.73, -2.75);
        bar_wood.cuboid(c + Vec3::X * 0.02, vec3(0.08, 0.7, 0.62), 0.0, trim_color, 0.8);
        for side in [-1.0f32, 1.0] {
            bar_wood.cuboid(
                c + vec3(-0.15, 0.0, side * 0.46),
                vec3(0.3, 0.7, 0.025),
                side * 0.5,
                trim_color,
                0.8,
            );
        }
        disc_facing(&mut darts, c - Vec3::X * 0.03, -Vec3::X, 0.225, 48, WHITE);
        // Three darts in the board.
        for (dy, dz) in [(0.03f32, -0.02f32), (-0.05, 0.04), (0.08, 0.06)] {
            let p = c + vec3(-0.03, dy, dz);
            tube(&mut dark, p, p + vec3(-0.12, 0.01, 0.0), 0.004, 6, rgb(40, 40, 44));
            tube(
                &mut glow,
                p + vec3(-0.1, 0.008, 0.0),
                p + vec3(-0.15, 0.012, 0.0),
                0.012,
                4,
                rgb(220, 40, 40),
            );
        }
    }

    // --- Front wall: a door, a window, the jukebox, a neon sign ------------
    if detailed {
        let z = HZ - 0.03;
        // Door with its frame.
        bar_wood.cuboid(vec3(-3.4, 1.05, z), vec3(1.06, 2.1, 0.05), 0.0, trim_color, 0.8);
        bar_wood.cuboid(
            vec3(-3.4, 1.03, z - 0.03),
            vec3(0.9, 2.04, 0.04),
            0.0,
            rgb(110, 62, 40),
            0.8,
        );
        for (y, h) in [(1.5f32, 0.7f32), (0.55, 0.7)] {
            bar_wood.cuboid(
                vec3(-3.4, y, z - 0.055),
                vec3(0.66, h, 0.015),
                0.0,
                rgb(130, 76, 50),
                0.8,
            );
        }
        hoop(&mut brass, vec3(-3.05, 1.0, z - 0.09), 0.02, 0.008, 10, rgb(200, 160, 80));
        // A window at night with half-closed blinds.
        let wc = vec3(3.6, 1.75, z - 0.01);
        bar_wood.cuboid(wc, vec3(1.3, 1.1, 0.06), 0.0, trim_color, 0.8);
        let mut sky = MeshBuilder::default();
        sky.quad(
            [
                wc + vec3(0.58, -0.48, -0.035),
                wc + vec3(-0.58, -0.48, -0.035),
                wc + vec3(-0.58, 0.48, -0.035),
                wc + vec3(0.58, 0.48, -0.035),
            ],
            [Vec2::ZERO; 4],
            rgb(10, 16, 32),
        );
        for k in 0..14 {
            let y = wc.y + 0.45 - k as f32 * 0.066;
            dark.cuboid(
                vec3(wc.x, y, z - 0.06),
                vec3(1.15, 0.045, 0.006),
                0.0,
                rgb(220, 208, 180),
                1.0,
            );
        }
        night = Some(sky);
        // The jukebox: an arched cabinet with a glowing front.
        let jc = vec3(2.0, 0.0, HZ - 0.35);
        bar_wood.cuboid(jc + Vec3::Y * 0.55, vec3(0.8, 1.1, 0.5), 0.0, rgb(120, 60, 34), 0.8);
        let arch: Vec<Vec2> = (0..=10)
            .map(|k| {
                let t = k as f32 / 10.0 * std::f32::consts::PI;
                vec2(0.4 * t.cos(), 0.32 * t.sin())
            })
            .collect();
        bar_wood.extrude(
            jc + vec3(0.0, 1.1, 0.25),
            jc + vec3(0.0, 1.1, -0.25),
            Vec3::X,
            &arch,
            rgb(120, 60, 34),
        );
        let front =
            [(0.6f32, rgb(255, 170, 60)), (0.9, rgb(255, 90, 60)), (1.2, rgb(255, 200, 90))];
        for (k, (y, color)) in front.into_iter().enumerate() {
            glow.cuboid(
                jc + vec3(0.0, y, -0.256),
                vec3(0.62 - k as f32 * 0.1, 0.06, 0.01),
                0.0,
                color,
                1.0,
            );
        }
        dark.cuboid(jc + vec3(0.0, 0.3, -0.256), vec3(0.5, 0.3, 0.01), 0.0, rgb(40, 30, 28), 1.0);
        // A neon eight-ball sign.
        let nc = vec3(-0.6, 2.15, HZ - 0.06);
        let neon = rgb(255, 80, 170);
        let mut ring = |center: Vec3, radius: f32| {
            for k in 0..28 {
                let a0 = k as f32 / 28.0 * std::f32::consts::TAU;
                let a1 = (k + 1) as f32 / 28.0 * std::f32::consts::TAU;
                let p = |a: f32| center + vec3(a.cos() * radius, a.sin() * radius, 0.0);
                tube(&mut glow, p(a0), p(a1), 0.009, 6, neon);
            }
        };
        ring(nc, 0.3);
        ring(nc + vec3(0.0, 0.07, 0.0), 0.06);
        ring(nc + vec3(0.0, -0.06, 0.0), 0.075);
        dark.cuboid(nc + Vec3::Z * 0.03, vec3(0.7, 0.7, 0.01), 0.0, rgb(20, 18, 20), 1.0);
        // A high table in the corner, with two glasses.
        let tc = vec3(4.3, 0.0, -2.9);
        dark.lathe(
            tc,
            &[(0.0, 0.25), (0.03, 0.25), (0.05, 0.04), (1.0, 0.035), (1.02, 0.08)],
            20,
            rgb(40, 36, 34),
        );
        bar_wood.lathe(
            tc + Vec3::Y * 1.02,
            &[(0.0, 0.0), (0.0, 0.34), (0.035, 0.35), (0.04, 0.0)],
            32,
            rgb(255, 210, 180),
        );
        for (dx, dz, color) in
            [(0.1f32, 0.05f32, rgb(200, 150, 50)), (-0.12, -0.08, rgb(110, 20, 20))]
        {
            glass.lathe(
                tc + vec3(dx, 1.06, dz),
                &[(0.0, 0.0), (0.0, 0.03), (0.11, 0.036), (0.12, 0.037)],
                14,
                color,
            );
        }
    }
    add(bar_wood, wood, true);
    add(brass, white, true);
    add(dark, white, true);
    add(glass, white, true);
    add(labels, &label_tex, true);
    add(boards, &chalk_tex, true);
    add(darts, &dart_tex, true);
    for (k, m) in poster_meshes.into_iter().enumerate() {
        add(m, &poster_tex[k], true);
    }
    // Lights (and the night outside) glow by themselves.
    add(glow, white, false);
    if let Some(sky) = night {
        add(sky, white, false);
    }
    parts.push(RoomPart {
        mesh: pools.build(&upload(textures::glow(), false)),
        surface: Surface::UNLIT,
        blended: true,
    });
    parts
}
