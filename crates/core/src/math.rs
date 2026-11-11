//! Portable math. `f64::sin`, `cos`, `tan`… come from each platform's C
//! library, which may round the last bit differently (glibc, Windows, macOS).
//! A shot is simulated by the player and again by the server, so anything
//! that feeds the physics must give identical bits everywhere: these
//! functions use `libm`, a pure-Rust port of musl's math library.
//!
//! `sqrt` and the basic operations are exact IEEE operations on every
//! platform, so they need no replacement.

use glam::DVec2;

pub fn sin(x: f64) -> f64 {
    libm::sin(x)
}

pub fn cos(x: f64) -> f64 {
    libm::cos(x)
}

pub fn tan(x: f64) -> f64 {
    libm::tan(x)
}

pub fn atan2(y: f64, x: f64) -> f64 {
    libm::atan2(y, x)
}

/// The unit vector at `angle` radians (portable `DVec2::from_angle`).
pub fn from_angle(angle: f64) -> DVec2 {
    DVec2::new(cos(angle), sin(angle))
}

#[cfg(test)]
mod tests {
    #[test]
    fn matches_known_values() {
        assert_eq!(super::sin(0.0), 0.0);
        assert_eq!(super::cos(0.0), 1.0);
        assert!((super::atan2(1.0, 1.0) - std::f64::consts::FRAC_PI_4).abs() < 1e-15);
        assert!((super::tan(0.25) - 0.25f64.tan()).abs() < 1e-15);
    }
}
