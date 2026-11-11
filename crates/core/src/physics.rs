//! Deterministic fixed-step rigid-body simulation of pool balls.
//!
//! The model follows the classic treatment of billiard physics:
//! - balls **slide** on the cloth after being struck (kinetic friction acts at the
//!   contact point and converts slip into roll),
//! - then **roll** without slipping (small rolling resistance),
//! - vertical-axis **spin** (English) decays independently,
//! - ball/ball, ball/cushion and ball/slate contacts are resolved with impulses that
//!   include friction, so spin is transferred and "throw" emerges naturally,
//! - balls can leave the slate (jump shots) and fly under gravity.
//!
//! The simulation uses a fixed time step and only `f64` arithmetic, so the same
//! inputs produce the same outputs on every platform running the same binary. The
//! game server re-simulates every shot to keep online matches authoritative.

use glam::{DQuat, DVec2, DVec3};
use serde::{Deserialize, Serialize};

use crate::ball::{Ball, BallId, BallStatus, Motion};
use crate::table::{PocketId, Table};

/// Tunable physical constants.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PhysicsParams {
    pub gravity: f64,
    /// Kinetic friction between a sliding ball and the cloth.
    pub sliding_friction: f64,
    /// Rolling resistance coefficient.
    pub rolling_friction: f64,
    /// Coefficient controlling how fast vertical-axis spin decays.
    pub spin_friction: f64,
    pub ball_restitution: f64,
    /// Friction between two balls (produces throw and spin transfer).
    pub ball_friction: f64,
    pub cushion_restitution: f64,
    pub cushion_friction: f64,
    pub slate_restitution: f64,
    pub slate_friction: f64,
    /// Effective mass of the cue (kg).
    pub cue_mass: f64,
    /// Restitution of the cue tip / ball collision.
    pub cue_restitution: f64,
    /// Maximum cue speed the player can produce (m/s).
    pub max_cue_speed: f64,
    /// Maximum tip offset from the ball centre, as a fraction of the radius.
    /// Strikes further out than this would miscue.
    pub max_tip_offset: f64,
    /// Maximum cue elevation (radians).
    pub max_elevation: f64,
    /// Cue-ball deflection ("squirt") at full side spin (radians).
    pub squirt: f64,
    /// Integration time step (s).
    pub time_step: f64,
    /// Safety limit on the duration of a single shot (s).
    pub max_shot_duration: f64,
}

impl Default for PhysicsParams {
    fn default() -> Self {
        PhysicsParams {
            gravity: 9.81,
            sliding_friction: 0.2,
            rolling_friction: 0.01,
            // Side spin on a ball turning in place dies within a few seconds.
            spin_friction: 0.022,
            ball_restitution: 0.95,
            ball_friction: 0.06,
            cushion_restitution: 0.8,
            cushion_friction: 0.2,
            slate_restitution: 0.5,
            slate_friction: 0.2,
            cue_mass: 0.54,
            cue_restitution: 0.8,
            max_cue_speed: 7.0,
            max_tip_offset: 0.5,
            max_elevation: 75f64.to_radians(),
            squirt: 1.6f64.to_radians(),
            time_step: 1.0 / 1000.0,
            max_shot_duration: 40.0,
        }
    }
}

/// A cue stroke as chosen by the player.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Shot {
    /// Aim direction in the table plane, radians counter-clockwise from `+x`.
    pub yaw: f64,
    /// Cue elevation above horizontal (radians). Raising the butt lets the player
    /// jump balls or curve the cue ball (massé).
    pub elevation: f64,
    /// Where the tip strikes the cue ball, in the plane facing the cue:
    /// `x` is right (+) / left (-) English, `y` is top (+) / bottom (-) spin.
    /// Length is clamped to 1, where 1 means [`PhysicsParams::max_tip_offset`].
    pub tip: DVec2,
    /// Cue speed at impact (m/s).
    pub speed: f64,
}

impl Shot {
    /// A level, centre-ball shot.
    pub fn new(yaw: f64, speed: f64) -> Self {
        Shot { yaw, elevation: 0.0, tip: DVec2::ZERO, speed }
    }

    pub fn with_tip(mut self, tip: DVec2) -> Self {
        self.tip = tip;
        self
    }

    pub fn with_elevation(mut self, elevation: f64) -> Self {
        self.elevation = elevation;
        self
    }

