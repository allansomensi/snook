//! Player controls and the per-frame table overlay (cue, aim line, markers).
//!
//! | Action | Mouse | Keyboard |
//! |---|---|---|
//! | Aim | drag with the left button (Shift: fine) | A / D or ← / → |
//! | Spin | spin ball in the panel | ↑ / ↓, Q / E |
//! | Cue elevation | elevation slider | W / S |
//! | Shoot | power slider + Shoot | hold Space, release |
//! | Ball in hand | drag the cue ball | B toggles, Enter confirms |
//! | Call the 8-ball pocket | click a pocket | |
//! | Camera | drag outside the table (or right-drag) to turn it, wheel zoom | Tab 3D/top, C top view |

use ::glam::DVec2;
use macroquad::prelude::*;
use snook_core::{BallId, BallInHand, PocketId};

use crate::camera::{CameraMode, CameraRig};
use crate::config::AimGuide;
use crate::coords::{TABLE_HEIGHT, surface, to_table};
use crate::game::{Controller, Match, Phase};
use crate::gfx::SceneCamera;
use crate::scene::{Guide, Overlay, RAIL_HEIGHT, Scene};

/// Fraction of a full charge gained per second while Space is held.
const CHARGE_RATE: f32 = 0.7;

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum DragMode {
    #[default]
    None,
    /// Moving the cue ball in hand.
    Place,
    /// Swinging the cue stick itself around the cue ball.
    Cue,
    /// The cue follows the pointer.
    Point,
    /// Shift-drag: very fine rotation.
    Fine,
}

#[derive(Default)]
pub struct Controls {
    drag: f32,
    last_mouse: Vec2,
    mode: DragMode,
    yaw_before: f64,
    /// How long an aim key has been held (keys speed up).
    key_hold: f64,
    /// Fine-aim speed multiplier from the settings.
    pub sensitivity: f32,
    /// The pointer is over the cue (shown with a "move" cursor).
    over_cue: bool,
    /// A left-button drag that turns the camera (it started off the table,
    /// or when it isn't this player's turn).
    orbiting: bool,
}

impl Controls {
    /// True while the player is aiming with the mouse (dragging the cue or
    /// the aim point, or moving the cue ball): the HUD gets out of the way.
    pub fn aiming(&self) -> bool {
        !self.orbiting
            && self.mode != DragMode::None
            && is_mouse_button_down(MouseButton::Left)
            && self.drag > 3.0
    }

    fn set_over_cue(&mut self, over: bool) {
        if over != self.over_cue {
            self.over_cue = over;
            use macroquad::miniquad::{CursorIcon, window::set_mouse_cursor};
            set_mouse_cursor(if over { CursorIcon::Move } else { CursorIcon::Default });
        }
    }
}

