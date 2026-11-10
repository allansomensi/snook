//! Computer opponent.
//!
//! The AI enumerates candidate shots with the "ghost ball" method, simulates each
//! one with the real physics engine and scores the result with the real rules. The
//! difficulty levels differ in how many options they consider, whether they plan
//! the cue ball's position for the next shot, and how accurately they execute.

use glam::DVec2;
use serde::{Deserialize, Serialize};

use crate::ball::BallId;
use crate::physics::Shot;
use crate::rng::Rng;
use crate::rules::EightBallGame;
use crate::table::PocketId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Difficulty {
    Easy,
    Medium,
    Hard,
}

impl Difficulty {
    pub const ALL: [Difficulty; 3] = [Difficulty::Easy, Difficulty::Medium, Difficulty::Hard];

    pub fn name(self) -> &'static str {
        match self {
            Difficulty::Easy => "Easy",
            Difficulty::Medium => "Medium",
            Difficulty::Hard => "Hard",
        }
    }
}

/// What the AI wants to do on its turn.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AiDecision {
    /// Where to put the cue ball first, when it is in hand.
    pub cue_ball: Option<DVec2>,
    pub shot: Shot,
    pub called_pocket: Option<PocketId>,
}

struct Profile {
    /// Execution error (standard deviation) in aim, degrees.
    aim_noise_deg: f64,
    /// Relative execution error in speed.
    speed_noise: f64,
    break_speed: f64,
    /// Speeds tried for every candidate in the first pass.
    coarse_speeds: &'static [f64],
    /// Speeds and tips tried for the most promising candidates.
    refine_speeds: &'static [f64],
    tips: &'static [(f64, f64)],
    /// Candidates kept for refinement.
    keep: usize,
    /// Aim points across the pocket mouth (fractions of the half-mouth).
    aim_spread: &'static [f64],
    position_weight: f64,
    plays_safe: bool,
    banks: bool,
    /// Simulated imperfect executions used to judge how safe a shot is.
    robustness_samples: usize,
    choose_among_top: usize,
}

fn profile(d: Difficulty) -> Profile {
    match d {
        Difficulty::Easy => Profile {
            aim_noise_deg: 0.75,
            speed_noise: 0.12,
            break_speed: 5.5,
            coarse_speeds: &[2.0],
            refine_speeds: &[1.6, 2.6],
            tips: &[(0.0, 0.0)],
            keep: 2,
            aim_spread: &[0.0],
            position_weight: 0.0,
            plays_safe: false,
            banks: false,
            robustness_samples: 0,
            choose_among_top: 3,
        },
        Difficulty::Medium => Profile {
            aim_noise_deg: 0.28,
            speed_noise: 0.06,
            break_speed: 6.3,
            coarse_speeds: &[1.6, 2.8],
            refine_speeds: &[1.2, 1.7, 2.4, 3.3, 4.4],
            tips: &[(0.0, 0.0), (0.0, 0.5), (0.0, -0.5)],
            keep: 4,
            aim_spread: &[-0.3, 0.0, 0.3],
            position_weight: 350.0,
            plays_safe: true,
            banks: false,
            robustness_samples: 2,
            choose_among_top: 1,
        },
        Difficulty::Hard => Profile {
            aim_noise_deg: 0.08,
            speed_noise: 0.025,
            break_speed: 7.2,
            coarse_speeds: &[1.4, 2.4, 3.8],
            refine_speeds: &[0.9, 1.25, 1.7, 2.2, 2.9, 3.8, 5.0],
            tips: &[
                (0.0, 0.0),
                (0.0, 0.35),
                (0.0, 0.7),
                (0.0, -0.4),
                (0.0, -0.75),
                (0.45, 0.2),
                (-0.45, 0.2),
                (0.45, -0.4),
                (-0.45, -0.4),
            ],
            keep: 6,
            aim_spread: &[-0.35, 0.0, 0.35],
            position_weight: 650.0,
            plays_safe: true,
            banks: true,
            robustness_samples: 4,
            choose_among_top: 1,
        },
    }
}

