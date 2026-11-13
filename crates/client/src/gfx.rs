//! Low-level rendering: the lighting shader, mesh building and textures.
//!
//! Everything targets OpenGL 2.1 / GLES 2.0 (GLSL 100), so the game runs on
//! practically any PC, including old integrated GPUs.

use macroquad::miniquad::{
    BlendFactor, BlendState, BlendValue, Comparison, CullFace, Equation, FilterMode,
    MipmapFilterMode, PipelineParams, TextureWrap, UniformDesc, UniformType,
};
use macroquad::prelude::*;

pub const LAMP_COUNT: usize = 3;

const VERTEX: &str = r#"#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;
attribute vec4 normal;

varying lowp vec2 uv;
varying lowp vec4 color;
varying highp vec3 v_normal;
varying highp vec3 v_world;

uniform mat4 Model;
uniform mat4 Projection;

void main() {
    vec4 world = Model * vec4(position, 1.0);
    v_world = world.xyz;
    v_normal = (Model * vec4(normal.xyz, 0.0)).xyz;
    uv = texcoord;
    color = color0 / 255.0;
    gl_Position = Projection * world;
}
"#;

const FRAGMENT: &str = r#"#version 100
#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif

varying lowp vec2 uv;
varying lowp vec4 color;
varying highp vec3 v_normal;
varying highp vec3 v_world;

uniform sampler2D Texture;
uniform highp vec3 CameraPos;
uniform highp vec3 Lamp0;
uniform highp vec3 Lamp1;
uniform highp vec3 Lamp2;
uniform vec4 LampColor;      // rgb, w = intensity
uniform vec4 Ambient;        // rgb, w = number of lamps (1 or 3)
uniform vec4 Surface;        // x = specular, y = shininess, z = unlit, w = rim
uniform vec4 Extra;          // x = clear coat (glossy reflection of the room)
uniform vec4 Shadows;        // x = soft shadows on, y = occlusion on, z = cloth height
uniform vec4 Balls[16];      // xyz = centre, w = radius (0 = empty slot)
uniform vec4 Stage;          // x = focus: the room fades into darkness away from the table
uniform vec4 Holes[6];       // pocket holes: x, z (world), radius
uniform vec4 Throats[12];    // pocket throats: 4 corners (x, z) per pocket
uniform vec4 CueA;           // the cue's tip: xyz, w = radius (0: no cue)
uniform vec4 CueB;           // the cue's butt: xyz, w = radius

// The pocket cut-outs. Extra.y selects what a surface loses:
// 1 = the holes (cloth), 2 = holes and throats (rails, cushions),
// 3 = the throat side of a pocket wall above the cloth (the well),
// 4 = the holes, on upward faces only (the table body under the cloth).
bool in_quad(vec2 p, vec2 a, vec2 b, vec2 c, vec2 d) {
    vec2 e0 = b - a; vec2 e1 = c - b; vec2 e2 = d - c; vec2 e3 = a - d;
    float s0 = e0.x * (p.y - a.y) - e0.y * (p.x - a.x);
    float s1 = e1.x * (p.y - b.y) - e1.y * (p.x - b.x);
    float s2 = e2.x * (p.y - c.y) - e2.y * (p.x - c.x);
    float s3 = e3.x * (p.y - d.y) - e3.y * (p.x - d.x);
    return (s0 >= 0.0 && s1 >= 0.0 && s2 >= 0.0 && s3 >= 0.0)
        || (s0 <= 0.0 && s1 <= 0.0 && s2 <= 0.0 && s3 <= 0.0);
}

// How far the cloth has dropped into the nearest pocket (0 flat … 1 at the
// hole's edge): the cloth rolls over the edge, so it darkens as it goes.
float pocket_drop(vec3 p) {
    float k = 0.0;
    for (int i = 0; i < 6; i++) {
        vec4 h = Holes[i];
        float d = length(p.xz - h.xy) - h.z;
        k = max(k, 1.0 - smoothstep(0.0, 0.028, d));
    }
    return k;
}

bool cut_away(vec3 p, float mode, bool n_up) {
    for (int i = 0; i < 6; i++) {
        vec4 h = Holes[i];
        vec2 d = p.xz - h.xy;
        bool hole = dot(d, d) < h.z * h.z;
        vec4 q0 = Throats[2 * i];
        vec4 q1 = Throats[2 * i + 1];
        bool throat = in_quad(p.xz, q0.xy, q0.zw, q1.xy, q1.zw);
        if (mode < 1.5) {
            if (hole) return true;
        } else if (mode < 2.5) {
            if (hole || throat) return true;
            // Nothing of the rail (the leather lip around a side pocket)
            // reaches in front of the cushion line, over the cloth.
            if (h.w > 0.0 && abs(p.z) < h.w - 0.008 && dot(d, d) < (h.z + 0.03) * (h.z + 0.03)) return true;
        } else if (mode < 3.5) {
            if (throat && p.y > Shadows.z + 0.0005) return true;
        } else {
            if (hole && n_up) return true;
        }
    }
    return false;
}

// How much of one lamp's light reaches `p` past one ball (soft sphere shadow:
// the penumbra grows with distance, like under a shade about 8 cm across).
float sphere_shadow(vec3 oc, float oc2, vec3 ld, float rad) {
    float ll = length(ld);
    ld /= ll;
    float t = dot(oc, ld);
    if (t <= 0.0) {
        return 1.0;
    }
    float d = sqrt(max(oc2 - t * t, 0.0));
    float pen = 0.045 * t / ll + 0.0015;
    return mix(0.12, 1.0, smoothstep(rad - pen, rad + pen, d));
}

