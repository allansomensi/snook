//! Prints how far off-centre a rolling ball may be and still drop, per pocket,
//! approach angle and speed.
//! `cargo run --release -p snook_core --example pocket_tolerance`
use glam::{DVec2, DVec3};
use snook_core::physics::Simulation;
use snook_core::{Ball, BallId, BallStatus, Motion, PhysicsParams, PocketId, Table, TableSpec};

fn window(t: &Table, pid: PocketId, angle: f64, speed: f64) -> (usize, f64, f64) {
    let r = t.spec.ball_radius;
    let p = *t.pocket(pid);
    let dir = DVec2::from_angle(angle.to_radians()).rotate(p.outward);
    let mut ok = Vec::new();
    for k in -80..=80 {
        let off = k as f64 * 0.001;
        let start = p.mouth_center - dir * 0.45 + dir.perp() * off;
        if !t.is_inside_playing_area(start) {
            continue;
        }
        let mut b = Ball::at_rest(BallId(1), start, r);
        let v = dir * speed;
        b.velocity = DVec3::new(v.x, v.y, 0.0);
        b.angular_velocity = DVec3::new(-v.y / r, v.x / r, 0.0);
        b.motion = Motion::Rolling;
        let mut sim = Simulation::new(t.clone(), PhysicsParams::default(), vec![b]);
        sim.run_to_rest();
        // Only a clean drop in the aimed pocket counts (not a later bank).
        let quick = sim.events().iter().any(|e| {
            matches!(e, snook_core::Event::Pocketed { time, pocket, .. }
                if *pocket == pid && *time < 0.45 / speed + 0.4)
        });
        let _ = BallStatus::InPlay;
        if quick {
            ok.push(off);
        }
    }
    // The contiguous window around a centred hit.
    let mut lo = 0.0;
    let mut hi = 0.0;
    if ok.iter().any(|o| o.abs() < 1e-9) {
        while ok.iter().any(|o| (o - (lo - 0.001)).abs() < 1e-9) {
            lo -= 0.001;
        }
        while ok.iter().any(|o| (o - (hi + 0.001)).abs() < 1e-9) {
            hi += 0.001;
        }
    }
    (ok.len(), lo, hi)
}

fn main() {
    let turns: Vec<f64> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
    let turns = if turns.is_empty() { vec![-1.0] } else { turns };
    for turn in turns {
        let mut spec = TableSpec::default();
        if turn >= 0.0 {
            spec.corner_facing_turn = turn.to_radians();
        }
        let t = Table::new(spec);
        println!("corner facing turn {:.1}°", spec.corner_facing_turn.to_degrees());
        for (pid, label) in [(PocketId::FootLeft, "corner"), (PocketId::SideLeft, "side")] {
            for angle in [0.0f64, 25.0, 40.0] {
                for speed in [0.8, 2.0, 4.0] {
                    let (n, lo, hi) = window(&t, pid, angle, speed);
                    println!(
                        "  {label:6} {angle:3.0}° {speed:3.1} m/s: {n:3} drops, window {:+.3}..{:+.3} m",
                        lo, hi
                    );
                }
            }
        }
    }
}