/// A shot to try: aim direction plus what it is meant to do.
#[derive(Clone, Copy)]
struct Candidate {
    yaw: f64,
    called: Option<PocketId>,
}

#[derive(Clone, Copy)]
struct Scored {
    score: f64,
    shot: Shot,
    called: Option<PocketId>,
}

/// Chooses the computer's next action. `seed` makes the decision reproducible.
pub fn decide(game: &EightBallGame, difficulty: Difficulty, seed: u64) -> AiDecision {
    let prof = profile(difficulty);
    let mut rng = Rng::new(seed);
    let mut g = game.clone();
    let me = g.current_player();

    let mut cue_ball = None;
    if g.ball_in_hand().is_some() {
        let p = choose_cue_ball_position(&g, &prof, &mut rng);
        if g.place_cue_ball(p) {
            cue_ball = Some(p);
        }
    }

    if g.is_break() {
        let cue = g.ball(BallId::CUE).xy();
        let d = g.table().foot_spot() - cue;
        // A touch of follow keeps the cue ball near the middle of the table.
        let shot = Shot::new(d.y.atan2(d.x), prof.break_speed).with_tip(DVec2::new(0.0, 0.15));
        return AiDecision { cue_ball, shot: perturb(shot, &prof, &mut rng), called_pocket: None };
    }

    // 1. Every geometrically sensible pot, tried quickly.
    let mut candidates = pocket_candidates(&g, &prof);
    if prof.banks {
        candidates.extend(bank_candidates(&g));
    }
    let mut coarse: Vec<(f64, Candidate)> = Vec::new();
    for c in &candidates {
        let mut best = f64::MIN;
        for &speed in prof.coarse_speeds {
            let shot = Shot::new(c.yaw, speed);
            best = best.max(evaluate(&g, &shot, c.called, me, &prof));
        }
        coarse.push((best, *c));
    }
    coarse.sort_by(|a, b| b.0.total_cmp(&a.0));
    coarse.truncate(prof.keep);

    // 2. The best ones with every speed and spin, for position.
    let mut options: Vec<Scored> = Vec::new();
    for &(_, c) in coarse.iter().filter(|(s, _)| *s > 0.0) {
        for &speed in prof.refine_speeds {
            for &(tx, ty) in prof.tips {
                let shot = Shot::new(c.yaw, speed).with_tip(DVec2::new(tx, ty));
                let score = evaluate(&g, &shot, c.called, me, &prof);
                options.push(Scored { score, shot, called: c.called });
            }
        }
    }
    options.sort_by(|a, b| b.score.total_cmp(&a.score));

    // 3. Prefer shots that still work when struck a little imperfectly.
    if prof.robustness_samples > 0 {
        robust_rescore(&g, &mut options, me, &prof, &mut rng);
    }

    // 4. When nothing is likely to drop, play safe (or at least legally).
    let best = options.first().map(|o| o.score).unwrap_or(f64::MIN);
    if best < 700.0 {
        let mut safeties = safety_options(&g, me, &prof);
        if prof.robustness_samples > 0 {
            robust_rescore(&g, &mut safeties, me, &prof, &mut rng);
        }
        options.extend(safeties);
        options.sort_by(|a, b| b.score.total_cmp(&a.score));
    }
    if options.is_empty() {
        let called = g.needs_called_pocket().then_some(PocketId::FootLeft);
        return AiDecision { cue_ball, shot: Shot::new(0.0, 2.0), called_pocket: called };
    }

    let pick = rng.below(prof.choose_among_top.min(options.len()));
    let o = options[pick];
    AiDecision { cue_ball, shot: perturb(o.shot, &prof, &mut rng), called_pocket: o.called }
}

