//! Stress test: plays many random shots and checks physical invariants.
//! `cargo run --release -p snook_core --example fuzz -- [shots]`
use snook_core::rng::Rng;
use snook_core::{BallId, EightBallGame, PocketId, Shot};

fn main() {
    let shots: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(3000);
    let mut rng = Rng::new(99);
    let mut g = EightBallGame::new(1, 0);
    let mut problems = 0;
    let mut longest: f64 = 0.0;
    let mut games = 0;
    let mut capped = 0;
    let t0 = std::time::Instant::now();
    for i in 0..shots {
        if g.game_over().is_some() {
            games += 1;
            g = EightBallGame::new(i as u64, i % 2);
        }
        if g.ball_in_hand().is_some() {
            for _ in 0..200 {
                let p = glam::DVec2::new(rng.next_f64() * 1.9 - 0.95, rng.next_f64() * 0.9 - 0.45);
                if g.place_cue_ball(p) {
                    break;
                }
            }
        }
        let shot = Shot {
            yaw: rng.next_f64() * std::f64::consts::TAU,
            elevation: if rng.below(4) == 0 { rng.next_f64() * 1.4 } else { 0.0 },
            tip: glam::DVec2::new(rng.next_f64() * 2.0 - 1.0, rng.next_f64() * 2.0 - 1.0),
            speed: rng.next_f64() * 8.0,
        };
        let called = g.needs_called_pocket().then(|| PocketId::ALL[rng.below(6)]);
        let trial = g.clone();
        let Ok(pending) = trial.prepare_shot(&shot, called) else { continue };
        let mut sim = pending.simulation.clone();
        // Find what keeps the table moving the longest.
        let mut last_mover = String::new();
        while !sim.is_settled() && sim.time() < trial.params().max_shot_duration {
            sim.step();
            if sim.time() > 8.0 && last_mover.is_empty() {
                last_mover = sim
                    .balls
                    .iter()
                    .filter(|b| b.is_moving())
                    .map(|b| {
                        format!(
                            "{:?} {:?} v={:.3} w={:.2}",
                            b.id,
                            b.motion,
                            b.velocity.length(),
                            b.angular_velocity.length()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
            }
        }
        if sim.time() > 12.0 {
            println!(
                "long shot {i} ({:.1} s) speed {:.1}: at 8 s: {last_mover}",
                sim.time(),
                shot.speed
            );
        }
        longest = longest.max(sim.time());
        if sim.time() >= trial.params().max_shot_duration {
            capped += 1;
        }
        let report = g.play_shot(&shot, called).expect("prepared above");
        let _ = report;
        let t = g.table();
        let r = t.spec.ball_radius;
        let live: Vec<_> = g.balls().iter().filter(|b| b.in_play()).collect();
        for b in &live {
            let p = b.position;
            if !p.is_finite() {
                problems += 1;
                println!("shot {i}: {:?} non-finite {:?}", b.id, p);
                continue;
            }
            // A resting ball may touch a cushion nose but not sink into it.
            let dh = t.spec.cushion_height - r;
            let contact = (r * r - dh * dh).sqrt();
            let nearest = t
                .cushions
                .iter()
                .map(|c| c.closest_point(b.xy()).distance(b.xy()))
                .fold(f64::MAX, f64::min);
            if nearest < contact - 5e-4 {
                problems += 1;
                println!(
                    "shot {i}: {:?} sunk {:.4} m into a cushion at {:?}",
                    b.id,
                    contact - nearest,
                    b.xy()
                );
            }
            let hl = t.spec.length / 2.0 + 0.02;
            let hw = t.spec.width / 2.0 + 0.02;
            if b.xy().x.abs() > hl || b.xy().y.abs() > hw {
                let in_throat = t.pockets.iter().any(|pk| pk.in_throat(b.xy()));
                if !in_throat {
                    problems += 1;
                    println!("shot {i}: {:?} outside the table at {:?}", b.id, b.xy());
                }
            }
            if (p.z - r).abs() > 1e-6 {
                problems += 1;
                println!("shot {i}: {:?} not resting on the slate (z={})", b.id, p.z);
            }
            if b.is_moving() {
                problems += 1;
                println!("shot {i}: {:?} still moving", b.id);
            }
        }
        for a in 0..live.len() {
            for c in a + 1..live.len() {
                let d = live[a].xy().distance(live[c].xy());
                if d < 2.0 * r - 1e-5 {
                    problems += 1;
                    println!(
                        "shot {i}: {:?} and {:?} overlap by {:.5} m",
                        live[a].id,
                        live[c].id,
                        2.0 * r - d
                    );
                }
            }
        }
        if g.ball(BallId::CUE).in_play() && g.ball_in_hand().is_some() && !g.is_break() {
            // fine: respotted cue ball
        }
    }
    println!(
        "{shots} shots, {games} games finished, {problems} problems, longest shot {longest:.1} s, {capped} hit the time cap, {:.1} ms/shot",
        t0.elapsed().as_secs_f64() * 1000.0 / shots as f64
    );
}
