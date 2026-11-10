//! Times AI decisions and plays AI-vs-AI games.
//! `cargo run --release -p snook_core --example ai_timing`
use std::time::Instant;

use snook_core::{Difficulty, EightBallGame, ai};

fn main() {
    for (a, b) in [
        (Difficulty::Hard, Difficulty::Medium),
        (Difficulty::Medium, Difficulty::Easy),
        (Difficulty::Hard, Difficulty::Hard),
    ] {
        let mut wins = [0, 0];
        let mut worst = 0.0f64;
        let mut total = 0.0;
        let mut decisions = 0;
        let mut potted_runs = Vec::new();
        for seed in 0..6u64 {
            let mut g = EightBallGame::new(seed + 11, (seed % 2) as usize);
            let levels = [a, b];
            let mut run = 0;
            for turn in 0..400u64 {
                if g.game_over().is_some() {
                    break;
                }
                let t0 = Instant::now();
                let d = ai::decide(&g, levels[g.current_player()], seed * 1000 + turn);
                let dt = t0.elapsed().as_secs_f64();
                worst = worst.max(dt);
                total += dt;
                decisions += 1;
                if let Some(p) = d.cue_ball {
                    g.place_cue_ball(p);
                }
                let r = g.play_shot(&d.shot, d.called_pocket).unwrap();
                if r.continues {
                    run += 1
                } else {
                    potted_runs.push(run);
                    run = 0
                }
            }
            if let Some(o) = g.game_over() {
                wins[o.winner] += 1;
            }
        }
        let avg_run = potted_runs.iter().sum::<i32>() as f64 / potted_runs.len().max(1) as f64;
        println!(
            "{:?} vs {:?}: wins {:?}, avg decision {:.0} ms, worst {:.0} ms, avg run {:.2}",
            a,
            b,
            wins,
            total / decisions as f64 * 1000.0,
            worst * 1000.0,
            avg_run
        );
    }
}