// How much of one lamp's light reaches `p` past the cue: a rod tapering
// from CueA to CueB. The closest points of the ray towards the lamp and the
// rod's axis give the distance; the penumbra widens with the distance to
// the cue, like the balls' shadows.
float cue_shadow(vec3 p, vec3 lamp) {
    vec3 d1 = lamp - p;
    vec3 d2 = CueB.xyz - CueA.xyz;
    vec3 r = p - CueA.xyz;
    float a = dot(d1, d1);
    float e = dot(d2, d2);
    float f = dot(d2, r);
    float c = dot(d1, r);
    float b = dot(d1, d2);
    float denom = a * e - b * b;
    float s = denom > 1e-9 ? clamp((b * f - c * e) / denom, 0.0, 1.0) : 0.0;
    float t = (b * s + f) / e;
    if (t < 0.0) {
        t = 0.0;
        s = clamp(-c / a, 0.0, 1.0);
    } else if (t > 1.0) {
        t = 1.0;
        s = clamp((b - c) / a, 0.0, 1.0);
    }
    if (s <= 0.0) {
        return 1.0;
    }
    float dist = length(p + d1 * s - (CueA.xyz + d2 * t));
    float rad = mix(CueA.w, CueB.w, t);
    float pen = 0.045 * s + 0.0015;
    return mix(0.3, 1.0, smoothstep(rad - pen, rad + pen, dist));
}

// Ball shadows for the three lamps (xyz) and contact occlusion (w) at `p`.
// One pass over the balls; balls more than ~22 cm away are skipped early,
// and the list ends at the first empty slot.
vec4 ball_terms(vec3 p, vec3 n) {
    vec4 r = vec4(1.0);
#ifdef NO_SHADOWS
    return r;
#else
    if (Shadows.x < 0.5 || abs(p.y - Shadows.z) > 0.12) {
        return r;
    }
    float occ = 0.0;
    bool three = Ambient.w > 1.5;
    for (int i = 0; i < 16; i++) {
        vec4 b = Balls[i];
        if (b.w <= 0.0) {
            break;
        }
        vec3 oc = b.xyz - p;
        if (dot(oc.xz, oc.xz) > 0.05) {
            continue;
        }
        float oc2 = dot(oc, oc);
        if (oc2 < b.w * b.w * 1.005) {
            continue;
        }
        if (Shadows.y > 0.5) {
            occ += max(dot(n, oc), 0.0) * inversesqrt(oc2) * (b.w * b.w) / oc2;
        }
        r.y *= sphere_shadow(oc, oc2, Lamp1 - p, b.w);
        if (three) {
            r.x *= sphere_shadow(oc, oc2, Lamp0 - p, b.w);
            r.z *= sphere_shadow(oc, oc2, Lamp2 - p, b.w);
        }
    }
    r.w = clamp(1.0 - occ * 1.3, 0.2, 1.0);
    // The cue (not on itself: points on or next to its surface are skipped).
    if (CueA.w > 0.0) {
        vec3 ab = CueB.xyz - CueA.xyz;
        float k = clamp(dot(p - CueA.xyz, ab) / dot(ab, ab), 0.0, 1.0);
        if (length(p - CueA.xyz - ab * k) > mix(CueA.w, CueB.w, k) * 1.25) {
            r.y *= cue_shadow(p, Lamp1);
            if (three) {
                r.x *= cue_shadow(p, Lamp0);
                r.z *= cue_shadow(p, Lamp2);
            }
        }
    }
    return r;
#endif
}

// What a glossy surface reflects: the dark bar, the green cloth below and the
// bright lamps above the table. Cheap and analytic, no textures needed.
vec3 lamp_reflection(vec3 pos, vec3 r) {
    vec3 l = normalize(pos - v_world);
    float d = dot(r, l);
    return LampColor.rgb * LampColor.w * (smoothstep(0.975, 0.995, d) * 1.6 + smoothstep(0.80, 0.99, d) * 0.08);
}

vec3 environment(vec3 r) {
    vec3 room = vec3(0.09, 0.06, 0.045);
    vec3 cloth = vec3(0.03, 0.17, 0.08);
    vec3 c = mix(room, cloth, smoothstep(-0.05, -0.45, r.y));
    c += vec3(0.05, 0.035, 0.02) * smoothstep(0.2, 0.9, r.y);
    c += lamp_reflection(Lamp1, r);
    if (Ambient.w > 1.5) {
        c += lamp_reflection(Lamp0, r);
        c += lamp_reflection(Lamp2, r);
    }
    return c;
}

vec3 lamp(vec3 pos, vec3 n, vec3 v, vec3 base, float shadow) {
    vec3 l = pos - v_world;
    float d = length(l);
    l /= d;
    // Downward cone of a billiard lamp.
    float cone = smoothstep(0.25, 0.75, l.y);
    float att = LampColor.w / (1.0 + 0.9 * d * d);
    float diff = max(dot(n, l), 0.0);
    vec3 h = normalize(l + v);
    float spec = pow(max(dot(n, h), 0.0), Surface.y) * Surface.x;
    return LampColor.rgb * (diff * base + spec) * att * cone * shadow;
}

void main() {
    if (Extra.y > 0.5 && cut_away(v_world, Extra.y, v_normal.y > 0.5)) {
        discard;
    }
    vec4 base = texture2D(Texture, uv) * color;
    if (Surface.z > 0.5) {
        gl_FragColor = base;
        return;
    }
    vec3 n = normalize(v_normal);
    vec3 v = normalize(CameraPos - v_world);
#ifdef DEBUG_NORMALS
    // Development check: faces whose normal points away from the viewer
    // (they are lit from behind and look dark) show up red.
    if (dot(n, v) < -0.02) {
        gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0);
        return;
    }
