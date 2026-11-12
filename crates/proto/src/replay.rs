//! A whole game as data: the rack's seed, who broke and every stroke. The
//! physics is deterministic, so this rebuilds every shot exactly: replays,
//! reports with the game attached, and games that survive a server restart.

use glam::DVec2;
use serde::{Deserialize, Serialize};
use snook_core::{BallId, EightBallGame, PocketId, Shot, ShotReport};

/// One thing that changed the game.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "k", rename_all = "snake_case")]
pub enum Stroke {
    /// A shot by the player at the table; `cue_ball` is where the cue ball
    /// was placed when it was in hand.
    Shot {
        shot: Shot,
        #[serde(default)]
        called: Option<PocketId>,
        #[serde(default)]
        cue_ball: Option<DVec2>,
        /// The shot clock ran out and the server played it.
        #[serde(default)]
        timeout: bool,
    },
    /// The shot clock ran out and the turn passed (ball in hand).
    TimeFoul,
    Resign {
        seat: usize,
    },
    /// The player ran out of reconnection time.
    Abandon {
        seat: usize,
    },
}

/// A recorded game.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Replay {
    pub seed: u64,
    pub breaker: usize,
    pub strokes: Vec<Stroke>,
}

impl Replay {
    pub fn new(seed: u64, breaker: usize) -> Replay {
        Replay { seed, breaker, strokes: Vec::new() }
    }

    /// The game before any stroke.
    pub fn start(&self) -> EightBallGame {
        EightBallGame::new(self.seed, self.breaker)
    }

    /// The game after the first `upto` strokes (all of them if `None`), and
    /// the report of each shot on the way.
    pub fn play(&self, upto: Option<usize>) -> Result<(EightBallGame, Vec<ShotReport>), String> {
        let mut game = self.start();
        let mut reports = Vec::new();
        for s in self.strokes.iter().take(upto.unwrap_or(usize::MAX)) {
            if let Some(r) = apply(&mut game, s)? {
                reports.push(r);
            }
        }
        Ok((game, reports))
    }
}

/// Applies one stroke (shots are simulated to rest).
pub fn apply(game: &mut EightBallGame, stroke: &Stroke) -> Result<Option<ShotReport>, String> {
    match *stroke {
        Stroke::Shot { shot, called, cue_ball, .. } => {
            if let Some(p) = cue_ball
                && game.ball_in_hand().is_some()
                && game.ball(BallId::CUE).xy() != p
                && !game.place_cue_ball(p)
            {
                return Err("cue ball placement refused".into());
            }
            let mut pending = game.prepare_shot(&shot, called).map_err(|e| format!("{e:?}"))?;
            pending.simulation.run_to_rest();
            Ok(Some(game.finish_shot(pending)))
        }
        Stroke::TimeFoul => {
            game.time_foul();
            Ok(None)
        }
        Stroke::Resign { seat } => {
            game.resign(seat % 2);
            Ok(None)
        }
        Stroke::Abandon { seat } => {
            game.abandon(seat % 2);
            Ok(None)
        }
    }
}

/// Difficulty bands of a pot: the cut angle (5 bands of 15°, the last one
/// 60–90°) times the distance the balls travel (under 0.75 m, to 1.5 m, more).
pub const CUT_BANDS: usize = 5;
pub const DISTANCE_BANDS: usize = 3;
pub const BANDS: usize = CUT_BANDS * DISTANCE_BANDS;

pub fn band(cut_degrees: f64, distance: f64) -> usize {
    let cut = ((cut_degrees / 15.0).floor() as usize).min(CUT_BANDS - 1);
    let dist = if distance < 0.75 {
        0
    } else if distance < 1.5 {
        1
    } else {
        2
    };
    cut * DISTANCE_BANDS + dist
}

/// Bands that are hard even for strong players: thin cuts (45° or more)
/// over a long distance.
pub fn is_hard_band(b: usize) -> bool {
    b / DISTANCE_BANDS >= 3 && b % DISTANCE_BANDS == 2
}

/// How hard the shot at the first ball it hit was, as a band, and whether
/// that ball went in. `None` for the break, a miss, or a ball hit after
/// another (combinations are not measured).
pub fn shot_difficulty(
    before: &EightBallGame,
    shot: &Shot,
    report: &ShotReport,
) -> Option<(usize, bool)> {
    if report.was_break {
        return None;
    }
    let target = report.first_contact?;
    let r = before.table().spec.ball_radius;
    let cue = before.ball(BallId::CUE).xy();
    let object = before.ball(target).xy();
    let dir = DVec2::new(shot.yaw.cos(), shot.yaw.sin());
    // Where the cue ball touches the object ball: along the stroke, at two
    // radii from its centre (the "ghost ball").
    let to_obj = object - cue;
    let along = to_obj.dot(dir);
    let off2 = to_obj.length_squared() - along * along;
    let reach = 4.0 * r * r - off2;
    if along <= 0.0 || reach < 0.0 {
        return None;
    }
    let ghost = cue + dir * (along - reach.sqrt());
    let line = (object - ghost).normalize_or_zero();
    let cut = dir.dot(line).clamp(-1.0, 1.0).acos().to_degrees();
    // The pocket the object ball was sent towards: the one closest to its line.
    let pocket = before.table().pockets.iter().map(|p| p.mouth_center).max_by(|a, b| {
        let ka = (*a - object).normalize_or_zero().dot(line);
        let kb = (*b - object).normalize_or_zero().dot(line);
        ka.total_cmp(&kb)
    })?;
    let distance = (ghost - cue).length() + (pocket - object).length();
    let made = !report.is_foul() && report.pocketed.iter().any(|(b, _)| *b == target);
    Some((band(cut, distance), made))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_rebuilds_the_game() {
        let mut replay = Replay::new(42, 1);
        let mut game = replay.start();
        for k in 0..6 {
            let shot = Shot::new(0.1 + k as f64 * 0.7, 3.0);
            let stroke = Stroke::Shot { shot, called: None, cue_ball: None, timeout: false };
            if game.game_over().is_some() {
                break;
            }
            let called = game.needs_called_pocket().then(|| game.nearest_pocket(BallId::EIGHT));
            let stroke = match stroke {
                Stroke::Shot { shot, cue_ball, timeout, .. } => {
                    Stroke::Shot { shot, called, cue_ball, timeout }
                }
                s => s,
            };
            apply(&mut game, &stroke).unwrap();
            replay.strokes.push(stroke);
        }
        replay.strokes.push(Stroke::TimeFoul);
        game.time_foul();
        let json = serde_json::to_string(&replay).unwrap();
        let back: Replay = serde_json::from_str(&json).unwrap();
        let (rebuilt, _) = back.play(None).unwrap();
        assert_eq!(rebuilt, game);
    }

    #[test]
    fn bands() {
        assert_eq!(band(5.0, 0.3), 0);
        assert_eq!(band(70.0, 2.0), BANDS - 1);
        assert!(is_hard_band(band(70.0, 2.0)));
        assert!(is_hard_band(band(50.0, 1.6)));
        assert!(!is_hard_band(band(50.0, 1.0)));
        assert!(!is_hard_band(band(20.0, 2.0)));
    }
}
