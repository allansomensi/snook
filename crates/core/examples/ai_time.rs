use snook_core::{EightBallGame, Shot, ai, ai::Difficulty};
fn main() {
    for level in [Difficulty::Easy, Difficulty::Medium, Difficulty::Hard] {
        let mut g = EightBallGame::new(5, 1);
        let t = std::time::Instant::now();
        let d = ai::decide(&g, level, 1);
        println!("{level:?} break: {:?}", t.elapsed());
        g.play_shot(&d.shot, d.called_pocket).ok();
        let _ = Shot::new(0.0, 1.0);
        let t = std::time::Instant::now();
        let _ = ai::decide(&g, level, 2);
        println!("{level:?} after break: {:?}", t.elapsed());
    }
}