#endif
    // Hemisphere ambient: a little brighter from above, a green bounce from the cloth below.
    float hemi = 0.5 + 0.5 * n.y;
    vec3 amb = Ambient.rgb * (0.55 + 0.6 * hemi) + vec3(-0.02, 0.05, 0.01) * max(-n.y, 0.0);
    vec4 bt = ball_terms(v_world, n);
    // "Focused" stage: the room falls off into darkness around the table.
    float away = smoothstep(1.3, 3.2, length(v_world.xz));
    amb *= mix(1.0, 0.12, away * Stage.x);
    vec3 c = amb * base.rgb * bt.w;
    // A pool of light on the table that fades towards the rails and beyond.
    float spot = length(v_world.xz / vec2(1.25, 0.72));
    float pool = mix(1.0, mix(1.0, 0.42, smoothstep(0.3, 1.35, spot)), Stage.x);
    c += lamp(Lamp1, n, v, base.rgb, bt.y);
    if (Ambient.w > 1.5) {
        c += lamp(Lamp0, n, v, base.rgb, bt.x);
        c += lamp(Lamp2, n, v, base.rgb, bt.z);
    }
    c *= pool;
    // A little rim light keeps dark objects readable.
    float rim = pow(1.0 - max(dot(n, v), 0.0), 3.0) * Surface.w;
    c += rim * vec3(0.35, 0.3, 0.25);
    if (Extra.x > 0.0) {
        float ndv = max(dot(n, v), 0.0);
        float fresnel = 0.04 + 0.96 * pow(1.0 - ndv, 5.0);
        vec3 r = reflect(-v, n);
        c += environment(r) * Extra.x * (0.05 + 0.75 * fresnel);
    }
    // The cloth darkens where it rolls over into a pocket.
    if (Extra.y > 0.5 && Extra.y < 1.5) {
        float drop = pocket_drop(v_world);
        c *= 1.0 - 0.82 * drop * drop;
    }
    // Filmic-style exposure curve: highlights (white balls under the lamps)
    // roll off smoothly instead of burning out to flat white.
    c = vec3(1.0) - exp(-c * 1.45);
    gl_FragColor = vec4(c, base.a);
}
"#;

/// How a surface reacts to light.
#[derive(Clone, Copy)]
pub struct Surface {
    pub specular: f32,
    pub shininess: f32,
    pub unlit: bool,
    pub rim: f32,
    /// Glossy clear-coat reflection (balls, varnish, glass).
    pub coat: f32,
    /// Pocket cut-out mode (see `cut_away` in the shader); 0 = none.
    pub cut: f32,
}

impl Surface {
    pub const MATTE: Surface =
        Surface { specular: 0.04, shininess: 8.0, unlit: false, rim: 0.0, coat: 0.0, cut: 0.0 };
    pub const WOOD: Surface =
        Surface { specular: 0.35, shininess: 40.0, unlit: false, rim: 0.05, coat: 0.35, cut: 0.0 };
    pub const BALL: Surface =
        Surface { specular: 2.2, shininess: 380.0, unlit: false, rim: 0.03, coat: 0.8, cut: 0.0 };
    pub const UNLIT: Surface =
        Surface { specular: 0.0, shininess: 1.0, unlit: true, rim: 0.0, coat: 0.0, cut: 0.0 };
    pub const fn lit(specular: f32, shininess: f32, rim: f32, coat: f32) -> Surface {
        Surface { specular, shininess, unlit: false, rim, coat, cut: 0.0 }
    }

    /// The same surface with pocket cut-outs (see the shader's `cut_away`).
    pub const fn cut(mut self, mode: f32) -> Surface {
        self.cut = mode;
        self
    }
}

/// The shared lit material plus a blended variant for decals and shadows.
pub struct Materials {
    pub lit: Material,
    pub blended: Material,
}

impl Materials {
    pub fn new() -> Self {
        let uniforms = vec![
            UniformDesc::new("CameraPos", UniformType::Float3),
            UniformDesc::new("Lamp0", UniformType::Float3),
            UniformDesc::new("Lamp1", UniformType::Float3),
            UniformDesc::new("Lamp2", UniformType::Float3),
            UniformDesc::new("LampColor", UniformType::Float4),
            UniformDesc::new("Ambient", UniformType::Float4),
            UniformDesc::new("Surface", UniformType::Float4),
            UniformDesc::new("Extra", UniformType::Float4),
            UniformDesc::new("Shadows", UniformType::Float4),
            UniformDesc::new("Balls", UniformType::Float4).array(16),
            UniformDesc::new("Stage", UniformType::Float4),
            UniformDesc::new("Holes", UniformType::Float4).array(6),
            UniformDesc::new("Throats", UniformType::Float4).array(12),
            UniformDesc::new("CueA", UniformType::Float4),
            UniformDesc::new("CueB", UniformType::Float4),
        ];
        // Some old drivers reject the shadow loop; fall back to a shader
        // without it rather than failing to start.
        static SIMPLE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        let simple: &'static str = SIMPLE.get_or_init(|| {
            FRAGMENT.replacen("#version 100\n", "#version 100\n#define NO_SHADOWS\n", 1)
        });
        let load = |vertex: &'static str, pipeline_params: PipelineParams| -> Material {
            let params =
                || MaterialParams { pipeline_params, uniforms: uniforms.clone(), textures: vec![] };
            // SNOOKERED_SAFE_GRAPHICS=1 forces the basic shader (for troubleshooting).
            static DEBUG: std::sync::OnceLock<String> = std::sync::OnceLock::new();
            let debug: &'static str = DEBUG.get_or_init(|| {
                FRAGMENT.replacen("#version 100\n", "#version 100\n#define DEBUG_NORMALS\n", 1)
            });
            let first = if crate::dev::flag("SNOOKERED_SAFE_GRAPHICS") {
                simple
            } else if crate::dev::flag("SNOOKERED_DEBUG_NORMALS") {
                debug
            } else {
                FRAGMENT
            };
            match load_material(ShaderSource::Glsl { vertex, fragment: first }, params()) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("shader fallback (no ball shadows): {e:?}");
                    load_material(ShaderSource::Glsl { vertex, fragment: simple }, params())
                        .expect("basic shader compiles")
                }
            }
        };
        let lit = load(
            VERTEX,
            PipelineParams {
                depth_test: Comparison::LessOrEqual,
                depth_write: true,
                cull_face: CullFace::Nothing,
                ..Default::default()
            },
        );
        let blended = load(
            VERTEX,
            PipelineParams {
                // miniquad disables the depth *test* when depth writes are off,
                // so decals write depth too (they lie flat on surfaces).
                depth_test: Comparison::LessOrEqual,
                depth_write: true,
                cull_face: CullFace::Nothing,
                color_blend: Some(BlendState::new(
                    Equation::Add,
                    BlendFactor::Value(BlendValue::SourceAlpha),
                    BlendFactor::OneMinusValue(BlendValue::SourceAlpha),
                )),
                ..Default::default()
            },
        );
        Materials { lit, blended }
    }

    /// Sets the per-frame lighting uniforms on both materials.
    /// `lamp_count` is 1 (cheap) or 3; a single lamp is brighter to compensate.
    pub fn set_frame(
        &self,
        camera: Vec3,
        lamps: [Vec3; LAMP_COUNT],
        intensity: f32,
        lamp_count: u32,
        focus: f32,
    ) {
        let intensity = if lamp_count == 1 { intensity * 2.3 } else { intensity };
        for m in [&self.lit, &self.blended] {
            m.set_uniform("CameraPos", camera);
            m.set_uniform("Lamp0", lamps[0]);
            m.set_uniform("Lamp1", lamps[1]);
            m.set_uniform("Lamp2", lamps[2]);
            m.set_uniform("LampColor", vec4(1.0, 0.88, 0.70, intensity));
            m.set_uniform("Ambient", vec4(0.30, 0.25, 0.21, lamp_count as f32));
            m.set_uniform("Stage", vec4(focus, 0.0, 0.0, 0.0));
        }
    }

    /// Turns the analytic ball shadows / contact occlusion on or off and sets
    /// the balls that cast them.
    /// `cue`: the cue's tip and butt (centre and radius), when it is shown.
    pub fn set_shadows(
        &self,
        soft_shadows: bool,
        occlusion: bool,
        balls: &[Vec4; 16],
        cue: Option<(Vec4, Vec4)>,
    ) {
        let (cue_a, cue_b) = cue.filter(|_| soft_shadows).unwrap_or((Vec4::ZERO, Vec4::ZERO));
        for m in [&self.lit, &self.blended] {
            m.set_uniform("CueA", cue_a);
            m.set_uniform("CueB", cue_b);
            m.set_uniform(
                "Shadows",
                vec4(
                    soft_shadows as u8 as f32,
                    occlusion as u8 as f32,
                    crate::coords::TABLE_HEIGHT,
                    0.0,
                ),
            );
            m.set_uniform_array("Balls", &balls[..]);
        }
    }

    pub fn use_lit(&self, s: Surface) {
        set_surface(&self.lit, s);
        gl_use_material(&self.lit);
    }

    /// The pocket holes and throats cut out of the table (see `cut_away`).
    pub fn set_pockets(&self, holes: &[Vec4; 6], throats: &[Vec4; 12]) {
        for m in [&self.lit, &self.blended] {
            m.set_uniform_array("Holes", &holes[..]);
            m.set_uniform_array("Throats", &throats[..]);
        }
    }

    pub fn use_blended(&self, s: Surface) {
        set_surface(&self.blended, s);
        gl_use_material(&self.blended);
    }
}