/// True if the screen point `mouse` is on (or very near) the cue stick.
fn pointer_on_cue(m: &Match, cam: &SceneCamera, radius: f32, mouse: Vec2) -> bool {
    let cue = m.game.ball(BallId::CUE).xy();
    let dir = DVec2::from_angle(m.aim.yaw);
    let r = radius as f64;
    // Test the first ~70 cm of the stick in short pieces (the far end can be
    // behind the camera).
    let mut prev: Option<Vec2> = None;
    for k in 0..=14 {
        let along = r + 0.02 + k as f64 * 0.05;
        let lift = radius + along as f32 * 0.04;
        let Some(p) = cam.project(surface(cue - dir * along, lift)) else { break };
        if let Some(a) = prev {
            let ab = p - a;
            let t = ((mouse - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
            if mouse.distance(a + ab * t) < 16.0 {
                return true;
            }
        }
        prev = Some(p);
    }
    false
}

/// True for a point on the table, rails included.
fn on_the_table(p: DVec2) -> bool {
    p.x.abs() < 1.17 && p.y.abs() < 0.68
}

/// Point on the table plane (at ball-centre height) under the mouse.
pub fn pointer_on_table(cam: &SceneCamera, radius: f32) -> Option<DVec2> {
    let (origin, dir) = cam.ray(mouse_position().into());
    let plane_y = TABLE_HEIGHT + radius;
    if dir.y.abs() < 1e-5 {
        return None;
    }
    let t = (plane_y - origin.y) / dir.y;
    (t > 0.0).then(|| to_table(origin + dir * t))
}

impl Controls {
    /// Camera controls (allowed at any time during a match). Dragging with
    /// the left button outside the table (or anywhere while it's not this
    /// player's turn) turns the view, going 3D; the right button always does.
    pub fn camera(
        &mut self,
        rig: &mut CameraRig,
        cam: &SceneCamera,
        ui_hovered: bool,
        typing: bool,
        my_turn: bool,
    ) {
        let mouse: Vec2 = mouse_position().into();
        let delta = mouse - self.last_mouse;
        if is_mouse_button_pressed(MouseButton::Left) && !ui_hovered {
            let on_table = pointer_on_table(cam, 0.0).is_some_and(on_the_table);
            self.orbiting = !my_turn || !on_table;
        }
        if !is_mouse_button_down(MouseButton::Left) {
            self.orbiting = false;
        }
        if !typing {
            if is_key_pressed(KeyCode::Tab) || is_key_pressed(KeyCode::T) {
                if rig.mode == CameraMode::Top {
                    rig.enter_3d();
                } else {
                    rig.reset();
                }
            }
            if is_key_pressed(KeyCode::C) {
                rig.reset();
            }
        }
        let orbit_drag = self.orbiting && is_mouse_button_down(MouseButton::Left);
        if orbit_drag && ui_hovered {
            // Keep turning even when the pointer passes over the interface.
            self.orbit(rig, delta);
        }
        if !ui_hovered {
            if orbit_drag {
                self.orbit(rig, delta);
            } else if is_mouse_button_down(MouseButton::Right) {
                // Dragging with the right button turns the table: from the
                // overhead view, that means going 3D.
                if rig.mode == CameraMode::Top && delta.length() > 1.5 {
                    rig.enter_3d();
                }
                rig.orbit += delta.x * 0.005;
                rig.pitch = (rig.pitch + delta.y * 0.004).clamp(0.12, 1.4);
            }
            let (_, wheel) = mouse_wheel();
            if wheel != 0.0 {
                let step = if wheel > 0.0 { 0.88 } else { 1.0 / 0.88 };
                let toward = pointer_on_table(cam, 0.0).map(|p| surface(p, 0.0));
                rig.zoom(step, toward);
            }
        }
    }

    fn orbit(&self, rig: &mut CameraRig, delta: Vec2) {
        if rig.mode == CameraMode::Top && delta.length() > 1.5 {
            rig.enter_3d();
        }
        rig.orbit += delta.x * 0.005;
        rig.pitch = (rig.pitch + delta.y * 0.004).clamp(0.12, 1.4);
    }

    /// Shot controls for a human player.
    pub fn player(
        &mut self,
        m: &mut Match,
        scene: &Scene,
        cam: &SceneCamera,
        rig: &mut CameraRig,
        ui_hovered: bool,
        typing: bool,
    ) {
        let mouse: Vec2 = mouse_position().into();
        let delta = mouse - self.last_mouse;
        self.last_mouse = mouse;
        rig.hold = false;
        if !m.is_human_turn() || typing || self.orbiting {
            self.mode = DragMode::None;
            self.set_over_cue(false);
            return;
        }
        let dt = get_frame_time() as f64;
        let fine = is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift);
        let pointer_free = !ui_hovered;
        let radius = scene.ball_radius();
        let pointer = pointer_on_table(cam, radius);
        let cue = m.game.ball(BallId::CUE).xy();

        // Ball in hand: B toggles placing, Enter confirms.
        if is_key_pressed(KeyCode::B) && m.game.ball_in_hand().is_some() {
            let placing = m.is_placing_cue_ball();
            m.set_placing(!placing);
        }
        if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter) {
            m.set_placing(false);
        }

        // A "move" cursor over the cue says it can be grabbed.
        let hovering_cue = self.mode == DragMode::Cue
            || (self.mode == DragMode::None
                && !ui_hovered
                && !m.is_placing_cue_ball()
                && pointer_on_cue(m, cam, radius, mouse));
        self.set_over_cue(hovering_cue);

        // Start of a mouse drag: decide what it does.
        if is_mouse_button_pressed(MouseButton::Left) && pointer_free {
            self.drag = 0.0;
            self.yaw_before = m.aim.yaw;
            let on_cue_ball = pointer.is_some_and(|p| p.distance(cue) < radius as f64 * 1.8);
            self.mode =
                if m.game.ball_in_hand().is_some() && (m.is_placing_cue_ball() || on_cue_ball) {
                    m.set_placing(true);
                    DragMode::Place
                } else if !m.is_placing_cue_ball()
                    && (pointer_on_cue(m, cam, radius, mouse)
                        || pointer_on_cue(m, cam, radius, mouse - delta))
                {
                    DragMode::Cue
                } else if fine {
                    DragMode::Fine
                } else {
                    DragMode::Point
                };
        }
        let down = is_mouse_button_down(MouseButton::Left);
        if down {
            self.drag += delta.length();
        }
        match self.mode {
            DragMode::Place if down => {
                rig.hold = true;
                if let Some(p) = pointer {
                    place_near(m, p);
                }
            }
            DragMode::Point if down => {
                // The cue points at the mouse; the camera waits until release.
                rig.hold = true;
                if let Some(p) = pointer
                    && p.distance(cue) > radius as f64 * 1.5
                    && self.drag > 3.0
                {
                    let d = p - cue;
                    m.aim.yaw = d.y.atan2(d.x);
                }
            }
            DragMode::Cue if down => {
                // The cue swings around the cue ball, following the pointer:
                // the shot goes from the pointer through the ball.
                rig.hold = true;
                if let Some(p) = pointer
                    && p.distance(cue) > radius as f64 * 1.5
                {
                    let d = cue - p;
                    m.aim.yaw = d.y.atan2(d.x);
                }
            }
            DragMode::Fine if down => {
                m.aim.yaw -= delta.x as f64 * 0.0002 * self.sensitivity.max(0.1) as f64;
            }
            _ => {}
        }
        if is_mouse_button_released(MouseButton::Left) {
            match self.mode {
                DragMode::Place => m.set_placing(false),
                DragMode::Point | DragMode::Fine if self.drag < 4.0 && pointer_free => {
                    m.aim.yaw = self.yaw_before;
                    click(m, scene, pointer);
                }
                _ => {}
            }
            self.mode = DragMode::None;
        }
        if m.is_placing_cue_ball() {
            if !is_key_pressed(KeyCode::Space) {
                return;
            }
            m.set_placing(false);
        }

        // Keyboard aim: starts very fine and speeds up while held.
        let left = is_key_down(KeyCode::A) || is_key_down(KeyCode::Left);
        let right = is_key_down(KeyCode::D) || is_key_down(KeyCode::Right);
        if left || right {
            self.key_hold += dt;
            let rate =
                if fine { 0.012 } else { 0.03 + 0.9 * (self.key_hold - 0.25).clamp(0.0, 1.0) };
            m.aim.yaw += if left { rate * dt } else { -rate * dt };
        } else {
            self.key_hold = 0.0;
        }
        m.aim.yaw = m.aim.yaw.rem_euclid(std::f64::consts::TAU);

        // Spin and elevation.
        let tip_rate = 1.2 * dt;
        let mut tip = m.aim.tip;
        if is_key_down(KeyCode::Up) {
            tip.y += tip_rate;
        }
        if is_key_down(KeyCode::Down) {
            tip.y -= tip_rate;
        }
        if is_key_down(KeyCode::Q) {
            tip.x -= tip_rate;
        }
        if is_key_down(KeyCode::E) {
            tip.x += tip_rate;
        }
        m.aim.tip = if tip.length() > 1.0 { tip.normalize() } else { tip };
        let max_elevation = m.game.params().max_elevation;
        if is_key_down(KeyCode::W) {
            m.aim.elevation = (m.aim.elevation + 0.6 * dt).min(max_elevation);
        }
        if is_key_down(KeyCode::S) {
            m.aim.elevation = (m.aim.elevation - 0.6 * dt).max(0.0);
        }

        // Power: hold Space, release to shoot.
        if is_key_pressed(KeyCode::Space) {
            m.aim.charging = true;
            m.aim.power = 0.0;
        }
        if m.aim.charging {
            if is_key_pressed(KeyCode::Escape) {
                m.aim.charging = false;
                m.aim.power = 0.0;
            } else if is_key_down(KeyCode::Space) {
                m.aim.power = (m.aim.power + CHARGE_RATE * dt as f32).min(1.0);
            } else if is_key_released(KeyCode::Space) {
                if m.aim.power > 0.01 {
                    m.shoot();
                } else {
                    m.aim.charging = false;
                }
            }
        }
    }
}

