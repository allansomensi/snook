//! The bar room, the table, balls, cue and aiming guides.

use std::collections::HashMap;

use ::glam::{DVec2, DVec3};
use macroquad::prelude::*;
use snookered_core::{Ball, BallId, BallStatus, PhysicsParams, PocketId, Table};

use crate::config::Quality;
use crate::coords::{TABLE_HEIGHT, ball_rotation, dir_to_world, surface, to_world};
use crate::gfx::{self, LAMP_COUNT, Materials, MeshBuilder, SceneCamera, Surface};
use crate::textures;

/// Height of the cushion top above the cloth.
const CUSHION_TOP: f32 = 0.043;
/// How far the cushion extends behind its nose.
const CUSHION_DEPTH: f32 = 0.05;
/// Height of the wooden rail top above the cloth.
const RAIL_TOP: f32 = 0.046;
/// Colour of the brass pocket castings (and the jaws cut through the rail).
const CAST_COLOR: Color = Color::new(0.47, 0.35, 0.17, 1.0);

/// Thickness of the metal pocket castings on the rail.
const CAST_H: f32 = 0.006;
const LAMP_Y: f32 = TABLE_HEIGHT + 0.95;
const SINK_TIME: f32 = 0.42;

struct Part {
    mesh: gfx::Model,
    surface: Surface,
}

pub struct Scene {
    pub table: Table,
    pub materials: Materials,
    parts: Vec<Part>,
    lamp_glow: Part,
    lamp_parts: Vec<Part>,
    /// The inside of the pockets.
    pocket_inside: gfx::Model,
    holes: [Vec4; 6],
    throats: [Vec4; 12],
    /// Drawn after the pockets: leather rims and diamond sights.
    rail_details: Vec<Part>,
    floor_shadow: Part,
    /// The pool hall around the table (none on the dark stage).
    room: Vec<crate::room::RoomPart>,
    balls: Vec<gfx::Model>,
    cue: gfx::Model,
    ghost: gfx::Model,
    white: Texture2D,
    shadow: Texture2D,
    lamps: [Vec3; LAMP_COUNT],
    sinking: HashMap<BallId, (f32, Vec3)>,
    /// Cue meshes by (cue, chalk) design.
    cues: HashMap<(String, String), gfx::Model>,
    pub lamp_visible: bool,
    /// The cloth and balls on the table now, and those being made.
    looks: Looks,
    wanted: Looks,
    /// Parts drawn with the cloth texture.
    felt_parts: Vec<usize>,
    ball_tex_size: u16,
    felt_size: u16,
    /// New textures being made on another thread.
    making: Option<std::sync::mpsc::Receiver<Made>>,
    /// Made textures waiting to go to the GPU, a few per frame.
    uploading: Option<(Made, Vec<Texture2D>)>,
    lamp_count: u32,
    /// Dark stage around the table (see `Scene::new`).
    focus: f32,
    /// Per-pixel soft shadows and contact occlusion from the balls (High, Ultra).
    soft_shadows: bool,
}

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgba(r, g, b, 255)
}

fn upload(p: textures::Pixels, repeat: bool) -> Texture2D {
    gfx::texture(p.width, p.height, &p.rgba, repeat)
}