fn set_surface(m: &Material, s: Surface) {
    m.set_uniform("Surface", vec4(s.specular, s.shininess, if s.unlit { 1.0 } else { 0.0 }, s.rim));
    m.set_uniform("Extra", vec4(s.coat, s.cut, 0.0, 0.0));
}

/// Uploads an RGBA image as a mipmapped, repeating, trilinear-filtered texture.
/// Mipmaps are what keep fine patterns (cloth, wood grain) from shimmering.
pub fn texture(width: u16, height: u16, rgba: &[u8], repeat: bool) -> Texture2D {
    let tex = Texture2D::from_rgba8(width, height, rgba);
    let id = tex.raw_miniquad_id();
    let ctx = unsafe { get_internal_gl() }.quad_context;
    ctx.texture_generate_mipmaps(id);
    ctx.texture_set_min_filter(id, FilterMode::Linear, MipmapFilterMode::Linear);
    ctx.texture_set_mag_filter(id, FilterMode::Linear);
    let wrap = if repeat { TextureWrap::Repeat } else { TextureWrap::Clamp };
    ctx.texture_set_wrap(id, wrap, wrap);
    tex
}

/// A 1x1 white texture, for meshes coloured only through vertex colours.
pub fn white_texture() -> Texture2D {
    texture(1, 1, &[255, 255, 255, 255], true)
}

/// Builds triangle meshes vertex by vertex.
#[derive(Default)]
pub struct MeshBuilder {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u16>,
}

fn vtx(p: Vec3, uv: Vec2, n: Vec3, color: Color) -> Vertex {
    Vertex {
        position: p,
        uv,
        color: [
            (color.r * 255.0) as u8,
            (color.g * 255.0) as u8,
            (color.b * 255.0) as u8,
            (color.a * 255.0) as u8,
        ],
        normal: n.extend(0.0),
    }
}

impl MeshBuilder {
    pub fn push(&mut self, p: Vec3, uv: Vec2, n: Vec3, color: Color) -> u16 {
        assert!(self.vertices.len() < u16::MAX as usize, "mesh too large");
        self.vertices.push(vtx(p, uv, n, color));
        (self.vertices.len() - 1) as u16
    }

    /// Quad `a b c d` (counter-clockwise) with a flat normal.
    pub fn quad(&mut self, p: [Vec3; 4], uv: [Vec2; 4], color: Color) {
        let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
        let i: Vec<u16> = (0..4).map(|k| self.push(p[k], uv[k], n, color)).collect();
        self.indices.extend_from_slice(&[i[0], i[1], i[2], i[0], i[2], i[3]]);
    }

    /// Quad with texture coordinates in metres (for tiling textures) using the
    /// two axes most aligned with the face.
    pub fn quad_world_uv(&mut self, p: [Vec3; 4], color: Color, scale: f32) {
        let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
        let uv = |v: Vec3| {
            if n.y.abs() > 0.7 {
                vec2(v.x, v.z) * scale
            } else if n.x.abs() > n.z.abs() {
                vec2(v.z, v.y) * scale
            } else {
                vec2(v.x, v.y) * scale
            }
        };
        self.quad(p, [uv(p[0]), uv(p[1]), uv(p[2]), uv(p[3])], color);
    }