/// Moves the cue ball in hand to `p`, or to the nearest free legal spot.
fn place_near(m: &mut Match, p: DVec2) {
    if m.game.can_place_cue_ball(p) {
        m.game.place_cue_ball(p);
        return;
    }
    let spec = m.game.table().spec;
    let r = spec.ball_radius;
    let mut q = p;
    // Stay on the cloth and, before the break or after a scratch, in the kitchen.
    q.x = q.x.clamp(-spec.length / 2.0 + r, spec.length / 2.0 - r);
    q.y = q.y.clamp(-spec.width / 2.0 + r, spec.width / 2.0 - r);
    if m.game.ball_in_hand() == Some(BallInHand::Kitchen) {
        q.x = q.x.min(m.game.table().head_string_x() - 1e-4);
    }
    for ring in 0..12 {
        let dist = ring as f64 * r * 0.35;
        let steps = if ring == 0 { 1 } else { 16 };
        let mut best: Option<DVec2> = None;
        for k in 0..steps {
            let a = k as f64 / steps as f64 * std::f64::consts::TAU;
            let c = q + DVec2::from_angle(a) * dist;
            if m.game.can_place_cue_ball(c) && best.is_none_or(|b| b.distance(p) > c.distance(p)) {
                best = Some(c);
            }
        }
        if let Some(c) = best {
            m.game.place_cue_ball(c);
            return;
        }
    }
}