/// Replaces the scores of the leading options with their average over
/// slightly perturbed executions.
fn robust_rescore(
    g: &EightBallGame,
    options: &mut [Scored],
    me: usize,
    prof: &Profile,
    rng: &mut Rng,
) {
    let n = options.len().min(5);
    let aim = (prof.aim_noise_deg * 2.0).max(0.15).to_radians();
    for o in options.iter_mut().take(n) {
        let mut total = o.score;
        for k in 0..prof.robustness_samples {
            let mut s = o.shot;
            let sign = if k % 2 == 0 { 1.0 } else { -1.0 };
            s.yaw += sign * aim * (0.6 + 0.4 * rng.next_f64());
            s.speed *= 1.0 + rng.gaussian() * prof.speed_noise * 1.5;
            total += evaluate(g, &s, o.called, me, prof);
        }
        o.score = total / (prof.robustness_samples + 1) as f64;
    }
    options[..n].sort_by(|a, b| b.score.total_cmp(&a.score));
}

fn perturb(mut shot: Shot, prof: &Profile, rng: &mut Rng) -> Shot {
    shot.yaw += rng.gaussian() * prof.aim_noise_deg.to_radians();
    shot.speed *= 1.0 + rng.gaussian() * prof.speed_noise;
    shot
}

fn evaluate(
    g: &EightBallGame,
    shot: &Shot,
    called: Option<PocketId>,
    me: usize,
    prof: &Profile,
) -> f64 {
    let mut trial = g.clone();
    let Ok(report) = trial.play_shot(shot, called) else {
        return f64::MIN / 2.0;
    };
    if let Some(over) = report.game_over {
        return if over.winner == me { 10_000.0 } else { -10_000.0 };
    }
    let mut score = 0.0;
    if report.is_foul() {
        // Ball in hand for the opponent is about as bad as it gets.
        score -= 800.0 + 400.0 * opponent_quality(&trial);
    }
    if report.continues {
        score += 1_000.0 + prof.position_weight * position_quality(&trial);
        score += 30.0 * report.pocketed.len() as f64;
    } else if !report.is_foul() && prof.plays_safe {
        // Leave the opponent nothing easy.
        score += 400.0 * (1.0 - opponent_quality(&trial));
    }
    score
}

/// How easy the best available shot is for the player to move (0 to ~1).
fn position_quality(g: &EightBallGame) -> f64 {
    let cue = g.ball(BallId::CUE).xy();
    let mut best: f64 = 0.0;
    let mut count = 0.0;
    for target in g.legal_targets() {
        let mut ball_best: f64 = 0.0;
        for pocket in PocketId::ALL {
            ball_best = ball_best.max(shot_ease(g, cue, target, pocket));
        }
        if ball_best > 0.05 {
            count += 1.0;
        }
        best = best.max(ball_best);
    }
    // Several options are better than one.
    best * (1.0 + 0.08 * (count - 1.0f64).max(0.0)).min(1.3)
}

/// How good the table is for the player who shoots next (after a miss).
fn opponent_quality(g: &EightBallGame) -> f64 {
    if g.ball_in_hand().is_some() {
        return 1.0;
    }
    position_quality(g)
}

fn shot_ease(g: &EightBallGame, cue: DVec2, target: BallId, pocket: PocketId) -> f64 {
    let r = g.table().spec.ball_radius;
    let t = g.ball(target).xy();
    let p = g.table().pocket(pocket);
    let aim = p.aim_point();
    let to_pocket = aim - t;
    let dir = to_pocket.normalize_or_zero();
    // Balls that approach a pocket at a steep angle to its axis rarely drop.
    let approach = dir.dot(p.outward);
    let limit = if pocket.is_side() { 0.45 } else { 0.25 };
    if approach < limit {
        return 0.0;
    }
    let ghost = t - dir * 2.0 * r;
    let to_ghost = ghost - cue;
    if to_ghost.length() < 1e-6 {
        return 0.0;
    }
    let cos_cut = to_ghost.normalize().dot(dir);
    if cos_cut < 0.26 {
        return 0.0; // cut thinner than ~75 degrees
    }
    if !path_clear(g, cue, ghost, &[BallId::CUE, target])
        || !path_clear(g, t, aim, &[BallId::CUE, target])
    {
        return 0.0;
    }
    let pocket_factor = 0.5 + 0.5 * approach;
    cos_cut * cos_cut * pocket_factor / (1.0 + 0.6 * (to_ghost.length() + to_pocket.length()))
}