    /// Axis-aligned box (after an optional rotation about Y) with world-space UVs.
    pub fn cuboid(&mut self, center: Vec3, size: Vec3, yaw: f32, color: Color, uv_scale: f32) {
        let h = size / 2.0;
        let rot = Quat::from_rotation_y(yaw);
        let c = |x: f32, y: f32, z: f32| center + rot * vec3(x * h.x, y * h.y, z * h.z);
        let faces = [
            [c(-1., 1., 1.), c(1., 1., 1.), c(1., 1., -1.), c(-1., 1., -1.)], // top
            [c(-1., -1., -1.), c(1., -1., -1.), c(1., -1., 1.), c(-1., -1., 1.)], // bottom
            [c(-1., -1., 1.), c(1., -1., 1.), c(1., 1., 1.), c(-1., 1., 1.)], // +z
            [c(1., -1., -1.), c(-1., -1., -1.), c(-1., 1., -1.), c(1., 1., -1.)], // -z
            [c(1., -1., 1.), c(1., -1., -1.), c(1., 1., -1.), c(1., 1., 1.)], // +x
            [c(-1., -1., -1.), c(-1., -1., 1.), c(-1., 1., 1.), c(-1., 1., -1.)], // -x
        ];
        for f in faces {
            self.quad_world_uv(f, color, uv_scale);
        }
    }

    /// Extrudes a 2D profile (in the plane spanned by `out` and +Y) along the
    /// segment `a -> b`. Used for cushions.
    pub fn extrude(&mut self, a: Vec3, b: Vec3, out: Vec3, profile: &[Vec2], color: Color) {
        // The faces' normals come from the winding, which depends on the
        // direction of `a -> b` relative to `out`: run it the way that makes
        // them face outwards (otherwise half the cushions are lit from
        // behind and look dark).
        let (a, b) = if (b - a).dot(out.cross(Vec3::Y)) < 0.0 { (b, a) } else { (a, b) };
        let n = profile.len();
        let point = |base: Vec3, p: Vec2| base + out * p.x + Vec3::Y * p.y;
        let len = (b - a).length();
        for k in 0..n {
            let p0 = profile[k];
            let p1 = profile[(k + 1) % n];
            let quad = [point(a, p0), point(b, p0), point(b, p1), point(a, p1)];
            let v0 = k as f32 * 0.05;
            self.quad(
                quad,
                [
                    vec2(0.0, v0),
                    vec2(len * 4.0, v0),
                    vec2(len * 4.0, v0 + 0.05),
                    vec2(0.0, v0 + 0.05),
                ],
                color,
            );
        }
        // End caps (fan; profiles are convex).
        for (base, flip) in [(a, true), (b, false)] {
            let dir = (b - a).normalize_or_zero() * if flip { -1.0 } else { 1.0 };
            let first = self.push(point(base, profile[0]), Vec2::ZERO, dir, color);
            let mut prev = self.push(point(base, profile[1]), Vec2::ZERO, dir, color);
            for p in &profile[2..] {
                let cur = self.push(point(base, *p), Vec2::ZERO, dir, color);
                self.indices.extend_from_slice(&[first, prev, cur]);
                prev = cur;
            }
        }
    }

    /// A turned shape (a lathe): `profile` is (height, radius) pairs from the
    /// bottom up; normals follow the curve, so it shades smoothly.
    pub fn lathe(&mut self, at: Vec3, profile: &[(f32, f32)], segments: u32, color: Color) {
        let n = profile.len();
        if n < 2 {
            return;
        }
        // The slope at each ring, averaged with its neighbours.
        let normal_2d = |i: usize| -> Vec2 {
            let a = profile[i.saturating_sub(1)];
            let b = profile[(i + 1).min(n - 1)];
            let (dy, dr) = (b.0 - a.0, b.1 - a.1);
            vec2(dy, -dr).normalize_or_zero()
        };
        let first = self.vertices.len();
        for k in 0..=segments {
            let a = k as f32 / segments as f32 * std::f32::consts::TAU;
            let (sin, cos) = a.sin_cos();
            for (i, &(y, r)) in profile.iter().enumerate() {
                let nn = normal_2d(i);
                let normal = vec3(cos * nn.x, nn.y, sin * nn.x).normalize_or_zero();
                let uv = vec2(k as f32 / segments as f32, y);
                self.push(at + vec3(cos * r, y, sin * r), uv, normal, color);
            }
        }
        let stride = n as u16;
        let base = first as u16;
        for k in 0..segments as u16 {
            for i in 0..(n as u16 - 1) {
                let a = base + k * stride + i;
                let b = base + (k + 1) * stride + i;
                self.indices.extend_from_slice(&[a, a + 1, b + 1, a, b + 1, b]);
            }
        }
        // Closed ends where the profile meets the axis are already closed;
        // a flat top cap where it doesn't.
        let (ty, tr) = profile[n - 1];
        if tr > 1e-4 {
            self.disc(at + Vec3::Y * ty, tr, segments, color);
        }
    }

    /// A flat rectangle split into `nu` × `nv` cells (so baked lighting can
    /// vary across it), with texture coordinates in metres × `uv_scale`.
    #[allow(clippy::too_many_arguments)]
    pub fn grid(
        &mut self,
        origin: Vec3,
        u: Vec3,
        v: Vec3,
        nu: u32,
        nv: u32,
        color: Color,
        uv_scale: f32,
    ) {
        let normal = u.cross(v).normalize_or_zero();
        let (lu, lv) = (u.length(), v.length());
        let first = self.vertices.len() as u16;
        for j in 0..=nv {
            for i in 0..=nu {
                let (fu, fv) = (i as f32 / nu as f32, j as f32 / nv as f32);
                let p = origin + u * fu + v * fv;
                self.push(p, vec2(fu * lu, fv * lv) * uv_scale, normal, color);
            }
        }
        let row = (nu + 1) as u16;
        for j in 0..nv as u16 {
            for i in 0..nu as u16 {
                let a = first + j * row + i;
                self.indices.extend_from_slice(&[a, a + 1, a + row + 1, a, a + row + 1, a + row]);
            }
        }
    }

    /// Multiplies every vertex colour by `light(position, normal)` (baked
    /// lighting for the room, which is drawn unlit).
    pub fn bake(&mut self, light: impl Fn(Vec3, Vec3) -> Vec3) {
        for v in &mut self.vertices {
            let l = light(v.position, v.normal.truncate());
            for (k, c) in v.color.iter_mut().take(3).enumerate() {
                *c = (*c as f32 * l[k].clamp(0.0, 1.0)).round() as u8;
            }
        }
    }