/// A click on the table: calls a pocket for the 8-ball, or points the cue at
/// the clicked spot (a ball, or any point on the cloth).
fn click(m: &mut Match, scene: &Scene, at: Option<DVec2>) {
    let Some(p) = at else { return };
    if !matches!(m.phase, Phase::Turn { .. }) {
        return;
    }
    if m.game.needs_called_pocket() {
        let nearest = PocketId::ALL
            .into_iter()
            .map(|id| (id, scene.pocket_center(id).distance(p)))
            .filter(|(_, d)| *d < 0.12)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((id, _)) = nearest {
            m.called_pocket = Some(id);
            let name = m.names[m.game.current_player()].clone();
            m.notify(
                crate::i18n::tw(
                    "{name} calls the 8-ball in the {pocket} pocket.",
                    &[("name", &name), ("pocket", &crate::i18n::pocket(id))],
                ),
                false,
            );
            return;
        }
    }
    let r = scene.ball_radius() as f64;
    let cue = m.game.ball(BallId::CUE).xy();
    // Snap to the centre of a ball that was clicked (or nearly).
    let target = m
        .game
        .balls()
        .iter()
        .filter(|b| b.in_play() && b.id != BallId::CUE)
        .map(|b| (b.xy(), b.xy().distance(p)))
        .filter(|(_, d)| *d < r * 1.6)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(c, _)| c)
        .unwrap_or(p);
    let d = target - cue;
    if d.length() > r {
        m.aim.yaw = d.y.atan2(d.x).rem_euclid(std::f64::consts::TAU);
    }
}

