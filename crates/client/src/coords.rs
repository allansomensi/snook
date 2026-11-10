//! Conversions between simulation space (z-up, `f64`) and render space
//! (y-up, `f32`). The table surface is at [`TABLE_HEIGHT`] above the floor.

use ::glam::{DQuat, DVec2, DVec3};
use macroquad::math::{Quat, Vec3};

pub const TABLE_HEIGHT: f32 = 0.80;

pub fn to_world(p: DVec3) -> Vec3 {
    Vec3::new(p.x as f32, p.z as f32 + TABLE_HEIGHT, -p.y as f32)
}

/// Point on the table at `height` above the cloth.
pub fn surface(p: DVec2, height: f32) -> Vec3 {
    Vec3::new(p.x as f32, TABLE_HEIGHT + height, -p.y as f32)
}

pub fn to_table(p: Vec3) -> DVec2 {
    DVec2::new(p.x as f64, -p.z as f64)
}

pub fn dir_to_world(d: DVec3) -> Vec3 {
    Vec3::new(d.x as f32, d.z as f32, -d.y as f32)
}

/// Ball orientation -> rotation of the ball mesh (numbers on local ±Z; an
/// identity orientation shows the number facing up).
pub fn ball_rotation(q: DQuat) -> Quat {
    let q = Quat::from_xyzw(q.x as f32, q.y as f32, q.z as f32, q.w as f32);
    let c = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    c * q * c.inverse() * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_axes() {
        let p = DVec2::new(0.3, -0.2);
        assert!((to_table(surface(p, 0.0)) - p).length() < 1e-6);
        assert!((dir_to_world(DVec3::Z) - Vec3::Y).length() < 1e-6);
        let r = ball_rotation(DQuat::IDENTITY);
        assert!((r * Vec3::Z - Vec3::Y).length() < 1e-5);
    }
}