impl Scene {
    /// `focused`: a dark stage around the table instead of the whole bar.
    pub fn new(table: Table, quality: Quality, focused: bool, looks: &Looks) -> Self {
        let detailed_room = quality != Quality::Low;
        let (ball_tex, ball_segments) = match quality {
            Quality::Low => (512, 24),
            Quality::Medium => (512, 32),
            Quality::High => (512, 48),
            Quality::Ultra => (1024, 64),
        };
        let white = gfx::white_texture();
        let felt_size = felt_size(quality);
        let felt = upload(textures::felt_sized(looks.cloth, felt_size), true);
        // Dark mahogany rails and body.
        let rail_wood =
            upload(textures::fine_wood([0.17, 0.075, 0.045], [0.05, 0.022, 0.014], 11), true);
        let body_wood =
            upload(textures::fine_wood([0.13, 0.06, 0.035], [0.04, 0.018, 0.012], 17), true);
        let shadow = upload(textures::shadow(), false);

        let mut parts = Vec::new();
        let mut add = |b: MeshBuilder, tex: &Texture2D, surface: Surface| -> usize {
            parts.push(Part { mesh: b.build(tex), surface });
            parts.len() - 1
        };

        // --- Room (not on the dark stage) ------------------------------------
        let half_table = vec2(
            table.spec.length as f32 / 2.0 + table.spec.rail_width as f32 + 0.05,
            table.spec.width as f32 / 2.0 + table.spec.rail_width as f32 + 0.05,
        );
        let room = if focused {
            Vec::new()
        } else {
            crate::room::build(quality, half_table, &white, &body_wood)
        };
        // Billiard lamp: three green glass shades on a brass bar, glowing inside.
        let mut glass = MeshBuilder::default();
        let mut brass = MeshBuilder::default();
        let mut glow = MeshBuilder::default();
        let shade_seg = if detailed_room { 32 } else { 18 };
        for x in [-0.45f32, 0.0, 0.45] {
            let at = vec3(x, LAMP_Y, 0.0);
            // Bell: wide at the bottom, curving in to a small neck.
            let profile =
                [(0.0, 0.165), (0.05, 0.150), (0.11, 0.115), (0.16, 0.070), (0.19, 0.040)];
            for w in profile.windows(2) {
                glass.frustum_at(at, w[0].0, w[0].1, w[1].0, w[1].1, shade_seg, rgb(44, 140, 82));
            }
            // Inside of the shade: bright near the bulb, warm towards the rim.
            glow.frustum_at(at, 0.001, 0.161, 0.06, 0.143, shade_seg, rgb(255, 214, 150));
            glow.frustum_at(at, 0.06, 0.143, 0.15, 0.073, shade_seg, rgb(255, 236, 196));
            glow.disc(at + Vec3::Y * 0.15, 0.073, shade_seg, rgb(255, 246, 225));
            glow.disc(at + Vec3::Y * 0.07, 0.042, 20, rgb(255, 255, 245));
            // Brass rim and neck.
            brass.frustum_at(at, -0.006, 0.168, 0.006, 0.168, shade_seg, rgb(176, 136, 62));
            brass.frustum_at(at, 0.19, 0.040, 0.24, 0.022, 16, rgb(176, 136, 62));
        }
        // The bar that holds the shades and the rods to the ceiling.
        brass.cuboid(
            vec3(0.0, LAMP_Y + 0.25, 0.0),
            vec3(1.24, 0.028, 0.04),
            0.0,
            rgb(150, 112, 50),
            1.0,
        );
        for x in [-0.55f32, 0.55] {
            brass.cuboid(
                vec3(x, LAMP_Y + 0.25 + 0.75, 0.0),
                vec3(0.008, 1.5, 0.008),
                0.0,
                rgb(60, 50, 40),
                1.0,
            );
        }
        let mut lamp_parts = vec![
            Part { mesh: glass.build(&white), surface: Surface::lit(0.9, 90.0, 0.35, 0.8) },
            Part { mesh: brass.build(&white), surface: Surface::lit(1.0, 50.0, 0.1, 0.6) },
        ];
        let lamp_glow = Part { mesh: glow.build(&white), surface: Surface::UNLIT };
        if focused {
            // No ceiling and no lamp fixture on the dark stage: only its light.
            lamp_parts.clear();
        }

        // --- Table ----------------------------------------------------------
        let spec = table.spec;
        let (l, w) = (spec.length as f32, spec.width as f32);
        // Wide, solid rails (the physics only needs `rail_width`).
        let rail_outer = spec.rail_width as f32 + 0.05;

        // Cloth bed.
        let mut bed = MeshBuilder::default();
        let m = CUSHION_DEPTH + 0.01;
        let y = TABLE_HEIGHT;
        bed.quad_world_uv(
            [
                vec3(-l / 2.0 - m, y, w / 2.0 + m),
                vec3(l / 2.0 + m, y, w / 2.0 + m),
                vec3(l / 2.0 + m, y, -w / 2.0 - m),
                vec3(-l / 2.0 - m, y, -w / 2.0 - m),
            ],
            WHITE,
            0.6,
        );
        let mut felt_parts = vec![add(bed, &felt, Surface::MATTE.cut(1.0))];

        // Cushions: a sloped rubber profile extruded along every cushion segment.
        let profile = [
            vec2(0.010, 0.0),
            vec2(0.0, spec.cushion_height as f32),
            vec2(0.008, CUSHION_TOP),
            vec2(CUSHION_DEPTH, CUSHION_TOP),
            vec2(CUSHION_DEPTH, 0.0),
        ];
        let mut cushions = MeshBuilder::default();
        // Jaw rubbers are drawn after the pocket holes are opened, so their
        // ends stay whole where they meet the hole.
        let mut jaws = MeshBuilder::default();
        let mut jaw_rails = MeshBuilder::default();
        for seg in &table.cushions {
            let mid = (seg.a + seg.b) / 2.0;
            let dir = (seg.b - seg.a).normalize();
            let mut out = dir.perp();
            let axis_aligned = dir.x.abs() < 1e-6 || dir.y.abs() < 1e-6;
            if axis_aligned {
                // A rail: the rubber goes away from the middle of the table.
                if out.dot(mid) < 0.0 {
                    out = -out;
                }
            } else {
                // A pocket jaw: the rubber goes away from the pocket's channel.
                let mouth = table
                    .pockets
                    .iter()
                    .map(|p| p.mouth_center)
                    .min_by(|a, b| a.distance(mid).total_cmp(&b.distance(mid)))
                    .unwrap_or(DVec2::ZERO);
                if out.dot(mouth - mid) > 0.0 {
                    out = -out;
                }
            }
            let out3 = vec3(out.x as f32, 0.0, -out.y as f32);
            if !axis_aligned {
                let end = clip_to_hole(&table, seg.a, seg.b);
                // The facing looks across the throat, towards the pocket's
                // axis (from the middle of its mouth to the hole): what side
                // the mouth's centre is on is not reliable for side pockets.
                let pocket = table
                    .pockets
                    .iter()
                    .min_by(|x, y| {
                        x.mouth_center.distance(mid).total_cmp(&y.mouth_center.distance(mid))
                    })
                    .expect("a jaw belongs to a pocket");
                let axis =
                    (pocket_center_of(&table, pocket.id) - pocket.mouth_center).normalize_or_zero();
                let rel = mid - pocket.mouth_center;
                let toward_axis = pocket.mouth_center + axis * rel.dot(axis) - mid;
                let across =
                    if dir.perp().dot(toward_axis) >= 0.0 { dir.perp() } else { -dir.perp() };
                jaw_wall(&mut jaws, &mut jaw_rails, &table, seg.a, end, across);
                continue;
            }
            // Rail cushions run on into the pocket corners (the throat and hole
            // are cut out in the shader); a jaw stops where it meets the hole.
            let (start, end) = if axis_aligned {
                // Each end runs on towards the nearest pocket hole, but stops
                // where the cushion line passes closest to the hole's centre:
                // further on it would come out of the cut, past the pocket.
                let run_on = |p: DVec2, out: DVec2| -> f64 {
                    let nearest = table.pockets.iter().min_by(|x, y| {
                        x.mouth_center.distance(p).total_cmp(&y.mouth_center.distance(p))
                    });
                    match nearest {
                        Some(pk) => (pocket_center_of(&table, pk.id) - p).dot(out).clamp(0.0, 0.13),
                        None => 0.0,
                    }
                };
                (seg.a - dir * run_on(seg.a, -dir), seg.b + dir * run_on(seg.b, dir))
            } else {
                unreachable!("jaws handled above")
            };
            cushions.extrude(
                surface(start, 0.0),
                surface(end, 0.0),
                out3,
                &profile,
                rgb(205, 215, 205),
            );
        }
        felt_parts.push(add(cushions, &felt, Surface::MATTE.cut(2.0)));
        felt_parts.push(add(jaws, &felt, Surface::MATTE));
        add(jaw_rails, &white, Surface::lit(0.6, 50.0, 0.1, 0.3));

        // Table body: one solid piece with rounded outer corners: the rail
        // ring on top (between the cushions and the outer edge) and the apron
        // below it, plus metal castings on the rail around every pocket.
        let depth = rail_outer - CUSHION_DEPTH;
        let off = CUSHION_DEPTH + depth / 2.0;
        let (bi, bo) = (CUSHION_DEPTH, rail_outer);
        let apron = 0.2;
        let bottom = y - apron;
        let outer = rounded_loop(l / 2.0 + bo, w / 2.0 + bo, 0.075);
        let inner = rounded_loop(l / 2.0 + bi, w / 2.0 + bi, 0.0);
        let mut body = MeshBuilder::default();
        let mut cast = MeshBuilder::default();
        // Polished antique brass, like the lamp (cream plates read as paper).
        let cast_color = CAST_COLOR;
        let n = outer.len();
        // Arc length along the loop (for wood grain that follows the rail).
        let mut along = vec![0.0f32; n + 1];
        for k in 0..n {
            along[k + 1] = along[k] + outer[k].distance(outer[(k + 1) % n]);
        }
        let in_casting = |p: Vec2| {
            let corner = p.x.abs() > l / 2.0 - 0.10 && p.y.abs() > w / 2.0 - 0.10;
            let side = p.x.abs() < 0.11 && p.y.abs() > w / 2.0;
            corner || side
        };
        let top = y + RAIL_TOP;
        let ctop = top + CAST_H;
        let wood = WHITE;
        for k in 0..n {
            let k1 = (k + 1) % n;
            let (o0, o1, i0, i1) = (outer[k], outer[k1], inner[k], inner[k1]);
            let (u0, u1) = (along[k] * 0.7, along[k + 1] * 0.7);
            let v3 = |p: Vec2, h: f32| vec3(p.x, h, p.y);
            let normal_out = {
                let t = o1 - o0;
                vec3(t.y, 0.0, -t.x).normalize_or_zero()
            };
            let normal_out =
                if normal_out.dot(v3(o0, 0.0)) < 0.0 { -normal_out } else { normal_out };
            let quad = |mb: &mut MeshBuilder, p: [Vec3; 4], uv: [Vec2; 4], nrm: Vec3, c: Color| {
                let idx: Vec<u16> = (0..4).map(|q| mb.push(p[q], uv[q], nrm, c)).collect();
                mb.indices.extend_from_slice(&[idx[0], idx[1], idx[2], idx[0], idx[2], idx[3]]);
            };
            let across = (bo - bi) * 1.2;
            // The outer edge of the rail is rounded (a quarter circle of
            // radius `round`), and the apron ends in a small lip.
            let round = if quality == Quality::Low { 0.0 } else { 0.014 };
            let out2 = |p: Vec2, d: f32| p + vec2(normal_out.x, normal_out.z) * d;
            let (oi0, oi1) = (out2(o0, -round), out2(o1, -round));
            // Rail top.
            quad(
                &mut body,
                [v3(i0, top), v3(i1, top), v3(oi1, top), v3(oi0, top)],
                [vec2(u0, 0.0), vec2(u1, 0.0), vec2(u1, across), vec2(u0, across)],
                Vec3::Y,
                wood,
            );
            if round > 0.0 {
                let steps = if quality == Quality::Ultra { 5 } else { 3 };
                for s in 0..steps {
                    let a0 = s as f32 / steps as f32 * std::f32::consts::FRAC_PI_2;
                    let a1 = (s + 1) as f32 / steps as f32 * std::f32::consts::FRAC_PI_2;
                    // From the top (a = 0) down to the side (a = 90°).
                    let at = |p: Vec2, a: f32| {
                        let q = out2(p, -round + round * a.sin());
                        vec3(q.x, top - round + round * a.cos(), q.y)
                    };
                    let n = |a: f32| (Vec3::Y * a.cos() + normal_out * a.sin()).normalize_or_zero();
                    let v = |a: f32| across + a * round;
                    let idx = [
                        body.push(at(o0, a0), vec2(u0, v(a0)), n(a0), wood),
                        body.push(at(o1, a0), vec2(u1, v(a0)), n(a0), wood),
                        body.push(at(o1, a1), vec2(u1, v(a1)), n(a1), wood),
                        body.push(at(o0, a1), vec2(u0, v(a1)), n(a1), wood),
                    ];
                    body.indices
                        .extend_from_slice(&[idx[0], idx[1], idx[2], idx[0], idx[2], idx[3]]);
                }
            }
            // Outer side, rail and apron in one.
            let lip = if quality == Quality::Low { 0.0 } else { 0.035 };
            quad(
                &mut body,
                [
                    v3(o0, bottom + lip),
                    v3(o1, bottom + lip),
                    v3(o1, top - round),
                    v3(o0, top - round),
                ],
                [vec2(u0, 0.0), vec2(u1, 0.0), vec2(u1, apron * 1.2), vec2(u0, apron * 1.2)],
                normal_out,
                wood,
            );
            if lip > 0.0 {
                // A rounded lip standing a little proud at the bottom of the apron.
                let (l0, l1) = (out2(o0, 0.012), out2(o1, 0.012));
                quad(
                    &mut body,
                    [
                        v3(o0, bottom + lip),
                        v3(o1, bottom + lip),
                        v3(l1, bottom + lip - 0.008),
                        v3(l0, bottom + lip - 0.008),
                    ],
                    [vec2(u0, 0.0), vec2(u1, 0.0), vec2(u1, 0.02), vec2(u0, 0.02)],
                    (Vec3::Y + normal_out).normalize_or_zero(),
                    wood,
                );
                quad(
                    &mut body,
                    [
                        v3(l0, bottom),
                        v3(l1, bottom),
                        v3(l1, bottom + lip - 0.008),
                        v3(l0, bottom + lip - 0.008),
                    ],
                    [vec2(u0, 0.0), vec2(u1, 0.0), vec2(u1, 0.04), vec2(u0, 0.04)],
                    normal_out,
                    wood,
                );
                quad(
                    &mut body,
                    [v3(o0, bottom), v3(o1, bottom), v3(l1, bottom), v3(l0, bottom)],
                    [Vec2::ZERO; 4],
                    -Vec3::Y,
                    wood,
                );
            }
            // Inner side of the rail (behind the cushions; open at the pockets).
            quad(
                &mut body,
                [v3(i0, y), v3(i1, y), v3(i1, top), v3(i0, top)],
                [vec2(u0, 0.0), vec2(u1, 0.0), vec2(u1, 0.05), vec2(u0, 0.05)],
                -normal_out,
                wood,
            );
            // Castings.
            let mid = (o0 + o1) / 2.0;
            if in_casting(mid) {
                let z = [Vec2::ZERO; 4];
                quad(
                    &mut cast,
                    [v3(i0, ctop), v3(i1, ctop), v3(o1, ctop), v3(o0, ctop)],
                    z,
                    Vec3::Y,
                    cast_color,
                );
                quad(
                    &mut cast,
                    [v3(o0, top), v3(o1, top), v3(o1, ctop), v3(o0, ctop)],
                    z,
                    normal_out,
                    cast_color,
                );
                quad(
                    &mut cast,
                    [v3(i0, top), v3(i1, top), v3(i1, ctop), v3(i0, ctop)],
                    z,
                    -normal_out,
                    cast_color,
                );
                let prev = (outer[(k + n - 1) % n] + o0) / 2.0;
                let next = (o1 + outer[(k + 2) % n]) / 2.0;
                let tangent = (o1 - o0).normalize_or_zero();
                let tn = vec3(tangent.x, 0.0, tangent.y);
                if !in_casting(prev) {
                    quad(
                        &mut cast,
                        [v3(i0, top), v3(o0, top), v3(o0, ctop), v3(i0, ctop)],
                        z,
                        -tn,
                        cast_color,
                    );
                }
                if !in_casting(next) {
                    quad(
                        &mut cast,
                        [v3(i1, top), v3(o1, top), v3(o1, ctop), v3(i1, ctop)],
                        z,
                        tn,
                        cast_color,
                    );
                }
            }
        }
        add(body, &rail_wood, Surface::WOOD.cut(2.0));
        add(cast, &white, Surface::lit(1.6, 90.0, 0.12, 0.7).cut(2.0));

        // Legs: turned balusters with a square block under the apron, a
        // ring and a foot (square posts on Low).
        let mut legs = MeshBuilder::default();
        let leg_h = bottom;
        for sx in [1.0f32, -1.0] {
            for sz in [1.0f32, -1.0] {
                let at = vec3(sx * (l / 2.0 - 0.02), 0.0, sz * (w / 2.0 - 0.02));
                if quality == Quality::Low {
                    legs.cuboid(
                        at + Vec3::Y * leg_h / 2.0,
                        vec3(0.19, leg_h, 0.19),
                        0.0,
                        WHITE,
                        1.2,
                    );
                    continue;
                }
                let block = 0.16;
                legs.cuboid(
                    at + Vec3::Y * (leg_h - block / 2.0),
                    vec3(0.2, block, 0.2),
                    0.0,
                    WHITE,
                    1.2,
                );
                let h = leg_h - block;
                let profile = [
                    (0.0, 0.0),
                    (0.0, 0.105),
                    (0.025, 0.11),
                    (0.05, 0.1),
                    (0.07, 0.075),
                    (0.11, 0.07),
                    (0.16, 0.085),
                    (0.22, 0.1),
                    (0.30, 0.105),
                    (0.36, 0.095),
                    (0.40, 0.07),
                    (h * 0.75, 0.058),
                    (h * 0.8, 0.068),
                    (h * 0.84, 0.072),
                    (h * 0.88, 0.06),
                    (h - 0.015, 0.068),
                    (h, 0.085),
                ];
                legs.lathe(at, &profile, if quality == Quality::Ultra { 36 } else { 20 }, WHITE);
            }
        }
        add(legs, &body_wood, Surface::WOOD);

        // Diamond sights.
        let mut sights = MeshBuilder::default();
        let rail_mid = off as f64;
        for k in (1..8).filter(|k| *k != 4) {
            let x = -spec.length / 2.0 + k as f64 * spec.length / 8.0;
            for sy in [1.0, -1.0] {
                sights.disc(
                    surface(DVec2::new(x, sy * (spec.width / 2.0 + rail_mid)), RAIL_TOP + 0.0006),
                    0.0065,
                    12,
                    rgb(235, 228, 205),
                );
            }
        }
        for k in 1..4 {
            let yy = -spec.width / 2.0 + k as f64 * spec.width / 4.0;
            for sx in [1.0, -1.0] {
                sights.disc(
                    surface(DVec2::new(sx * (spec.length / 2.0 + rail_mid), yy), RAIL_TOP + 0.0006),
                    0.0065,
                    12,
                    rgb(235, 228, 205),
                );
            }
        }
        let mut rail_details =
            vec![Part { mesh: sights.build(&white), surface: Surface::lit(0.8, 60.0, 0.0, 0.3) }];

        // Pockets. The cloth, cushions and rails are cut by the shader along
        // the throat (between the jaws) and the hole; the hole's wall closes
        // the cut: it rises to the rail top on the rail side and starts at the
        // cloth on the throat side.
        let mut inside = MeshBuilder::default();
        let mut leather = MeshBuilder::default();
        let mut holes = [Vec4::ZERO; 6];
        let mut throats = [Vec4::ZERO; 12];
        for (k, pocket) in table.pockets.iter().enumerate() {
            let id = pocket.id;
            let r = pocket_radius_of(&table, id);
            let c2 = pocket_center_of(&table, id);
            let center = surface(c2, 0.0);
            // Side pockets: w = where the cushion line is, across the table
            // (the leather lip must not reach in front of it, over the cloth).
            let edge = if id.is_side() { (table.spec.width / 2.0) as f32 + 0.008 } else { 0.0 };
            holes[k] = vec4(center.x, center.z, r, edge);
            let jaws: Vec<(DVec2, DVec2)> = table
                .cushions
                .iter()
                .filter(|c| !is_axis_aligned(c.a, c.b))
                .filter(|c| c.a.distance(pocket.mouth_center) < pocket.half_mouth + 1e-6)
                .map(|c| {
                    let end = clip_to_hole(&table, c.a, c.b);
                    (c.a, end + (end - c.a).normalize() * 0.0015)
                })
                .collect();
            if let [(a1, b1), (a2, b2)] = jaws[..] {
                let w = |p: DVec2| {
                    let q = surface(p, 0.0);
                    vec2(q.x, q.z)
                };
                let (p0, p1, p2, p3) = (w(a1), w(b1), w(b2), w(a2));
                throats[2 * k] = vec4(p0.x, p0.y, p1.x, p1.y);
                throats[2 * k + 1] = vec4(p2.x, p2.y, p3.x, p3.y);
            }
            let top = RAIL_TOP + CAST_H + 0.0008;
            let bands = [
                (top, rgb(36, 24, 17)),
                (0.0, rgb(24, 16, 11)),
                (-0.04, rgb(9, 7, 6)),
                (-0.11, rgb(4, 3, 3)),
            ];
            for w in bands.windows(2) {
                let (y0, c0) = w[0];
                let (y1, _) = w[1];
                inside.tube_inside(center + Vec3::Y * y1, r, y0 - y1, 48, c0);
            }
            inside.disc(center - Vec3::Y * 0.11, r, 48, rgb(3, 3, 3));
            // Leather lip around the hole on the rail (cut away at the throat).
            leather.ring(center + Vec3::Y * (top + 0.0004), r, r + 0.011, 48, rgb(46, 28, 18));
        }
        let pocket_inside = inside.build(&white);
        rail_details.push(Part {
            mesh: leather.build(&white),
            surface: Surface::lit(0.5, 30.0, 0.0, 0.2).cut(2.0),
        });

        // Soft shadow of the table on the floor.
        let mut fs = MeshBuilder::default();
        let (sx, sz) = (l / 2.0 + 0.6, w / 2.0 + 0.6);
        fs.quad(
            [
                vec3(-sx, 0.002, sz),
                vec3(sx, 0.002, sz),
                vec3(sx, 0.002, -sz),
                vec3(-sx, 0.002, -sz),
            ],
            [vec2(0.0, 1.0), vec2(1.0, 1.0), vec2(1.0, 0.0), vec2(0.0, 0.0)],
            Color::new(1.0, 1.0, 1.0, 0.75),
        );
        let floor_shadow = Part { mesh: fs.build(&shadow), surface: Surface::UNLIT };

        // --- Balls and cue --------------------------------------------------
        let r = spec.ball_radius as f32;
        let balls = ball_pixels(ball_tex, &looks.balls)
            .into_iter()
            .map(|p| gfx::sphere_mesh(r, &upload(p, false), ball_segments))
            .collect();
        let cue = cue_mesh("cue_classic", "chalk_blue", &white);
        let ghost = gfx::sphere_mesh_tinted(r, &white, 32, Color::new(1.0, 1.0, 1.0, 0.28));

        let lamps = [
            vec3(-0.45, LAMP_Y - 0.02, 0.0),
            vec3(0.0, LAMP_Y - 0.02, 0.0),
            vec3(0.45, LAMP_Y - 0.02, 0.0),
        ];
        Scene {
            table,
            materials: Materials::new(),
            parts,
            lamp_glow,
            lamp_parts,
            pocket_inside,
            holes,
            throats,
            rail_details,
            floor_shadow,
            room,
            balls,
            cue,
            ghost,
            white,
            shadow,
            lamps,
            sinking: HashMap::new(),
            cues: HashMap::new(),
            lamp_visible: !focused,
            looks: looks.clone(),
            wanted: looks.clone(),
            felt_parts,
            ball_tex_size: ball_tex,
            felt_size,
            making: None,
            uploading: None,
            lamp_count: if quality == Quality::Low { 1 } else { 3 },
            focus: if focused { 1.0 } else { 0.0 },
            soft_shadows: matches!(quality, Quality::High | Quality::Ultra),
        }
    }