fn path_clear(g: &EightBallGame, from: DVec2, to: DVec2, ignore: &[BallId]) -> bool {
    let clearance = 2.0 * g.table().spec.ball_radius - 1e-4;
    let seg = to - from;
    let len_sq = seg.length_squared();
    g.balls().iter().filter(|b| b.in_play() && !ignore.contains(&b.id)).all(|b| {
        let p = b.xy();
        let t = if len_sq > 0.0 { ((p - from).dot(seg) / len_sq).clamp(0.0, 1.0) } else { 0.0 };
        (from + seg * t).distance(p) >= clearance
    })
}

/// Aim directions that send a legal ball towards a pocket.
fn pocket_candidates(g: &EightBallGame, prof: &Profile) -> Vec<Candidate> {
    let cue = g.ball(BallId::CUE).xy();
    let r = g.table().spec.ball_radius;
    let mut out = Vec::new();
    for target in g.legal_targets() {
        for pocket in PocketId::ALL {
            if shot_ease(g, cue, target, pocket) <= 0.0 {
                continue;
            }
            let called = (target == BallId::EIGHT).then_some(pocket);
            let p = g.table().pocket(pocket);
            let t = g.ball(target).xy();
            for &k in prof.aim_spread {
                let aim_at = p.aim_point() + p.outward.perp() * (k * p.half_mouth);
                let dir = (aim_at - t).normalize();
                let aim = t - dir * 2.0 * r - cue;
                out.push(Candidate { yaw: aim.y.atan2(aim.x), called });
            }
        }
    }
    out
}

/// One-cushion bank shots: the object ball goes off a rail into a pocket.
fn bank_candidates(g: &EightBallGame) -> Vec<Candidate> {
    let cue = g.ball(BallId::CUE).xy();
    let spec = g.table().spec;
    let r = spec.ball_radius;
    let (hl, hw) = (spec.length / 2.0 - r, spec.width / 2.0 - r);
    let mut out = Vec::new();
    for target in g.legal_targets() {
        let t = g.ball(target).xy();
        for pocket in PocketId::ALL {
            let aim = g.table().pocket(pocket).aim_point();
            // Mirror the pocket in each rail the ball would bounce off.
            for mirror in [
                DVec2::new(aim.x, 2.0 * hw - aim.y),
                DVec2::new(aim.x, -2.0 * hw - aim.y),
                DVec2::new(2.0 * hl - aim.x, aim.y),
                DVec2::new(-2.0 * hl - aim.x, aim.y),
            ] {
                let dir = (mirror - t).normalize_or_zero();
                let ghost = t - dir * 2.0 * r;
                let to_ghost = ghost - cue;
                if to_ghost.length() < 1e-6 || to_ghost.normalize().dot(dir) < 0.5 {
                    continue;
                }
                if !path_clear(g, cue, ghost, &[BallId::CUE, target]) {
                    continue;
                }
                let called = (target == BallId::EIGHT).then_some(pocket);
                out.push(Candidate { yaw: to_ghost.y.atan2(to_ghost.x), called });
            }
        }
    }
    out
}

/// Shots meant to hit a legal ball and leave the opponent in trouble.
fn safety_options(g: &EightBallGame, me: usize, prof: &Profile) -> Vec<Scored> {
    let called = g.needs_called_pocket().then(|| nearest_pocket(g, BallId::EIGHT));
    let r = g.table().spec.ball_radius;
    let cue = g.ball(BallId::CUE).xy();
    let mut out = Vec::new();
    let offsets: &[f64] = if prof.plays_safe {
        &[-1.6, -1.2, -0.7, -0.3, 0.0, 0.3, 0.7, 1.2, 1.6]
    } else {
        &[-0.8, 0.0, 0.8]
    };
    let speeds: &[f64] = if prof.plays_safe { &[0.8, 1.3, 2.0, 3.0] } else { &[1.4, 2.4] };
    for target in g.legal_targets() {
        let t = g.ball(target).xy();
        let dir = (t - cue).normalize_or_zero();
        for &offset in offsets {
            let aim = t + dir.perp() * offset * r - cue;
            for &speed in speeds {
                let shot = Shot::new(aim.y.atan2(aim.x), speed);
                let score = evaluate(g, &shot, called, me, prof);
                out.push(Scored { score, shot, called });
            }
        }
    }
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out
}

