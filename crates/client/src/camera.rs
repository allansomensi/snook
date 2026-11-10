//! Camera rig: a behind-the-cue view that keeps a comfortable distance from the
//! table, an overhead view, and a slow orbit for the menu.

use macroquad::prelude::*;

use crate::coords::{TABLE_HEIGHT, to_world};
use crate::game::{Match, Phase};
use crate::gfx::SceneCamera;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraMode {
    Aim,
    Top,
}

pub struct CameraRig {
    pub mode: CameraMode,
    /// Extra yaw added by orbiting with the right mouse button.
    pub orbit: f32,
    pub pitch: f32,
    pub distance: f32,
    frozen: Option<(Vec3, f32)>,
    /// Pull back to a wider, higher view while the balls roll.
    pub follow_shots: bool,
    /// Keep the view still (while the player drags the cue or the cue ball).
    pub hold: bool,
    /// Old behaviour: the view sits behind the cue and turns with the aim.
    /// Off by default: the camera only moves when the player moves it.
    pub follow_cue: bool,
    /// Where the fixed and overhead views look (on the cloth): zooming moves
    /// it towards the mouse pointer.
    pub focus: Vec3,
    eye: Vec3,
    target: Vec3,
    up: Vec3,
    initialised: bool,
    /// Current field of view (narrower overhead, so the table looks flat).
    fovy: f32,
}

pub const DEFAULT_PITCH: f32 = 0.42;
pub const DEFAULT_DISTANCE: f32 = 1.75;
/// The fixed view: from behind the head of the table, the whole table in sight.
pub const FIXED_PITCH: f32 = 0.70;
pub const FIXED_DISTANCE: f32 = 2.45;
/// Field of view of the overhead view: narrow, so the table looks flat.
const TOP_FOV: f32 = 0.63;

impl Default for CameraRig {
    fn default() -> Self {
        CameraRig {
            mode: CameraMode::Top,
            orbit: 0.0,
            pitch: DEFAULT_PITCH,
            distance: DEFAULT_DISTANCE,
            frozen: None,
            follow_shots: true,
            hold: false,
            follow_cue: false,
            focus: Vec3::ZERO,
            eye: vec3(-2.6, TABLE_HEIGHT + 1.2, 1.4),
            target: vec3(0.0, TABLE_HEIGHT, 0.0),
            up: Vec3::Y,
            initialised: false,
            fovy: 36f32.to_radians(),
        }
    }
}

/// Half extents of the table including rails (m).
const TABLE_HALF: Vec2 = Vec2::new(1.16, 0.665);
/// Never let the eye drop below this height while over the table.
const MIN_HEIGHT_OVER_TABLE: f32 = TABLE_HEIGHT + 0.32;

impl CameraRig {
    /// Zooms by `step` (< 1 is closer) towards `toward`, a point on the cloth
    /// under the mouse pointer, keeping that point roughly where it was.
    pub fn zoom(&mut self, step: f32, toward: Option<Vec3>) {
        let min = if self.follow_cue && self.mode == CameraMode::Aim { 0.9 } else { 0.55 };
        let old = self.distance;
        self.distance = (self.distance * step).clamp(min, 3.8);
        let k = 1.0 - self.distance / old;
        if self.follow_cue && self.mode == CameraMode::Aim {
            return;
        }
        match toward {
            Some(p) if k > 0.0 => {
                let center = vec3(0.0, TABLE_HEIGHT, 0.0);
                let target = vec3(p.x, TABLE_HEIGHT, p.z) - center;
                self.focus += (target - self.focus) * k;
            }
            _ => {
                // Zooming out drifts back towards the middle of the table.
                let back = (-k).max(0.0) * 1.5;
                self.focus *= (1.0 - back).max(0.0);
            }
        }
        self.focus.x = self.focus.x.clamp(-1.0, 1.0);
        self.focus.z = self.focus.z.clamp(-0.55, 0.55);
        self.focus.y = 0.0;
    }

    /// From the overhead view to the 3D view, seen from the same side, so a
    /// right-drag can turn the table straight away.
    pub fn enter_3d(&mut self) {
        if self.mode == CameraMode::Top {
            self.mode = CameraMode::Aim;
            if !self.follow_cue {
                self.orbit = -std::f32::consts::FRAC_PI_2;
                self.pitch = 1.15;
                self.distance = FIXED_DISTANCE;
            }
        }
    }

    /// Back to the default: the whole table seen from above.
    pub fn reset(&mut self) {
        self.mode = CameraMode::Top;
        self.orbit = 0.0;
        self.focus = Vec3::ZERO;
        if self.follow_cue {
            self.pitch = DEFAULT_PITCH;
            self.distance = DEFAULT_DISTANCE;
        } else {
            self.pitch = FIXED_PITCH;
            self.distance = FIXED_DISTANCE;
        }
    }

    /// Keeps the eye outside the table, above the floor and inside the room.
    fn constrain(eye: Vec3) -> Vec3 {
        let mut e = eye;
        e.x = e.x.clamp(-4.8, 4.8);
        e.z = e.z.clamp(-3.4, 3.4);
        e.y = e.y.clamp(0.9, 2.9);
        let over_table = e.x.abs() < TABLE_HALF.x + 0.25 && e.z.abs() < TABLE_HALF.y + 0.25;
        if over_table && e.y < MIN_HEIGHT_OVER_TABLE {
            e.y = MIN_HEIGHT_OVER_TABLE;
        }
        e
    }