    pub fn len(&self) -> usize {
        self.vertices.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }

    /// Vertical cylinder (open or capped on top).
    #[cfg(test)]
    pub fn cylinder(
        &mut self,
        center: Vec3,
        radius: f32,
        height: f32,
        segments: u32,
        color: Color,
        top: bool,
    ) {
        let ring = |y: f32, k: u32| {
            let a = k as f32 / segments as f32 * std::f32::consts::TAU;
            center + vec3(a.cos() * radius, y, a.sin() * radius)
        };
        for k in 0..segments {
            let quad = [ring(0.0, k + 1), ring(0.0, k), ring(height, k), ring(height, k + 1)];
            self.quad(quad, [Vec2::ZERO; 4], color);
        }
        if top {
            let c = self.push(center + Vec3::Y * height, Vec2::ZERO, Vec3::Y, color);
            for k in 0..segments {
                let a = self.push(ring(height, k), Vec2::ZERO, Vec3::Y, color);
                let b = self.push(ring(height, k + 1), Vec2::ZERO, Vec3::Y, color);
                self.indices.extend_from_slice(&[c, b, a]);
            }
        }
    }

    /// The inside of a vertical tube (a hole's wall): faces the axis.
    pub fn tube_inside(
        &mut self,
        center: Vec3,
        radius: f32,
        height: f32,
        segments: u32,
        color: Color,
    ) {
        let ring = |y: f32, k: u32| {
            let a = k as f32 / segments as f32 * std::f32::consts::TAU;
            center + vec3(a.cos() * radius, y, a.sin() * radius)
        };
        for k in 0..segments {
            let quad = [ring(0.0, k), ring(0.0, k + 1), ring(height, k + 1), ring(height, k)];
            self.quad(quad, [Vec2::ZERO; 4], color);
        }
    }

    /// Flat disc facing up.
    pub fn disc(&mut self, center: Vec3, radius: f32, segments: u32, color: Color) {
        let c = self.push(center, vec2(0.5, 0.5), Vec3::Y, color);
        for k in 0..segments {
            let a0 = k as f32 / segments as f32 * std::f32::consts::TAU;
            let a1 = (k + 1) as f32 / segments as f32 * std::f32::consts::TAU;
            let p0 = center + vec3(a0.cos(), 0.0, a0.sin()) * radius;
            let p1 = center + vec3(a1.cos(), 0.0, a1.sin()) * radius;
            let i0 =
                self.push(p0, vec2(0.5 + a0.cos() * 0.5, 0.5 + a0.sin() * 0.5), Vec3::Y, color);
            let i1 =
                self.push(p1, vec2(0.5 + a1.cos() * 0.5, 0.5 + a1.sin() * 0.5), Vec3::Y, color);
            self.indices.extend_from_slice(&[c, i1, i0]);
        }
    }

    /// Flat ring facing up.
    pub fn ring(&mut self, center: Vec3, inner: f32, outer: f32, segments: u32, color: Color) {
        for k in 0..segments {
            let a0 = k as f32 / segments as f32 * std::f32::consts::TAU;
            let a1 = (k + 1) as f32 / segments as f32 * std::f32::consts::TAU;
            let d0 = vec3(a0.cos(), 0.0, a0.sin());
            let d1 = vec3(a1.cos(), 0.0, a1.sin());
            // Counter-clockwise seen from above, so the face looks up.
            self.quad(
                [
                    center + d0 * inner,
                    center + d1 * inner,
                    center + d1 * outer,
                    center + d0 * outer,
                ],
                [Vec2::ZERO; 4],
                color,
            );
        }
    }

    /// Frustum along +Y from `y0` (radius `r0`) to `y1` (radius `r1`).
    /// A frustum along +Y like [`MeshBuilder::frustum`], placed at `at`.
    #[allow(clippy::too_many_arguments)]
    pub fn frustum_at(
        &mut self,
        at: Vec3,
        y0: f32,
        r0: f32,
        y1: f32,
        r1: f32,
        segments: u32,
        color: Color,
    ) {
        let first = self.vertices.len();
        self.frustum(y0, r0, y1, r1, segments, color);
        // Tilt the normals with the slope of the side, then move into place.
        let slope = (r0 - r1) / (y1 - y0).abs().max(1e-4);
        for v in &mut self.vertices[first..] {
            let n = vec3(v.normal.x, 0.0, v.normal.z);
            let n = (n + Vec3::Y * slope).normalize_or_zero();
            v.normal = vec4(n.x, n.y, n.z, 0.0);
            v.position += at;
        }
    }

    /// A frustum painted cell by cell: `paint(u, v)` gives the colour at
    /// `u` round the side (0..1) and `v` along it (0 at `y0`, 1 at `y1`).
    /// Each cell has its own vertices, so patterns keep sharp edges.
    #[allow(clippy::too_many_arguments)]
    pub fn painted_frustum(
        &mut self,
        y0: f32,
        r0: f32,
        y1: f32,
        r1: f32,
        segments: u32,
        rows: u32,
        paint: impl Fn(f32, f32) -> Color,
    ) {
        for row in 0..rows {
            let (v0, v1) = (row as f32 / rows as f32, (row + 1) as f32 / rows as f32);
            let (ya, yb) = (y0 + (y1 - y0) * v0, y0 + (y1 - y0) * v1);
            let (ra, rb) = (r0 + (r1 - r0) * v0, r0 + (r1 - r0) * v1);
            for k in 0..segments {
                let u = (k as f32 + 0.5) / segments as f32;
                let color = paint(u, (v0 + v1) / 2.0);
                let a0 = k as f32 / segments as f32 * std::f32::consts::TAU;
                let a1 = (k + 1) as f32 / segments as f32 * std::f32::consts::TAU;
                let d0 = vec3(a0.cos(), 0.0, a0.sin());
                let d1 = vec3(a1.cos(), 0.0, a1.sin());
                let p = [
                    d0 * ra + Vec3::Y * ya,
                    d1 * ra + Vec3::Y * ya,
                    d1 * rb + Vec3::Y * yb,
                    d0 * rb + Vec3::Y * yb,
                ];
                let i: Vec<u16> = [(0, d0), (1, d1), (2, d1), (3, d0)]
                    .iter()
                    .map(|&(k, n)| self.push(p[k], Vec2::ZERO, n, color))
                    .collect();
                self.indices.extend_from_slice(&[i[0], i[2], i[1], i[0], i[3], i[2]]);
            }
        }
    }