    /// Returns a copy with every field clamped to what the cue can physically do.
    pub fn sanitized(&self, params: &PhysicsParams) -> Shot {
        let finite = |v: f64| if v.is_finite() { v } else { 0.0 };
        let tip = DVec2::new(finite(self.tip.x), finite(self.tip.y));
        Shot {
            yaw: finite(self.yaw).rem_euclid(std::f64::consts::TAU),
            elevation: finite(self.elevation).clamp(0.0, params.max_elevation),
            tip: if tip.length() > 1.0 { tip.normalize() } else { tip },
            speed: finite(self.speed).clamp(0.0, params.max_cue_speed),
        }
    }
}

/// Something that happened during a shot. Used by the rules engine and for sound.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Event {
    BallBall { time: f64, a: BallId, b: BallId, speed: f64 },
    Cushion { time: f64, ball: BallId, speed: f64 },
    Slate { time: f64, ball: BallId, speed: f64 },
    Pocketed { time: f64, ball: BallId, pocket: PocketId },
    OffTable { time: f64, ball: BallId },
}

impl Event {
    pub fn time(&self) -> f64 {
        match *self {
            Event::BallBall { time, .. }
            | Event::Cushion { time, .. }
            | Event::Slate { time, .. }
            | Event::Pocketed { time, .. }
            | Event::OffTable { time, .. } => time,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrikeError {
    BallNotInPlay,
    BallsStillMoving,
}

/// A running simulation.
#[derive(Debug, Clone)]
pub struct Simulation {
    pub table: Table,
    pub params: PhysicsParams,
    pub balls: Vec<Ball>,
    time: f64,
    accumulator: f64,
    events: Vec<Event>,
}

const SLIP_EPS: f64 = 1e-4;
const LINEAR_EPS: f64 = 5e-4;
const SPIN_EPS: f64 = 0.05;
const AIRBORNE_EPS: f64 = 1e-7;
/// Most sub-steps a time step is split into (a ball at the maximum speed
/// needs 3).
const MAX_SUBSTEPS: usize = 16;
/// Bounces slower than this are absorbed by the slate.
const LANDING_SPEED: f64 = 0.15;

impl Simulation {
    pub fn new(table: Table, params: PhysicsParams, balls: Vec<Ball>) -> Self {
        Simulation { table, params, balls, time: 0.0, accumulator: 0.0, events: Vec::new() }
    }

    pub fn time(&self) -> f64 {
        self.time
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }

    pub fn ball(&self, id: BallId) -> Option<&Ball> {
        self.balls.iter().find(|b| b.id == id)
    }

    fn ball_index(&self, id: BallId) -> Option<usize> {
        self.balls.iter().position(|b| b.id == id)
    }

    fn radius(&self) -> f64 {
        self.table.spec.ball_radius
    }

    fn inertia(&self) -> f64 {
        0.4 * self.table.spec.ball_mass * self.radius() * self.radius()
    }

    /// True when nothing on the table can change any more: every ball has
    /// stopped, apart from balls spinning in place (they no longer move).
    pub fn is_settled(&self) -> bool {
        !self.balls.iter().any(|b| b.is_moving() && b.motion != Motion::Spinning)
    }

    /// Stops everything (balls left spinning in place, or the safety limit).
    fn stop_all(&mut self) {
        for b in &mut self.balls {
            if b.is_moving() {
                b.velocity = DVec3::ZERO;
                b.angular_velocity = DVec3::ZERO;
                b.motion = Motion::Stationary;
            }
        }
    }

    /// Strikes `ball` with the cue.
    pub fn strike(&mut self, ball: BallId, shot: &Shot) -> Result<(), StrikeError> {
        if !self.is_settled() {
            return Err(StrikeError::BallsStillMoving);
        }
        let idx = self.ball_index(ball).ok_or(StrikeError::BallNotInPlay)?;
        if !self.balls[idx].in_play() {
            return Err(StrikeError::BallNotInPlay);
        }
        let (v, w) = cue_impulse(&self.params, &self.table, &shot.sanitized(&self.params));
        let b = &mut self.balls[idx];
        b.velocity = v;
        b.angular_velocity = w;
        b.motion = Motion::Sliding;
        Ok(())
    }

    /// Advances the simulation by `dt` seconds of wall-clock time using fixed
    /// sub-steps. Returns true while balls are still moving.
    pub fn advance(&mut self, dt: f64) -> bool {
        self.accumulator += dt;
        while self.accumulator >= self.params.time_step {
            self.accumulator -= self.params.time_step;
            if self.is_settled() || self.time > self.params.max_shot_duration {
                self.accumulator = 0.0;
                break;
            }
            self.step();
        }
        let moving = !self.is_settled() && self.time <= self.params.max_shot_duration;
        if !moving {
            self.stop_all();
        }
        moving
    }

    /// Runs until every ball has stopped (or the safety limit is hit).
    pub fn run_to_rest(&mut self) {
        while !self.is_settled() && self.time <= self.params.max_shot_duration {
            self.step();
        }
        self.stop_all();
    }

    /// One fixed time step. Fast balls are moved in sub-steps, so none goes
    /// further than an eighth of its radius between two collision checks: a
    /// thin cut at full power still registers the contact.
    pub fn step(&mut self) {
        let dt = self.params.time_step;
        let fastest = self
            .balls
            .iter()
            .filter(|b| b.is_moving())
            .map(|b| b.velocity.length())
            .fold(0.0, f64::max);
        let max_travel = self.radius() / 8.0;
        let n = ((fastest * dt / max_travel).ceil() as usize).clamp(1, MAX_SUBSTEPS);
        let sub = dt / n as f64;
        for _ in 0..n {
            self.substep(sub);
        }
    }

    fn substep(&mut self, dt: f64) {
        self.time += dt;
        for i in 0..self.balls.len() {
            if self.balls[i].is_moving() {
                self.apply_cloth_friction(i, dt);
                self.integrate(i, dt);
            }
        }
        for i in 0..self.balls.len() {
            if self.balls[i].is_moving() {
                self.resolve_slate(i);
            }
        }
        // A few passes make clustered collisions (the break) resolve cleanly.
        for _ in 0..3 {
            if !self.resolve_ball_collisions(dt) {
                break;
            }
        }
        for i in 0..self.balls.len() {
            if self.balls[i].is_moving() {
                self.resolve_cushions(i);
                self.resolve_pockets_and_bounds(i);
            }
        }
        for i in 0..self.balls.len() {
            if self.balls[i].in_play() {
                self.classify_motion(i);
            }
        }
    }

    fn apply_cloth_friction(&mut self, i: usize, dt: f64) {
        let p = self.params;
        let r = self.radius();
        let b = &mut self.balls[i];
        if b.motion == Motion::Airborne {
            b.velocity.z -= p.gravity * dt;
            return;
        }
        let v = b.velocity.truncate();
        let w = b.angular_velocity;
        let slip = contact_slip(v, w, r);
        let slip_len = slip.length();
        if slip_len > SLIP_EPS {
            let friction_dv = p.sliding_friction * p.gravity * dt;
            if slip_len <= 3.5 * friction_dv {
                // Slip vanishes within this step: switch to pure rolling.
                let v_roll = v - slip * (2.0 / 7.0);
                set_rolling(b, v_roll, r);
            } else {
                let dir = slip / slip_len;
                let v_new = v - dir * friction_dv;
                b.velocity.x = v_new.x;
                b.velocity.y = v_new.y;
                let dw = 2.5 * p.sliding_friction * p.gravity / r * dt;
                b.angular_velocity.x += -dir.y * dw;
                b.angular_velocity.y += dir.x * dw;
            }
        } else {
            let speed = v.length();
            let dv = p.rolling_friction * p.gravity * dt;
            let v_new = if speed <= dv { DVec2::ZERO } else { v * ((speed - dv) / speed) };
            set_rolling(b, v_new, r);
        }
        let spin_decel = 2.5 * p.spin_friction * p.gravity / r * dt;
        let wz = b.angular_velocity.z;
        b.angular_velocity.z =
            if wz.abs() <= spin_decel { 0.0 } else { wz - wz.signum() * spin_decel };
    }

    fn integrate(&mut self, i: usize, dt: f64) {
        let b = &mut self.balls[i];
        b.position += b.velocity * dt;
        let rot = b.angular_velocity * dt;
        if rot.length_squared() > 0.0 {
            // Portable `DQuat::from_scaled_axis` (same bits on every OS).
            let angle = rot.length();
            let axis = rot / angle;
            let half = angle / 2.0;
            let (sh, ch) = (crate::math::sin(half), crate::math::cos(half));
            let q = DQuat::from_xyzw(axis.x * sh, axis.y * sh, axis.z * sh, ch);
            b.orientation = (q * b.orientation).normalize();
        }
    }

    fn resolve_slate(&mut self, i: usize) {
        let r = self.radius();
        let m = self.table.spec.ball_mass;
        let inertia = self.inertia();
        let p = self.params;
        let time = self.time;
        let b = &mut self.balls[i];
        if b.position.z > r {
            return;
        }
        b.position.z = r;
        if b.velocity.z >= 0.0 {
            return;
        }
        let impact = -b.velocity.z;
        if impact < LANDING_SPEED {
            b.velocity.z = 0.0;
            return;
        }
        let jn = (1.0 + p.slate_restitution) * m * impact;
        b.velocity.z += jn / m;
        // Friction at the contact point converts slip into spin during the bounce.
        let slip = contact_slip(b.velocity.truncate(), b.angular_velocity, r);
        let slip_len = slip.length();
        if slip_len > 1e-9 {
            let jt_max = slip_len / (1.0 / m + r * r / inertia);
            let jt = (p.slate_friction * jn).min(jt_max);
            let impulse = (-slip / slip_len * jt).extend(0.0);
            b.velocity += impulse / m;
            let contact = DVec3::new(0.0, 0.0, -r);
            b.angular_velocity += contact.cross(impulse) / inertia;
        }
        if b.velocity.z < LANDING_SPEED {
            b.velocity.z = 0.0;
        }
        let id = b.id;
        self.events.push(Event::Slate { time, ball: id, speed: impact });
    }

    /// Returns true if any collision was resolved. `dt` is the sub-step just
    /// integrated (for contacts that happened during it).
    fn resolve_ball_collisions(&mut self, dt: f64) -> bool {
        let r = self.radius();
        let m = self.table.spec.ball_mass;
        let inertia = self.inertia();
        let p = self.params;
        let min_dist = 2.0 * r;
        let mut any = false;
        let n_balls = self.balls.len();
        for i in 0..n_balls {
            if !self.balls[i].in_play() {
                continue;
            }
            for j in (i + 1)..n_balls {
                if !self.balls[j].in_play() {
                    continue;
                }
                let (bi, bj) = (self.balls[i], self.balls[j]);
                if !bi.is_moving() && !bj.is_moving() {
                    continue;
                }
                let mut delta = bj.position - bi.position;
                let mut dist_sq = delta.length_squared();
                // Apart now, but they may have touched during the last
                // sub-step (a thin cut): find when, go back to that moment,
                // collide, and finish the sub-step with the new velocities.
                let mut rewind = 0.0;
                if dist_sq >= min_dist * min_dist {
                    match touch_time(delta, bj.velocity - bi.velocity, min_dist, dt) {
                        Some(back) => {
                            rewind = back;
                            self.balls[i].position -= bi.velocity * back;
                            self.balls[j].position -= bj.velocity * back;
                            delta = self.balls[j].position - self.balls[i].position;
                            dist_sq = delta.length_squared();
                        }
                        None => continue,
                    }
                }
                if dist_sq == 0.0 {
                    continue;
                }
                let (bi, bj) = (self.balls[i], self.balls[j]);
                let dist = dist_sq.sqrt();
                let n = delta / dist;
                let ri = n * r;
                let rj = -n * r;
                let vci = bi.velocity + bi.angular_velocity.cross(ri);
                let vcj = bj.velocity + bj.angular_velocity.cross(rj);
                let rel = vci - vcj;
                let vn = rel.dot(n);

                // Push the balls apart so they never remain interpenetrated.
                let overlap = (min_dist - dist).max(0.0);
                let mut correction = n * (overlap / 2.0 + 1e-9);
                if bi.motion != Motion::Airborne && bj.motion != Motion::Airborne {
                    correction.z = 0.0;
                }
                self.balls[i].position -= correction;
                self.balls[j].position += correction;

                if vn <= 0.0 {
                    // Not approaching after all: put them back where they were.
                    self.balls[i].position += bi.velocity * rewind;
                    self.balls[j].position += bj.velocity * rewind;
                    continue;
                }
                any = true;
                let jn = (1.0 + p.ball_restitution) * vn / (2.0 / m);
                let mut impulse_i = -n * jn;
                let tangential = rel - n * vn;
                let t_len = tangential.length();
                if t_len > 1e-9 {
                    let jt_max = t_len / (2.0 * (1.0 / m + r * r / inertia));
                    let jt = (p.ball_friction * jn).min(jt_max);
                    impulse_i -= tangential / t_len * jt;
                }
                let impulse_j = -impulse_i;
                {
                    let b = &mut self.balls[i];
                    b.velocity += impulse_i / m;
                    b.angular_velocity += ri.cross(impulse_i) / inertia;
                    b.motion = Motion::Sliding;
                }
                {
                    let b = &mut self.balls[j];
                    b.velocity += impulse_j / m;
                    b.angular_velocity += rj.cross(impulse_j) / inertia;
                    b.motion = Motion::Sliding;
                }
                for k in [i, j] {
                    let b = &mut self.balls[k];
                    if b.position.z <= r + AIRBORNE_EPS && b.velocity.z < 0.0 {
                        b.velocity.z = 0.0;
                    }
                    // The rest of the sub-step, after the contact.
                    b.position += b.velocity * rewind;
                }
                self.events.push(Event::BallBall {
                    time: self.time,
                    a: bi.id,
                    b: bj.id,
                    speed: vn,
                });
            }
        }
        any
    }

    fn resolve_cushions(&mut self, i: usize) {
        let r = self.radius();
        let m = self.table.spec.ball_mass;
        let inertia = self.inertia();
        let p = self.params;
        let h = self.table.spec.cushion_height;
        let time = self.time;
        let ball = self.balls[i];
        let dh = h - ball.position.z;
        if dh.abs() >= r {
            return; // Flying over (or below) the cushion nose.
        }
        let contact_radius = (r * r - dh * dh).sqrt();
        let mut b = ball;
        let mut hit_speed: Option<f64> = None;
        for seg in &self.table.cushions {
            let c = b.position.truncate();
            let q = seg.closest_point(c);
            let delta = c - q;
            let dist = delta.length();
            if dist >= contact_radius || dist == 0.0 {
                continue;
            }
            let inward = delta / dist;
            // Contact point relative to the centre (nose of the cushion).
            let contact = (-inward * contact_radius).extend(dh);
            let normal = -contact / r;
            let vc = b.velocity + b.angular_velocity.cross(contact);
            let vn = vc.dot(normal);
            let fixed = q + inward * contact_radius;
            b.position.x = fixed.x;
            b.position.y = fixed.y;
            if vn >= 0.0 {
                continue;
            }
            let jn = -(1.0 + p.cushion_restitution) * m * vn;
            let mut impulse = normal * jn;
            let tangential = vc - normal * vn;
            let t_len = tangential.length();
            if t_len > 1e-9 {
                let jt_max = t_len / (1.0 / m + r * r / inertia);
                let jt = (p.cushion_friction * jn).min(jt_max);
                impulse -= tangential / t_len * jt;
            }
            b.velocity += impulse / m;
            b.angular_velocity += contact.cross(impulse) / inertia;
            // The slate absorbs the downward part of the cushion reaction.
            if b.position.z <= r + AIRBORNE_EPS && b.velocity.z < 0.0 {
                b.velocity.z = 0.0;
            }
            b.motion = Motion::Sliding;
            hit_speed = Some(hit_speed.unwrap_or(0.0).max(-vn));
        }
        self.balls[i] = b;
        if let Some(speed) = hit_speed {
            self.events.push(Event::Cushion { time, ball: b.id, speed });
        }
    }

    fn resolve_pockets_and_bounds(&mut self, i: usize) {
        let r = self.radius();
        let spec = self.table.spec;
        let time = self.time;
        let b = self.balls[i];
        let xy = b.xy();
        let low_enough = b.position.z - r < spec.cushion_height;
        if low_enough && let Some(pocket) = self.table.pockets.iter().find(|p| p.captures(xy)) {
            let id = pocket.id;
            self.drop_ball(i, BallStatus::Pocketed(id));
            self.events.push(Event::Pocketed { time, ball: b.id, pocket: id });
            return;
        }
        let outside = xy.x.abs() > spec.length / 2.0 + r || xy.y.abs() > spec.width / 2.0 + r;
        let landed = b.position.z <= r + spec.cushion_height;
        let in_throat = low_enough && self.table.pockets.iter().any(|p| p.in_throat(xy));
        if in_throat && !self.table.is_off_table(xy) {
            // On a pocket shelf or between the facings: still in play.
            return;
        }
        if self.table.is_off_table(xy) || (outside && landed && b.velocity.z <= 0.0) {
            if low_enough {
                // Squeezed somewhere around a pocket: it ends up in the pocket.
                let nearest = self
                    .table
                    .pockets
                    .iter()
                    .min_by(|a, c| {
                        a.hole_center.distance(xy).total_cmp(&c.hole_center.distance(xy))
                    })
                    .map(|p| p.id)
                    .expect("table has pockets");
                let hole = self.table.pocket(nearest);
                if hole.hole_center.distance(xy) < hole.hole_radius + 2.0 * r {
                    self.drop_ball(i, BallStatus::Pocketed(nearest));
                    self.events.push(Event::Pocketed { time, ball: b.id, pocket: nearest });
                    return;
                }
            }
            self.drop_ball(i, BallStatus::OffTable);
            self.events.push(Event::OffTable { time, ball: b.id });
        }
    }

    fn drop_ball(&mut self, i: usize, status: BallStatus) {
        let b = &mut self.balls[i];
        b.status = status;
        b.velocity = DVec3::ZERO;
        b.angular_velocity = DVec3::ZERO;
        b.motion = Motion::Stationary;
    }

    fn classify_motion(&mut self, i: usize) {
        let r = self.radius();
        let b = &mut self.balls[i];
        if b.position.z > r + AIRBORNE_EPS || b.velocity.z > 0.0 {
            b.motion = Motion::Airborne;
            return;
        }
        b.position.z = r;
        b.velocity.z = 0.0;
        let v = b.velocity.truncate();
        let slip = contact_slip(v, b.angular_velocity, r);
        b.motion = if slip.length() > SLIP_EPS {
            Motion::Sliding
        } else if v.length() > LINEAR_EPS {
            Motion::Rolling
        } else if b.angular_velocity.z.abs() > SPIN_EPS {
            b.velocity = DVec3::ZERO;
            b.angular_velocity.x = 0.0;
            b.angular_velocity.y = 0.0;
            Motion::Spinning
        } else {
            b.velocity = DVec3::ZERO;
            b.angular_velocity = DVec3::ZERO;
            Motion::Stationary
        };
    }
}

/// Two balls `delta` apart now (`rel` = their relative velocity) were within
/// `min_dist` at some point of the last `dt` seconds: how long ago they
/// first touched.
fn touch_time(delta: DVec3, rel: DVec3, min_dist: f64, dt: f64) -> Option<f64> {
    let a = rel.length_squared();
    if a <= 0.0 {
        return None;
    }
    // Where they were `dt` ago, and |d0 + rel·t| = min_dist for t in [0, dt].
    let d0 = delta - rel * dt;
    let b = 2.0 * d0.dot(rel);
    let c = d0.length_squared() - min_dist * min_dist;
    if c <= 0.0 {
        return None; // Already touching then: handled as an overlap.
    }
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return None;
    }
    let t = (-b - disc.sqrt()) / (2.0 * a);
    (0.0..=dt).contains(&t).then_some(dt - t)
}

/// Horizontal velocity of the point of the ball touching the cloth.
fn contact_slip(v: DVec2, w: DVec3, r: f64) -> DVec2 {
    DVec2::new(v.x - r * w.y, v.y + r * w.x)
}

fn set_rolling(b: &mut Ball, v: DVec2, r: f64) {
    b.velocity.x = v.x;
    b.velocity.y = v.y;
    b.angular_velocity.x = -v.y / r;
    b.angular_velocity.y = v.x / r;
}

/// Linear and angular velocity given to the cue ball by a cue strike.
///
/// The cue is modelled as a rigid body of mass `cue_mass` hitting the ball at the
/// chosen tip offset along the (possibly elevated) cue axis.
pub fn cue_impulse(params: &PhysicsParams, table: &Table, shot: &Shot) -> (DVec3, DVec3) {
    let r = table.spec.ball_radius;
    let m = table.spec.ball_mass;
    let inertia = 0.4 * m * r * r;
    let yaw = shot.yaw + params.squirt * shot.tip.x;
    let forward = DVec3::new(crate::math::cos(yaw), crate::math::sin(yaw), 0.0);
    let up = DVec3::Z;
    let right = forward.cross(up);
    let (sin_e, cos_e) = (crate::math::sin(shot.elevation), crate::math::cos(shot.elevation));
    let axis = forward * cos_e - up * sin_e;
    let cue_up = up * cos_e + forward * sin_e;
    let a = shot.tip.x * params.max_tip_offset * r;
    let b = shot.tip.y * params.max_tip_offset * r;
    let c = (r * r - a * a - b * b).max(0.0).sqrt();
    let contact = right * a + cue_up * b - axis * c;
    let lever = contact.cross(axis);
    let k = 1.0 / params.cue_mass + 1.0 / m + lever.length_squared() / inertia;
    let impulse = (1.0 + params.cue_restitution) * shot.speed / k;
    (axis * (impulse / m), contact.cross(axis * impulse) / inertia)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::TableSpec;
    use std::f64::consts::PI;

    fn sim_with(balls: &[(u8, DVec2)]) -> Simulation {
        let table = Table::default();
        let r = table.spec.ball_radius;
        let balls = balls.iter().map(|&(n, p)| Ball::at_rest(BallId(n), p, r)).collect();
        Simulation::new(table, PhysicsParams::default(), balls)
    }

    #[test]
    fn centre_ball_shot_rolls_at_five_sevenths() {
        let mut sim = sim_with(&[(0, DVec2::new(-0.6, 0.0))]);
        sim.strike(BallId::CUE, &Shot::new(0.0, 2.0)).unwrap();
        let v0 = sim.balls[0].velocity.x;
        while sim.balls[0].motion == Motion::Sliding {
            sim.step();
        }
        let v1 = sim.balls[0].velocity.x;
        assert!((v1 / v0 - 5.0 / 7.0).abs() < 0.01, "ratio {}", v1 / v0);
        assert_eq!(sim.balls[0].motion, Motion::Rolling);
    }

    #[test]
    fn stun_shot_stops_cue_ball_dead() {
        // A firm centre-ball hit from close range arrives sliding with no spin,
        // so the cue ball stops near the contact point.
        let mut sim = sim_with(&[(0, DVec2::new(-0.3, 0.0)), (1, DVec2::new(-0.15, 0.0))]);
        sim.strike(BallId::CUE, &Shot::new(0.0, 3.0)).unwrap();
        let contact_x = -0.15 - 2.0 * 0.028575;
        let mut first_contact = None;
        while !sim.is_settled() && sim.time() < 1.0 {
            sim.step();
            if first_contact.is_none() && !sim.events().is_empty() {
                first_contact = Some(sim.time());
            }
            if first_contact.is_some_and(|t| sim.time() > t + 0.5) {
                break;
            }
        }
        let cue = sim.ball(BallId::CUE).unwrap();
        assert!((cue.position.x - contact_x).abs() < 0.08, "cue at {}", cue.position.x);
    }

    #[test]
    fn draw_shot_comes_back() {
        let mut sim = sim_with(&[(0, DVec2::new(-0.4, 0.0)), (1, DVec2::new(0.0, 0.0))]);
        sim.strike(BallId::CUE, &Shot::new(0.0, 3.5).with_tip(DVec2::new(0.0, -1.0))).unwrap();
        sim.run_to_rest();
        let cue = sim.ball(BallId::CUE).unwrap();
        assert!(cue.position.x < -0.5, "cue should draw back, ended at {}", cue.position.x);
    }

    #[test]
    fn follow_shot_goes_forward() {
        let mut sim = sim_with(&[(0, DVec2::new(-0.4, 0.0)), (1, DVec2::new(0.0, 0.0))]);
        sim.strike(BallId::CUE, &Shot::new(0.0, 2.0).with_tip(DVec2::new(0.0, 1.0))).unwrap();
        let mut max_x = f64::MIN;
        while !sim.is_settled() {
            sim.step();
            max_x = max_x.max(sim.ball(BallId::CUE).unwrap().position.x);
        }
        assert!(max_x > 0.1, "cue should follow through, reached {max_x}");
    }

    #[test]
    fn straight_shot_pots_ball_in_corner() {
        let t = Table::default();
        let pocket = t.pocket(PocketId::FootLeft).aim_point();
        let obj = DVec2::new(0.4, 0.2);
        let dir = (pocket - obj).normalize();
        let cue = obj - dir * 0.5;
        let mut sim = sim_with(&[(0, cue), (3, obj)]);
        sim.strike(BallId::CUE, &Shot::new(dir.y.atan2(dir.x), 2.0)).unwrap();
        sim.run_to_rest();
        assert_eq!(sim.ball(BallId(3)).unwrap().status, BallStatus::Pocketed(PocketId::FootLeft));
        assert!(sim.ball(BallId::CUE).unwrap().in_play());
    }

    #[test]
    fn ball_bounces_off_cushion_and_loses_energy() {
        let mut sim = sim_with(&[(0, DVec2::new(0.4, 0.0))]);
        sim.strike(BallId::CUE, &Shot::new(PI / 2.0, 2.0)).unwrap();
        sim.run_to_rest();
        assert!(sim.events().iter().any(|e| matches!(e, Event::Cushion { .. })));
        let b = sim.ball(BallId::CUE).unwrap();
        assert!(b.in_play());
        assert!((b.position.x - 0.4).abs() < 0.02, "should come straight back, x={}", b.position.x);
        assert!(b.position.z > 0.0);
    }

    #[test]
    fn side_spin_changes_rebound_angle() {
        let run = |side: f64| {
            let mut sim = sim_with(&[(0, DVec2::new(0.4, 0.0))]);
            sim.strike(BallId::CUE, &Shot::new(PI / 2.0, 1.5).with_tip(DVec2::new(side, 0.0)))
                .unwrap();
            sim.run_to_rest();
            sim.ball(BallId::CUE).unwrap().position.x
        };
        let left = run(-1.0);
        let right = run(1.0);
        assert!((left - right).abs() > 0.1, "english should change the rebound: {left} vs {right}");
    }

    #[test]
    fn elevated_shot_jumps() {
        let mut sim = sim_with(&[(0, DVec2::new(-0.5, 0.0))]);
        sim.strike(BallId::CUE, &Shot::new(0.0, 5.0).with_elevation(45f64.to_radians())).unwrap();
        let mut max_z = 0.0f64;
        while !sim.is_settled() {
            sim.step();
            max_z = max_z.max(sim.balls[0].position.z);
        }
        assert!(max_z > 0.028575 + 0.02, "ball should leave the slate, max z {max_z}");
    }

    #[test]
    fn jump_shot_clears_blocker() {
        let r = TableSpec::BAR_BOX_7FT.ball_radius;
        let mut sim = sim_with(&[(0, DVec2::new(-0.5, 0.0)), (5, DVec2::new(-0.36, 0.0))]);
        sim.strike(BallId::CUE, &Shot::new(0.0, 5.5).with_elevation(50f64.to_radians())).unwrap();
        sim.run_to_rest();
        let cue = sim.ball(BallId::CUE).unwrap();
        let blocker = sim.ball(BallId(5)).unwrap();
        assert!(cue.position.x > -0.36 + r, "cue ball should land beyond the blocker");
        assert!(blocker.status == BallStatus::InPlay);
    }

    /// The thinnest cuts at full power must count as contact (they used to
    /// slip between two collision checks).
    #[test]
    fn thin_cuts_at_full_power_register() {
        let r = TableSpec::BAR_BOX_7FT.ball_radius;
        for k in 0..40 {
            // Overlap of 0.02 mm, at 40 different phases along the path.
            let offset = 2.0 * r - 0.00002;
            let start = -0.5 - k as f64 * 0.00017;
            let mut sim = sim_with(&[(0, DVec2::new(start, 0.0)), (5, DVec2::new(0.0, offset))]);
            sim.strike(BallId::CUE, &Shot::new(0.0, 7.0)).unwrap();
            sim.run_to_rest();
            let hit = sim.events().iter().any(|e| matches!(e, Event::BallBall { .. }));
            assert!(hit, "graze missed at phase {k}");
        }
    }

    #[test]
    fn determinism() {
        let run = || {
            let mut sim = sim_with(&[
                (0, DVec2::new(-0.5, 0.05)),
                (1, DVec2::new(0.2, 0.0)),
                (2, DVec2::new(0.3, 0.1)),
            ]);
            sim.strike(BallId::CUE, &Shot::new(0.1, 6.0).with_tip(DVec2::new(0.3, -0.4))).unwrap();
            sim.run_to_rest();
            sim.balls.iter().map(|b| b.position).collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn shot_sanitizing() {
        let p = PhysicsParams::default();
        let s = Shot { yaw: -0.5, elevation: 3.0, tip: DVec2::new(3.0, 4.0), speed: 99.0 }
            .sanitized(&p);
        assert!(s.yaw >= 0.0 && s.yaw < std::f64::consts::TAU);
        assert_eq!(s.elevation, p.max_elevation);
        assert!((s.tip.length() - 1.0).abs() < 1e-12);
        assert_eq!(s.speed, p.max_cue_speed);
    }
}