    pub fn update(&mut self, dt: f32, m: Option<&Match>, elapsed: f32) -> SceneCamera {
        let center = vec3(0.0, TABLE_HEIGHT, 0.0);
        let mut rate = 7.0;
        let (eye, target, up) = match m {
            None => {
                let t = elapsed * 0.07;
                (center + vec3(t.cos() * 2.6, 0.75, t.sin() * 2.1), center, Vec3::Y)
            }
            Some(m) => {
                let moving = matches!(m.phase, Phase::Simulating(_) | Phase::Pause(_));
                let cue = to_world(m.game.ball(snook_core::BallId::CUE).position);
                let aim = -(m.aim.yaw as f32);
                let (focus, base) = if moving || self.hold {
                    *self.frozen.get_or_insert((cue, aim))
                } else {
                    self.frozen = None;
                    (cue, aim)
                };
                match self.mode {
                    CameraMode::Top => {
                        // Height that fits the whole table (with the HUD) in the window.
                        // The space between the top and bottom bars of the HUD.
                        let (sw, sh) = (screen_width(), screen_height().max(1.0));
                        let (top_px, bottom_px) = (54.0, 78.0);
                        let free = (sh - top_px - bottom_px).max(100.0) / sh;
                        let tan = (TOP_FOV / 2.0).tan();
                        let aspect = sw / sh;
                        let fit = (TABLE_HALF.y + 0.03) / (tan * free);
                        let fit = fit.max((TABLE_HALF.x + 0.05) / (tan * aspect));
                        let zoom = (self.distance / FIXED_DISTANCE).clamp(0.25, 1.3);
                        // Centre the table between the bars.
                        let per_px = 2.0 * fit * zoom * tan / sh;
                        let shift = (bottom_px - top_px) / 2.0 * per_px;
                        let look = center + self.focus + vec3(0.0, 0.0, shift);
                        (look + Vec3::Y * fit * zoom, look, Vec3::NEG_Z)
                    }
                    CameraMode::Aim if !self.follow_cue => {
                        // Fixed view around the middle of the table: it only
                        // moves when the player orbits or zooms.
                        let yaw = self.orbit;
                        let forward = vec3(yaw.cos(), 0.0, yaw.sin());
                        let look = center - forward * 0.12 + self.focus;
                        let eye = look - forward * self.distance * self.pitch.cos()
                            + Vec3::Y * self.distance * self.pitch.sin();
                        rate = 6.0;
                        (Self::constrain(eye), look, Vec3::Y)
                    }
                    CameraMode::Aim if moving && self.follow_shots && !self.hold => {
                        // A wider, higher view from the shooter's side to watch the balls.
                        rate = 2.2;
                        let yaw = base + self.orbit;
                        let forward = vec3(yaw.cos(), 0.0, yaw.sin());
                        let look = center.lerp(focus, 0.3);
                        let (pitch, dist) =
                            (0.62f32.max(self.pitch), 2.25 * self.distance / DEFAULT_DISTANCE);
                        let eye =
                            look - forward * dist * pitch.cos() + Vec3::Y * dist * pitch.sin();
                        (Self::constrain(eye), look, Vec3::Y)
                    }
                    CameraMode::Aim => {
                        rate = if self.frozen.is_none() && self.eye.y > TABLE_HEIGHT + 1.2 {
                            3.5
                        } else {
                            7.0
                        };
                        let yaw = base + self.orbit;
                        let forward = vec3(yaw.cos(), 0.0, yaw.sin());
                        let pitch = if moving {
                            self.pitch
                        } else {
                            self.pitch.max(m.aim.elevation as f32 + 0.2)
                        };
                        let eye = focus - forward * self.distance * pitch.cos()
                            + Vec3::Y * self.distance * pitch.sin();
                        // Look a little past the cue ball so the shot line is in view.
                        let look = focus + forward * 0.35 * self.distance;
                        (Self::constrain(eye), look, Vec3::Y)
                    }
                }
            }
        };
        if !self.initialised {
            self.eye = eye;
            self.target = target;
            self.up = up;
            self.initialised = true;
        }
        let k = 1.0 - (-dt * rate).exp();
        self.eye = self.eye.lerp(eye, k);
        self.target = self.target.lerp(target, k);
        self.up = self.up.lerp(up, k).normalize_or(Vec3::Y);
        // Avoid a degenerate look-at when the view direction and up align.
        let dir = (self.target - self.eye).normalize_or(Vec3::NEG_Z);
        let up = if dir.cross(self.up).length() < 1e-3 { Vec3::NEG_Z } else { self.up };
        let fov = if matches!(m.map(|_| self.mode), Some(CameraMode::Top)) {
            TOP_FOV
        } else {
            48f32.to_radians()
        };
        self.fovy += (fov - self.fovy) * k;
        SceneCamera { eye: self.eye, target: self.target, up, fovy: self.fovy }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eye_never_enters_the_table() {
        let e = CameraRig::constrain(vec3(0.5, TABLE_HEIGHT + 0.05, 0.2));
        assert!(e.y >= MIN_HEIGHT_OVER_TABLE);
        let outside = CameraRig::constrain(vec3(2.0, 1.0, 0.0));
        assert_eq!(outside.y, 1.0);
        assert!(CameraRig::constrain(vec3(0.0, -3.0, 0.0)).y >= 0.9);
    }
}
