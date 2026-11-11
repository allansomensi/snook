use snook_core::{EightBallGame, Shot};
use std::time::Instant;
fn main() {
    let g = EightBallGame::new(5, 0);
    let json = serde_json::to_string(&g).unwrap();
    println!("snapshot bytes: {}", json.len());
    let mut worst = 0.0f64;
    let mut total = 0.0;
    for k in 0..40 {
        let mut game = g.clone();
        let shot = Shot::new(0.01 * k as f64 - 0.2, 4.0 + (k % 5) as f64);
        let t = Instant::now();
        let _ = game.play_shot(&shot, None);
        let dt = t.elapsed().as_secs_f64();
        worst = worst.max(dt);
        total += dt;
    }
    println!("break shot sim: avg {:.1} ms, worst {:.1} ms", total / 40.0 * 1e3, worst * 1e3);
}