fn nearest_pocket(g: &EightBallGame, ball: BallId) -> PocketId {
    let p = g.ball(ball).xy();
    PocketId::ALL
        .into_iter()
        .min_by(|a, b| {
            let da = g.table().pocket(*a).mouth_center.distance(p);
            let db = g.table().pocket(*b).mouth_center.distance(p);
            da.total_cmp(&db)
        })
        .expect("six pockets")
}

fn choose_cue_ball_position(g: &EightBallGame, prof: &Profile, rng: &mut Rng) -> DVec2 {
    if g.is_break() {
        let head = g.table().head_spot();
        let y = if prof.position_weight > 0.0 {
            0.12 * if rng.below(2) == 0 { 1.0 } else { -1.0 }
        } else {
            0.0
        };
        let p = DVec2::new(head.x, y);
        return if g.can_place_cue_ball(p) { p } else { head };
    }
    let r = g.table().spec.ball_radius;
    let mut best: Vec<(f64, DVec2)> = Vec::new();
    for target in g.legal_targets() {
        let t = g.ball(target).xy();
        for pocket in PocketId::ALL {
            let aim = g.table().pocket(pocket).aim_point();
            let dir = (aim - t).normalize_or_zero();
            let ghost = t - dir * 2.0 * r;
            for dist in [0.12, 0.2, 0.3, 0.45] {
                for angle in [0.0f64, 12.0, -12.0, 25.0, -25.0] {
                    let back = DVec2::from_angle(angle.to_radians()).rotate(dir);
                    let p = ghost - back * dist;
                    if g.can_place_cue_ball(p) {
                        let ease = shot_ease(g, p, target, pocket);
                        if ease > 0.0 {
                            best.push((ease, p));
                        }
                    }
                }
            }
        }
    }
    best.sort_by(|a, b| b.0.total_cmp(&a.0));
    if !best.is_empty() {
        return best[rng.below(prof.choose_among_top.min(best.len()))].1;
    }
    // Fallback: any legal spot near the head of the table.
    let head = g.table().head_spot();
    for _ in 0..500 {
        let p = head + DVec2::new(rng.next_f64() - 0.5, rng.next_f64() - 0.5) * 0.8;
        if g.can_place_cue_ball(p) {
            return p;
        }
    }
    head
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play_game(a: Difficulty, b: Difficulty, seed: u64) -> (Option<usize>, u32) {
        let mut g = EightBallGame::new(seed, 0);
        let levels = [a, b];
        for turn in 0..300u64 {
            if g.game_over().is_some() {
                break;
            }
            let d = decide(&g, levels[g.current_player()], seed * 1000 + turn);
            if let Some(p) = d.cue_ball {
                assert!(g.place_cue_ball(p));
            }
            g.play_shot(&d.shot, d.called_pocket).expect("AI must produce legal shots");
        }
        (g.game_over().map(|o| o.winner), g.shots_taken())
    }

    #[test]
    fn ai_finishes_games() {
        for seed in 1..=3 {
            let (winner, shots) = play_game(Difficulty::Medium, Difficulty::Medium, seed);
            assert!(winner.is_some(), "seed {seed}: no winner after {shots} shots");
        }
    }

    #[test]
    fn hard_beats_easy_most_of_the_time() {
        let mut hard_wins = 0;
        let games = 6;
        for seed in 0..games {
            // Alternate who breaks.
            let (winner, _) = if seed % 2 == 0 {
                play_game(Difficulty::Hard, Difficulty::Easy, seed)
            } else {
                let (w, s) = play_game(Difficulty::Easy, Difficulty::Hard, seed);
                (w.map(|w| 1 - w), s)
            };
            if winner == Some(0) {
                hard_wins += 1;
            }
        }
        assert!(hard_wins >= 4, "hard won only {hard_wins}/{games}");
    }
}