/// The smallest cue elevation (radians) that keeps the cue above the rail and
/// the other balls behind the cue ball.
fn cue_clearance(m: &Match, scene: &Scene, yaw: f64, gap: f64) -> f64 {
    let spec = scene.table.spec;
    let r = spec.ball_radius;
    let cue = m.game.ball(BallId::CUE);
    if !cue.in_play() {
        return 0.0;
    }
    let p = cue.xy();
    let back = -DVec2::from_angle(yaw);
    let (hl, hw) = (spec.length / 2.0, spec.width / 2.0);
    // Cue radius grows from ~6 mm at the tip to ~14 mm at the butt.
    let cue_r = |x: f64| 0.0064 + (x - r - gap).clamp(0.0, 1.45) * 0.0055;
    // Height of the cue axis above the cloth at the cue ball is about one radius.
    let base = r + cue.position.z.max(r) - r;
    let mut slope: f64 = 0.0;
    let mut need = |x: f64, height: f64| {
        if x > 0.01 {
            slope = slope.max((height - base) / x);
        }
    };
    // The cushion nose line behind the cue ball (the rail is just behind it).
    let mut wall = f64::MAX;
    for (d, lim, q) in [(back.x, hl, p.x), (back.y, hw, p.y)] {
        if d > 1e-9 {
            wall = wall.min((lim - q) / d);
        } else if d < -1e-9 {
            wall = wall.min((-lim - q) / d);
        }
    }
    if wall < 2.0 {
        let rail = RAIL_HEIGHT as f64 + 0.004;
        need(wall.max(0.01), rail + cue_r(wall));
    }
    // Balls in the way of the cue.
    for b in m.game.balls().iter().filter(|b| b.in_play() && b.id != BallId::CUE) {
        let rel = b.xy() - p;
        let along = rel.dot(back);
        if along <= 0.0 {
            continue;
        }
        let perp = (rel - back * along).length();
        if perp < r + cue_r(along) + 0.003 {
            need((along - r).max(0.01), 2.0 * r + cue_r(along) + 0.003);
        }
    }
    slope.max(0.0).atan().min(1.2)
}

/// Distance along `dir` until a ball of radius `r` at `start` touches one at `other`.
fn ray_ball(start: DVec2, dir: DVec2, other: DVec2, r: f64) -> Option<f64> {
    let rel = other - start;
    let along = rel.dot(dir);
    if along <= 0.0 {
        return None;
    }
    let perp_sq = rel.length_squared() - along * along;
    let reach = 4.0 * r * r;
    (perp_sq <= reach).then(|| along - (reach - perp_sq).sqrt())
}