    pub fn frustum(&mut self, y0: f32, r0: f32, y1: f32, r1: f32, segments: u32, color: Color) {
        for k in 0..segments {
            let a0 = k as f32 / segments as f32 * std::f32::consts::TAU;
            let a1 = (k + 1) as f32 / segments as f32 * std::f32::consts::TAU;
            let d0 = vec3(a0.cos(), 0.0, a0.sin());
            let d1 = vec3(a1.cos(), 0.0, a1.sin());
            let p = [
                d0 * r0 + Vec3::Y * y0,
                d1 * r0 + Vec3::Y * y0,
                d1 * r1 + Vec3::Y * y1,
                d0 * r1 + Vec3::Y * y1,
            ];
            let i: Vec<u16> = [(0, d0), (1, d1), (2, d1), (3, d0)]
                .iter()
                .map(|&(k, n)| self.push(p[k], Vec2::ZERO, n, color))
                .collect();
            self.indices.extend_from_slice(&[i[0], i[2], i[1], i[0], i[3], i[2]]);
        }
    }

    /// Finishes the geometry. Big meshes are split into pieces that fit
    /// macroquad's default per-draw-call buffers (5000 indices, 10000
    /// vertices); keeping those buffers small matters on some drivers.
    pub fn build(self, texture: &Texture2D) -> Model {
        const MAX_INDICES: usize = 4800;
        const MAX_VERTICES: usize = 9000;
        // A strong clone keeps the GPU texture alive as long as the mesh.
        let texture = Some(texture.clone());
        if self.indices.len() <= MAX_INDICES && self.vertices.len() <= MAX_VERTICES {
            return Model {
                parts: vec![Mesh { vertices: self.vertices, indices: self.indices, texture }],
            };
        }
        let parts = split_mesh(&self.vertices, &self.indices, MAX_VERTICES, MAX_INDICES)
            .into_iter()
            .map(|(vertices, indices)| Mesh { vertices, indices, texture: texture.clone() })
            .collect();
        Model { parts }
    }
}

/// Splits a triangle list into pieces of at most `max_v` vertices and `max_i`
/// indices, each with its own compact vertex list.
fn split_mesh(
    vertices: &[Vertex],
    indices: &[u16],
    max_v: usize,
    max_i: usize,
) -> Vec<(Vec<Vertex>, Vec<u16>)> {
    let mut parts = Vec::new();
    let mut remap = vec![u16::MAX; vertices.len()];
    let mut part_v: Vec<Vertex> = Vec::new();
    let mut part_i: Vec<u16> = Vec::new();
    for tri in indices.as_chunks::<3>().0 {
        if part_i.len() + 3 > max_i || part_v.len() + 3 > max_v {
            parts.push((std::mem::take(&mut part_v), std::mem::take(&mut part_i)));
            remap.iter_mut().for_each(|r| *r = u16::MAX);
        }
        for &i in tri {
            let slot = &mut remap[i as usize];
            if *slot == u16::MAX {
                *slot = part_v.len() as u16;
                part_v.push(vertices[i as usize]);
            }
            part_i.push(*slot);
        }
    }
    if !part_i.is_empty() {
        parts.push((part_v, part_i));
    }
    parts
}

/// One or more meshes drawn together (see [`MeshBuilder::build`]).
pub struct Model {
    parts: Vec<Mesh>,
}

impl Model {
    /// Draws with another texture from now on (the geometry stays).
    pub fn set_texture(&mut self, texture: &Texture2D) {
        for m in &mut self.parts {
            m.texture = Some(texture.clone());
        }
    }

    pub fn draw(&self) {
        for m in &self.parts {
            draw_mesh(m);
        }
    }

    /// Draws transformed by `model`. The vertices are transformed on the CPU
    /// so the whole scene goes to the GPU in one batch (changing macroquad's
    /// model matrix forces a flush per object, and many flushes per frame
    /// make some drivers misbehave).
    pub fn draw_at(&self, model: Mat4) {
        let normal = Mat3::from_mat4(model);
        for m in &self.parts {
            let vertices = m
                .vertices
                .iter()
                .map(|v| {
                    let mut v = *v;
                    v.position = model.transform_point3(v.position);
                    let n = normal * v.normal.truncate();
                    v.normal = n.extend(0.0);
                    v
                })
                .collect();
            draw_mesh(&Mesh { vertices, indices: m.indices.clone(), texture: m.texture.clone() });
        }
    }
}

/// UV sphere whose texture mapping matches [`crate::textures::uv_to_dir`].
pub fn sphere_mesh(radius: f32, texture: &Texture2D, segments: u32) -> Model {
    sphere_mesh_tinted(radius, texture, segments, WHITE)
}

/// A sphere with a vertex colour (e.g. translucent).
pub fn sphere_mesh_tinted(radius: f32, texture: &Texture2D, segments: u32, tint: Color) -> Model {
    let seg = segments.max(8);
    let rings = (segments / 2).max(4);
    let mut b = MeshBuilder::default();
    for ring in 0..=rings {
        let v = ring as f32 / rings as f32;
        for s in 0..=seg {
            let u = s as f32 / seg as f32;
            let d = crate::textures::uv_to_dir(u, v);
            b.push(d * radius, vec2(u, v), d, tint);
        }
    }
    let stride = (seg + 1) as u16;
    for ring in 0..rings as u16 {
        for s in 0..seg as u16 {
            let a = ring * stride + s;
            let c = a + stride;
            b.indices.extend_from_slice(&[a, a + 1, c, a + 1, c + 1, c]);
        }
    }
    b.build(texture)
}