    /// Dresses the table with another cloth or ball set. The textures are
    /// made on another thread and sent to the GPU a few at a time, so the
    /// game never stops (buying an item used to freeze it for a second).
    pub fn set_looks(&mut self, looks: &Looks) {
        if *looks == self.wanted {
            return;
        }
        self.wanted = looks.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let cloth = (looks.cloth != self.looks.cloth).then_some(looks.cloth);
        let balls = (looks.balls != self.looks.balls).then_some(looks.balls);
        let (felt_size, ball_size) = (self.felt_size, self.ball_tex_size);
        let target = looks.clone();
        std::thread::spawn(move || {
            let felt = cloth.map(|c| textures::felt_sized(c, felt_size));
            let balls = balls.map(|p| ball_pixels(ball_size, &p));
            let _ = tx.send(Made { looks: target, felt, balls });
        });
        // A newer request replaces one still being made.
        self.making = Some(rx);
        self.uploading = None;
    }

    /// Takes finished textures and puts them on the table.
    fn update_looks(&mut self) {
        if let Some(rx) = &self.making {
            match rx.try_recv() {
                Ok(made) => {
                    self.making = None;
                    self.uploading = Some((made, Vec::new()));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.making = None,
            }
        }
        let Some((made, done)) = self.uploading.as_mut() else { return };
        // A few textures a frame (a big ball texture is a few megabytes).
        let mut budget = 4;
        if let Some(felt) = made.felt.take() {
            let tex = upload(felt, true);
            for &i in &self.felt_parts {
                self.parts[i].mesh.set_texture(&tex);
            }
            budget -= 1;
        }
        if let Some(balls) = made.balls.as_mut() {
            while budget > 0 && !balls.is_empty() {
                done.push(upload(balls.remove(0), false));
                budget -= 1;
            }
            if !balls.is_empty() {
                return;
            }
            // The whole set at once: never two sets on the table.
            for (mesh, tex) in self.balls.iter_mut().zip(done.iter()) {
                mesh.set_texture(tex);
            }
        }
        self.looks = made.looks.clone();
        self.uploading = None;
    }

    pub fn ball_radius(&self) -> f32 {
        self.table.spec.ball_radius as f32
    }

    /// Draws the whole 3D scene.
    pub fn draw(&mut self, cam: &SceneCamera, balls: &[Ball], dt: f32, overlay: &Overlay) {
        self.update_looks();
        set_camera(cam);
        self.materials.set_frame(cam.eye, self.lamps, 1.55, self.lamp_count, self.focus);
        let mut casters = [Vec4::ZERO; 16];
        if self.soft_shadows {
            let r = self.ball_radius();
            for (slot, b) in casters.iter_mut().zip(balls.iter().filter(|b| b.in_play())) {
                let p = to_world(b.position);
                *slot = vec4(p.x, p.y, p.z, r);
            }
        }
        // The cue mesh runs along its local +Y from the tip (see `cue_mesh`).
        let cue = overlay.cue.as_ref().map(|c| {
            let butt = c.tip + c.rotation * (Vec3::Y * CUE_LENGTH);
            (c.tip.extend(0.0061), butt.extend(0.0148))
        });
        self.materials.set_shadows(self.soft_shadows, self.soft_shadows, &casters, cue);
        self.materials.set_pockets(&self.holes, &self.throats);
        let skip: Option<usize> =
            crate::dev::var("SNOOKERED_SKIP_PART").and_then(|v| v.parse().ok());
        for (k, part) in self.parts.iter().enumerate() {
            if skip == Some(k) {
                continue;
            }
            self.materials.use_lit(part.surface);
            part.mesh.draw();
        }
        for part in self.room.iter().filter(|p| !p.blended) {
            self.materials.use_lit(part.surface);
            part.mesh.draw();
        }
        if self.focus < 0.5 {
            self.materials.use_blended(Surface::UNLIT);
            self.floor_shadow.mesh.draw();
            for part in self.room.iter().filter(|p| p.blended) {
                part.mesh.draw();
            }
        }
        if !self.soft_shadows {
            self.draw_ball_shadows(balls);
        }
        self.materials.use_lit(Surface::UNLIT.cut(3.0));
        self.pocket_inside.draw();
        for part in &self.rail_details {
            self.materials.use_lit(part.surface);
            part.mesh.draw();
        }
        if self.lamp_visible && self.focus < 0.5 {
            for part in &self.lamp_parts {
                self.materials.use_lit(part.surface);
                part.mesh.draw();
            }
            self.materials.use_lit(self.lamp_glow.surface);
            self.lamp_glow.mesh.draw();
        }
        self.draw_balls(balls, dt);
        if let Some(cue) = &overlay.cue {
            // The shooter's own cue and chalk (built once per design).
            let key = (overlay.cue_look.0.clone(), overlay.cue_look.1.clone());
            if !key.0.is_empty() && !self.cues.contains_key(&key) {
                let mesh = cue_mesh(&key.0, &key.1, &self.white);
                self.cues.insert(key.clone(), mesh);
            }
            self.materials.use_lit(Surface::WOOD);
            let model = Mat4::from_rotation_translation(cue.rotation, cue.tip);
            self.cues.get(&key).unwrap_or(&self.cue).draw_at(model);
        }
        self.draw_guides(&overlay.guides, cam.eye);
        if let Some(p) = overlay.ghost {
            self.materials.use_blended(Surface::lit(0.6, 60.0, 0.0, 0.0));
            self.ghost.draw_at(Mat4::from_translation(p));
        }
        gl_use_default_material();
        set_default_camera();
    }

    fn draw_ball_shadows(&self, balls: &[Ball]) {
        let r = self.ball_radius();
        let mut b = MeshBuilder::default();
        for ball in balls.iter().filter(|b| b.in_play()) {
            let p = to_world(ball.position);
            let height = (p.y - TABLE_HEIGHT - r).max(0.0);
            // Offset away from the nearest lamp, like a real overhead light.
            let lamp = self
                .lamps
                .iter()
                .min_by(|a, c| a.distance(p).total_cmp(&c.distance(p)))
                .copied()
                .unwrap_or(Vec3::Y);
            let away = vec3(p.x - lamp.x, 0.0, p.z - lamp.z) * 0.035;
            let size = r * (1.35 + height * 6.0);
            let alpha = (0.7 - height * 4.0).clamp(0.15, 0.7);
            let c = vec3(p.x + away.x, TABLE_HEIGHT + 0.0008, p.z + away.z);
            b.quad(
                [
                    c + vec3(-size, 0.0, size),
                    c + vec3(size, 0.0, size),
                    c + vec3(size, 0.0, -size),
                    c + vec3(-size, 0.0, -size),
                ],
                [vec2(0.0, 1.0), vec2(1.0, 1.0), vec2(1.0, 0.0), vec2(0.0, 0.0)],
                Color::new(1.0, 1.0, 1.0, alpha),
            );
            // A tight, darker contact shadow right under the ball.
            if height < 0.004 {
                let c = vec3(p.x, TABLE_HEIGHT + 0.0009, p.z);
                let s = r * 0.8;
                b.quad(
                    [
                        c + vec3(-s, 0.0, s),
                        c + vec3(s, 0.0, s),
                        c + vec3(s, 0.0, -s),
                        c + vec3(-s, 0.0, -s),
                    ],
                    [vec2(0.0, 1.0), vec2(1.0, 1.0), vec2(1.0, 0.0), vec2(0.0, 0.0)],
                    Color::new(1.0, 1.0, 1.0, 0.75),
                );
            }
        }
        if !b.vertices.is_empty() {
            self.materials.use_blended(Surface::UNLIT.cut(1.0));
            b.build(&self.shadow).draw();
        }
    }

    fn draw_balls(&mut self, balls: &[Ball], dt: f32) {
        self.materials.use_lit(Surface::BALL);
        for ball in balls {
            let mesh = &self.balls[ball.id.index()];
            match ball.status {
                BallStatus::InPlay => {
                    self.sinking.remove(&ball.id);
                    let model = Mat4::from_rotation_translation(
                        ball_rotation(ball.orientation),
                        to_world(ball.position),
                    );
                    mesh.draw_at(model);
                }
                BallStatus::Pocketed(pocket) => {
                    let start = to_world(ball.position);
                    let entry = self.sinking.entry(ball.id).or_insert((0.0, start));
                    if entry.0 >= SINK_TIME {
                        continue;
                    }
                    entry.0 += dt;
                    let k = (entry.0 / SINK_TIME).clamp(0.0, 1.0);
                    // Roll over the lip into the hole, then drop to the bottom.
                    let hole = surface(pocket_center_of(&self.table, pocket), 0.0);
                    let start = entry.1;
                    let r = self.table.spec.ball_radius as f32;
                    let across = (k / 0.45).min(1.0);
                    let xz = start.lerp(hole, across * across * (3.0 - 2.0 * across) * 0.8);
                    let fall = (k * k) * (0.11 - r + 0.012);
                    let pos = vec3(xz.x, start.y - fall, xz.z);
                    let model =
                        Mat4::from_rotation_translation(ball_rotation(ball.orientation), pos);
                    mesh.draw_at(model);
                }
                BallStatus::OffTable => {}
            }
        }
    }

    fn draw_guides(&self, guides: &[Guide], eye: Vec3) {
        // Keep lines at least ~1.5 px wide however far the camera is.
        let px = |p: Vec3, w: f32| w.max(p.distance(eye) * 0.0016);
        if guides.is_empty() {
            return;
        }
        let mut b = MeshBuilder::default();
        for g in guides {
            match *g {
                Guide::Line { from, to, width, color } => {
                    let d = to - from;
                    if d.length() < 1e-5 {
                        continue;
                    }
                    // Split long lines so the width can grow with distance.
                    let steps = (d.length() / 0.1).ceil().max(1.0) as usize;
                    let dir = vec3(-d.z, 0.0, d.x).normalize();
                    for k in 0..steps {
                        let a = from + d * (k as f32 / steps as f32);
                        let c = from + d * ((k + 1) as f32 / steps as f32);
                        let wa = dir * px(a, width) / 2.0;
                        let wc = dir * px(c, width) / 2.0;
                        b.quad([a - wa, c - wc, c + wc, a + wa], [Vec2::ZERO; 4], color);
                    }
                }
                Guide::Ring { center, radius, width, color } => {
                    let w = px(center, width);
                    b.ring(center, radius - w / 2.0, radius + w / 2.0, 40, color);
                }
            }
        }
        self.materials.use_blended(Surface::UNLIT);
        b.build(&self.white).draw();
    }

    /// Position and orientation of the cue for the given stroke.
    pub fn cue_pose(
        &self,
        params: &PhysicsParams,
        cue_ball: DVec3,
        yaw: f64,
        elevation: f64,
        tip: DVec2,
        gap: f64,
    ) -> CuePose {
        let r = self.table.spec.ball_radius;
        let forward = DVec3::new(yaw.cos(), yaw.sin(), 0.0);
        let right = forward.cross(DVec3::Z);
        let (s, c) = elevation.sin_cos();
        let axis = forward * c - DVec3::Z * s;
        let up = DVec3::Z * c + forward * s;
        let a = tip.x * params.max_tip_offset * r;
        let b = tip.y * params.max_tip_offset * r;
        let depth = (r * r - a * a - b * b).max(0.0).sqrt();
        let contact = right * a + up * b - axis * depth;
        let tip_pos = cue_ball + contact - axis * gap;
        CuePose {
            tip: to_world(tip_pos),
            rotation: Quat::from_rotation_arc(Vec3::Y, -dir_to_world(axis).normalize()),
        }
    }

    /// Centre of the hole drawn for `pocket` (for highlights and clicks).
    pub fn pocket_center(&self, pocket: PocketId) -> DVec2 {
        pocket_center_of(&self.table, pocket)
    }

    pub fn pocket_radius(&self, pocket: PocketId) -> f32 {
        pocket_radius_of(&self.table, pocket)
    }
}

/// Textures made away from the main thread (see [`Scene::set_looks`]).
struct Made {
    looks: Looks,
    felt: Option<textures::Pixels>,
    balls: Option<Vec<textures::Pixels>>,
}

fn felt_size(quality: Quality) -> u16 {
    if quality == Quality::Ultra { 1024 } else { 512 }
}

/// The sixteen ball textures, made side by side on all cores.
fn ball_pixels(size: u16, palette: &textures::Palette) -> Vec<textures::Pixels> {
    let ids: Vec<BallId> = BallId::all().collect();
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).clamp(1, 8);
    let chunk = ids.len().div_ceil(threads);
    std::thread::scope(|s| {
        let handles: Vec<_> = ids
            .chunks(chunk)
            .map(|part| {
                s.spawn(move || {
                    part.iter().map(|id| textures::ball(*id, size, palette)).collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().expect("ball texture")).collect()
    })
}

/// What the shop changes on the table itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Looks {
    pub cloth: [u8; 3],
    pub balls: textures::Palette,
}

impl Looks {
    /// From the codes of the equipped cloth and ball set.
    pub fn of(cloth: &str, balls: &str) -> Looks {
        let cloth = match snookered_proto::shop::item(cloth).map(|i| i.look) {
            Some(snookered_proto::shop::Look::Cloth(c)) => c,
            _ => [24, 96, 48],
        };
        Looks { cloth, balls: textures::Palette::of(balls) }
    }
}

impl Default for Looks {
    fn default() -> Self {
        Looks::of("cloth_green", "balls_classic")
    }
}

/// A cue of the given design, chalked with `chalk` (shop codes).
pub fn cue_mesh(cue: &str, chalk: &str, white: &Texture2D) -> gfx::Model {
    use snookered_proto::shop::{Look, item};
    let (butt, accent, ring) = match item(cue).map(|i| i.look) {
        Some(Look::Cue { butt, accent, ring, .. }) => (butt, accent, ring),
        _ => ([35, 20, 12], [20, 20, 24], [240, 238, 228]),
    };
    let chalk = match item(chalk).map(|i| i.look) {
        Some(Look::Chalk(c)) => c,
        _ => [40, 80, 150],
    };
    let c = |v: [u8; 3]| rgb(v[0], v[1], v[2]);
    let mut m = MeshBuilder::default();
    m.frustum(-0.004, 0.0060, 0.0, 0.0061, 12, c(chalk)); // tip, chalked
    m.frustum(0.0, 0.0062, 0.018, 0.0063, 12, rgb(240, 238, 228)); // ferrule
    m.frustum(0.018, 0.0063, 0.80, 0.0105, 16, rgb(222, 190, 135)); // maple shaft
    m.frustum(0.80, 0.0105, 0.815, 0.0106, 16, c(ring)); // joint ring
    m.frustum(0.815, 0.0106, 1.10, 0.0125, 16, c(butt)); // forearm
    m.frustum(1.10, 0.0125, 1.115, 0.0126, 16, c(ring)); // ring
    m.frustum(1.115, 0.0126, 1.35, 0.0138, 16, c(accent)); // wrap
    m.frustum(1.35, 0.0138, 1.47, 0.0148, 16, c(butt)); // butt
    m.frustum(1.47, 0.0148, 1.48, 0.0, 16, c(ring)); // cap
    m.build(white)
}

pub struct CuePose {
    pub tip: Vec3,
    pub rotation: Quat,
}

/// Flat markings drawn on the cloth.
pub enum Guide {
    Line { from: Vec3, to: Vec3, width: f32, color: Color },
    Ring { center: Vec3, radius: f32, width: f32, color: Color },
}

/// Per-frame extras drawn over the table.
#[derive(Default)]
pub struct Overlay {
    pub cue: Option<CuePose>,
    /// The shooter's cue and chalk (shop codes; empty: the classic ones).
    pub cue_look: (String, String),
    /// Where the cue ball will touch the first ball: drawn as a see-through ball.
    pub ghost: Option<Vec3>,
    pub guides: Vec<Guide>,
}

pub const RAIL_HEIGHT: f32 = RAIL_TOP;

/// Length of the cue mesh, tip to butt (see `cue_mesh`).
const CUE_LENGTH: f32 = 1.48;

/// Radius of the hole drawn for a pocket: exactly where balls drop.
fn pocket_radius_of(table: &Table, pocket: PocketId) -> f32 {
    table.pocket(pocket).hole_radius as f32
}

/// Centre of the hole drawn for a pocket.
fn pocket_center_of(table: &Table, pocket: PocketId) -> DVec2 {
    table.pocket(pocket).hole_center
}

/// Shortens the segment `a`→`b` so it ends where it enters the nearest pocket hole.
fn clip_to_hole(table: &Table, a: DVec2, b: DVec2) -> DVec2 {
    let Some(pocket) = table
        .pockets
        .iter()
        .min_by(|p, q| p.mouth_center.distance(b).total_cmp(&q.mouth_center.distance(b)))
    else {
        return b;
    };
    let c = pocket_center_of(table, pocket.id);
    let r = pocket_radius_of(table, pocket.id) as f64 + 0.001;
    let d = b - a;
    // Solve |a + t d - c| = r for the first t in [0, 1].
    let f = a - c;
    let (qa, qb, qc) = (d.dot(d), 2.0 * f.dot(d), f.dot(f) - r * r);
    let disc = qb * qb - 4.0 * qa * qc;
    if qa >= 1e-12 && disc >= 0.0 {
        let t = (-qb - disc.sqrt()) / (2.0 * qa);
        if (0.0..=1.0).contains(&t) {
            return a + d * t.max(0.05);
        }
    }
    // The jaw line passes beside the hole: stop where it comes closest to it,
    // so the throat never cuts a slit into the rail beyond the pocket.
    let t = (-f.dot(d) / qa.max(1e-12)).clamp(0.05, 1.0);
    a + d * t
}

fn is_axis_aligned(a: DVec2, b: DVec2) -> bool {
    let d = b - a;
    d.x.abs() < 1e-6 || d.y.abs() < 1e-6
}

/// The face of a pocket jaw: the vertical cut through the cushion rubber and
/// the rail along the jaw line, from `a` (the cushion nose) to `b` (the hole).
/// Its top follows what was cut: the nose slope, the cushion top, then the rail.
fn jaw_wall(
    mb: &mut MeshBuilder,
    rail: &mut MeshBuilder,
    table: &Table,
    a: DVec2,
    b: DVec2,
    into_throat: DVec2,
) {
    let along = b - a;
    let len = along.length();
    if len < 1e-6 {
        return;
    }
    let dir = along / len;
    // Distance from the rail's cushion nose grows along the jaw.
    let rail_normal = table
        .cushions
        .iter()
        .filter(|c| is_axis_aligned(c.a, c.b))
        .find(|c| c.a.distance(a) < 1e-6 || c.b.distance(a) < 1e-6)
        .map(|c| {
            let n = (c.b - c.a).normalize().perp();
            if n.dot(c.a) < 0.0 { -n } else { n }
        })
        .unwrap_or(dir);
    let rate = dir.dot(rail_normal).abs().max(0.05);
    let nose = table.spec.cushion_height as f32;
    let height = |d: f64| -> f32 {
        if d < 0.008 {
            nose + (CUSHION_TOP - nose) * (d / 0.008) as f32
        } else if d < CUSHION_DEPTH as f64 {
            CUSHION_TOP
        } else {
            RAIL_TOP + CAST_H
        }
    };
    // Samples along the jaw, with a step where the rail starts.
    let d_end = len * rate;
    let mut ds: Vec<f64> = vec![0.0, 0.004, 0.008];
    let rail_d = CUSHION_DEPTH as f64;
    ds.extend([rail_d - 1e-4, rail_d + 1e-4]);
    let mut k = 0.012;
    while k < d_end {
        ds.push(k);
        k += 0.01;
    }
    ds.push(d_end);
    ds.retain(|d| *d <= d_end + 1e-9);
    ds.sort_by(|x, y| x.total_cmp(y));
    ds.dedup_by(|x, y| (*x - *y).abs() < 1e-7);
    // Lit like the cushion nose: facing the throat and tilted a little up,
    // so the facings don't read as black slabs.
    let n = surface(into_throat, 0.0) - surface(DVec2::ZERO, 0.0);
    let n = (n.normalize_or_zero() + Vec3::Y * 0.6).normalize_or_zero();
    let felt = rgb(170, 180, 170);
    // Where the jaw cuts through the rail, the facing is the pocket casting
    // (only the rubber part is covered in cloth).
    let cast = CAST_COLOR;
    for w in ds.windows(2) {
        let (d0, d1) = (w[0], w[1]);
        let p0 = a + dir * (d0 / rate);
        let p1 = a + dir * (d1 / rate);
        let (h0, h1) = (height(d0), height(d1));
        let in_rail = d0 >= rail_d - 1e-6;
        let (target, color) = if in_rail { (&mut *rail, cast) } else { (&mut *mb, felt) };
        // The facing goes on down into the pocket (seen where the mouth opens).
        let quad = [surface(p0, -0.06), surface(p1, -0.06), surface(p1, h1), surface(p0, h0)];
        let i: Vec<u16> = quad.iter().map(|q| target.push(*q, Vec2::ZERO, n, color)).collect();
        target.indices.extend_from_slice(&[i[0], i[1], i[2], i[0], i[2], i[3]]);
    }
}

/// A rounded rectangle around the origin in the world XZ plane (as `Vec2`
/// x, z), densely sampled. Loops with different radii have matching points,
/// so rings can be built between them.
fn rounded_loop(ex: f32, ez: f32, radius: f32) -> Vec<Vec2> {
    const CORNER: usize = 12;
    const STEP: f32 = 0.02;
    let r = radius.min(ex).min(ez);
    let corners = [
        (vec2(ex - r, ez - r), 0.0f32),
        (vec2(-(ex - r), ez - r), 90.0),
        (vec2(-(ex - r), -(ez - r)), 180.0),
        (vec2(ex - r, -(ez - r)), 270.0),
    ];
    // A fixed number of points per side, so every loop has the same count.
    let counts = [((2.2 / STEP) as usize).max(4), ((1.3 / STEP) as usize).max(4)];
    let mut pts = Vec::new();
    for (k, (c, a0)) in corners.iter().enumerate() {
        for j in 0..=CORNER {
            let a = (a0 + 90.0 * j as f32 / CORNER as f32).to_radians();
            pts.push(*c + vec2(a.cos(), a.sin()) * r);
        }
        // The straight side to the next corner (exclusive of both ends).
        let (c1, a1) = corners[(k + 1) % 4];
        let end = c1 + vec2(a1.to_radians().cos(), a1.to_radians().sin()) * r;
        let start = *pts.last().expect("corner points");
        let m = counts[k % 2];
        for j in 1..m {
            pts.push(start.lerp(end, j as f32 / m as f32));
        }
    }
    pts
}