/// Builds the cue and the markings for this frame.
pub fn overlay(m: &Match, scene: &Scene, time: f32) -> Overlay {
    let mut out = Overlay::default();
    let table = &scene.table;
    let r = table.spec.ball_radius;
    let cue = m.game.ball(BallId::CUE).xy();
    let line_y = 0.0012;

    // Cue stick.
    let remote_waiting = m.current_controller() == Controller::Remote
        && m.online.as_ref().is_some_and(|o| !o.remote_aimed);
    let gap = match &m.phase {
        Phase::Turn { placing_cue_ball: false, .. } if !remote_waiting => {
            Some(0.006 + if m.aim.charging { 0.03 + m.aim.power as f64 * 0.25 } else { 0.03 })
        }
        Phase::Simulating(_) if m.shot_time().is_some_and(|t| t < 0.15) => Some(-0.012),
        _ => None,
    };
    if m.replay.is_some() {
        return out;
    }
    if let Some(gap) = gap {
        // Raise the cue just enough to clear the rail and other balls, like a
        // player would, so it never passes through anything.
        let elevation = m.aim.elevation.max(cue_clearance(m, scene, m.aim.yaw, gap));
        out.cue = Some(scene.cue_pose(
            m.game.params(),
            m.game.ball(BallId::CUE).position,
            m.aim.yaw,
            elevation,
            m.aim.tip,
            gap,
        ));
    }

    let Phase::Turn { placing_cue_ball, .. } = m.phase else { return out };

    if placing_cue_ball {
        let ok = m.game.can_place_cue_ball(cue);
        let color =
            if ok { Color::new(0.3, 1.0, 0.45, 0.9) } else { Color::new(1.0, 0.3, 0.25, 0.9) };
        out.guides.push(Guide::Ring {
            center: surface(cue, line_y),
            radius: r as f32 * 1.4,
            width: 0.004,
            color,
        });
        if m.game.ball_in_hand() == Some(BallInHand::Kitchen) {
            let x = table.head_string_x();
            let hw = table.spec.width / 2.0;
            out.guides.push(Guide::Line {
                from: surface(DVec2::new(x, -hw), line_y),
                to: surface(DVec2::new(x, hw), line_y),
                width: 0.003,
                color: Color::new(1.0, 1.0, 1.0, 0.35),
            });
        }
        return out;
    }

    // Pocket call for the 8-ball.
    if m.game.needs_called_pocket() {
        let pulse = 0.5 + 0.5 * (time * 4.0).sin();
        for id in PocketId::ALL {
            let called = m.called_pocket == Some(id);
            if called || m.called_pocket.is_none() {
                let alpha = if called { 1.0 } else { 0.3 + 0.5 * pulse };
                out.guides.push(Guide::Ring {
                    center: surface(scene.pocket_center(id), RAIL_HEIGHT + 0.004),
                    radius: scene.pocket_radius(id) + 0.018,
                    width: 0.008,
                    color: Color::new(1.0, 0.82, 0.25, alpha),
                });
            }
        }
    }

    if m.current_controller() != Controller::Human {
        return out;
    }

    // Aim line to the first contact.
    let dir = DVec2::from_angle(m.aim.yaw);
    let mut hit: Option<(f64, BallId)> = None;
    for b in m.game.balls().iter().filter(|b| b.in_play() && b.id != BallId::CUE) {
        if let Some(t) = ray_ball(cue, dir, b.xy(), r)
            && hit.is_none_or(|(best, _)| t < best)
        {
            hit = Some((t, b.id));
        }
    }
    let bound = DVec2::new(table.spec.length / 2.0 - r, table.spec.width / 2.0 - r);
    let mut wall = f64::MAX;
    for (d, lim, p) in [(dir.x, bound.x, cue.x), (dir.y, bound.y, cue.y)] {
        if d > 1e-9 {
            wall = wall.min((lim - p) / d);
        } else if d < -1e-9 {
            wall = wall.min((-lim - p) / d);
        }
    }
    let (t, target) = match hit {
        Some((t, id)) if t < wall => (t, Some(id)),
        _ => (wall.max(0.0), None),
    };
    let white = Color::new(1.0, 1.0, 1.0, 0.5);
    let start = cue + dir * r;
    if m.guide == AimGuide::Short {
        let len = t.min(0.35);
        out.guides.push(Guide::Line {
            from: surface(start, line_y),
            to: surface(cue + dir * len, line_y),
            width: 0.0025,
            color: white,
        });
        return out;
    }
    let ghost = cue + dir * t;
    // A fine, discreet line to a see-through "ghost" ball where the cue ball
    // will make contact.
    out.guides.push(Guide::Line {
        from: surface(start, line_y),
        to: surface(ghost - dir * r, line_y),
        width: 0.0012,
        color: Color::new(1.0, 1.0, 1.0, 0.42),
    });
    out.ghost = Some(surface(ghost, r as f32));
    // A faint ring under it: red when the first ball hit would be a foul.
    let ring = match target {
        Some(id) if !m.game.legal_targets().contains(&id) => Color::new(1.0, 0.35, 0.3, 0.75),
        _ => Color::new(1.0, 1.0, 1.0, 0.18),
    };
    out.guides.push(Guide::Ring {
        center: surface(ghost, line_y),
        radius: r as f32,
        width: 0.0012,
        color: ring,
    });
    if m.guide != AimGuide::Full {
        return out;
    }
    if let Some(id) = target {
        let obj = m.game.ball(id).xy();
        let n = (obj - ghost).normalize();
        let legal = m.game.legal_targets().contains(&id);
        let color =
            if legal { Color::new(0.45, 1.0, 0.55, 0.9) } else { Color::new(1.0, 0.4, 0.35, 0.9) };
        out.guides.push(Guide::Line {
            from: surface(obj + n * r, line_y),
            to: surface(obj + n * 0.3, line_y),
            width: 0.0025,
            color,
        });
        let tangent = dir - n * dir.dot(n);
        if tangent.length() > 1e-3 {
            let tangent = tangent.normalize();
            out.guides.push(Guide::Line {
                from: surface(ghost + tangent * r, line_y),
                to: surface(ghost + tangent * 0.15, line_y),
                width: 0.002,
                color: Color::new(1.0, 1.0, 1.0, 0.35),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_hits_ball_in_front() {
        let r = 0.028575;
        let t = ray_ball(DVec2::ZERO, DVec2::X, DVec2::new(0.5, 0.0), r).unwrap();
        assert!((t - (0.5 - 2.0 * r)).abs() < 1e-9);
        assert!(ray_ball(DVec2::ZERO, DVec2::X, DVec2::new(-0.5, 0.0), r).is_none());
        assert!(ray_ball(DVec2::ZERO, DVec2::X, DVec2::new(0.5, 0.2), r).is_none());
    }
}