/// Perspective camera with a sensible depth range (macroquad's default far
/// plane is so distant that thin decals z-fight).
pub struct SceneCamera {
    pub eye: Vec3,
    pub target: Vec3,
    pub up: Vec3,
    pub fovy: f32,
}

impl Camera for SceneCamera {
    fn matrix(&self) -> Mat4 {
        let aspect = screen_width() / screen_height().max(1.0);
        Mat4::perspective_rh_gl(self.fovy, aspect, 0.03, 40.0)
            * Mat4::look_at_rh(self.eye, self.target, self.up)
    }
    fn depth_enabled(&self) -> bool {
        true
    }
    fn render_pass(&self) -> Option<macroquad::texture::RenderPass> {
        None
    }
    fn viewport(&self) -> Option<(i32, i32, i32, i32)> {
        None
    }
}

impl SceneCamera {
    /// Screen position of a world point (None if behind the camera).
    pub fn project(&self, p: Vec3) -> Option<Vec2> {
        let clip = self.matrix() * p.extend(1.0);
        if clip.w <= 1e-5 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(vec2((ndc.x + 1.0) * 0.5 * screen_width(), (1.0 - ndc.y) * 0.5 * screen_height()))
    }

    /// Ray through a screen point, for picking on the table plane.
    pub fn ray(&self, screen: Vec2) -> (Vec3, Vec3) {
        let ndc =
            vec2(screen.x / screen_width() * 2.0 - 1.0, 1.0 - screen.y / screen_height() * 2.0);
        let inv = self.matrix().inverse();
        let near = inv.project_point3(vec3(ndc.x, ndc.y, -1.0));
        let far = inv.project_point3(vec3(ndc.x, ndc.y, 1.0));
        (near, (far - near).normalize())
    }
}

#[cfg(test)]
mod split_tests {
    use super::*;

    #[test]
    fn splitting_keeps_every_triangle() {
        let mut b = MeshBuilder::default();
        for k in 0..2000 {
            let x = k as f32;
            b.quad(
                [
                    vec3(x, 0.0, 0.0),
                    vec3(x + 1.0, 0.0, 0.0),
                    vec3(x + 1.0, 1.0, 0.0),
                    vec3(x, 1.0, 0.0),
                ],
                [Vec2::ZERO; 4],
                WHITE,
            );
        }
        let parts = split_mesh(&b.vertices, &b.indices, 9000, 4800);
        assert!(parts.len() > 1);
        let total: usize = parts.iter().map(|(_, i)| i.len()).sum();
        assert_eq!(total, b.indices.len());
        for (v, i) in &parts {
            assert!(i.len() <= 4800 && v.len() <= 9000);
            // Same triangles, same positions.
            assert!(i.iter().all(|&k| (k as usize) < v.len()));
        }
        let first = &parts[0];
        assert_eq!(
            first.0[first.1[0] as usize].position,
            b.vertices[b.indices[0] as usize].position
        );
    }
}

/// Every face must point the way it is seen from: a face whose normal points
/// away from the viewer is lit from behind and looks dark (the bottom
/// cushion, the leather around the pockets…). These check the primitives.
#[cfg(test)]
mod orientation_tests {
    use super::*;

    /// The normal of each triangle from its winding, with its centroid.
    fn faces(b: &MeshBuilder) -> Vec<(Vec3, Vec3, Vec3)> {
        b.indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| {
                let [p0, p1, p2] = t.map(|i| b.vertices[i as usize].position);
                let n = b.vertices[t[0] as usize].normal.truncate();
                let geometric = (p1 - p0).cross(p2 - p0).normalize_or_zero();
                ((p0 + p1 + p2) / 3.0, n, geometric)
            })
            .collect()
    }

    #[test]
    fn ring_faces_up() {
        let mut b = MeshBuilder::default();
        b.ring(Vec3::ZERO, 0.05, 0.06, 24, WHITE);
        for (_, n, _) in faces(&b) {
            assert!(n.y > 0.99, "ring normal {n}");
        }
    }

    #[test]
    fn tube_inside_faces_the_axis_and_cylinder_faces_out() {
        let mut b = MeshBuilder::default();
        b.tube_inside(Vec3::ZERO, 0.06, 0.1, 24, WHITE);
        for (c, n, _) in faces(&b) {
            assert!(n.dot(vec3(c.x, 0.0, c.z)) < 0.0, "tube normal {n} at {c}");
        }
        let mut b = MeshBuilder::default();
        b.cylinder(Vec3::ZERO, 0.06, 0.1, 24, WHITE, false);
        for (c, n, _) in faces(&b) {
            assert!(n.dot(vec3(c.x, 0.0, c.z)) > 0.0, "cylinder normal {n} at {c}");
        }
    }

    #[test]
    fn extruded_cushions_face_out_whichever_way_they_run() {
        // A cushion profile: nose towards -out, top, back.
        let profile = [
            vec2(0.01, 0.0),
            vec2(0.0, 0.03),
            vec2(0.008, 0.04),
            vec2(0.05, 0.04),
            vec2(0.05, 0.0),
        ];
        let out = vec3(0.0, 0.0, 1.0);
        for (a, b) in [
            (vec3(-1.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0)),
            (vec3(1.0, 0.0, 0.0), vec3(-1.0, 0.0, 0.0)),
        ] {
            let mut m = MeshBuilder::default();
            m.extrude(a, b, out, &profile, WHITE);
            // The solid's middle: every face must point away from it.
            let inside = vec3(0.0, 0.02, 0.03);
            for (c, n, _) in faces(&m) {
                assert!(n.dot(c - inside) > 0.0, "cushion face {n} at {c}");
            }
        }
    }

    #[test]
    fn quads_get_the_normal_of_their_winding() {
        let mut b = MeshBuilder::default();
        b.quad(
            [vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(1.0, 0.0, -1.0), vec3(0.0, 0.0, -1.0)],
            [Vec2::ZERO; 4],
            WHITE,
        );
        for (_, n, g) in faces(&b) {
            assert!(n.dot(g) > 0.99 && n.y > 0.99);
        }
    }
}
