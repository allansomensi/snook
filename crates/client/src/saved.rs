//! The game against the computer, kept on disk between turns so it can be
//! picked up again after the game is closed.

use serde::{Deserialize, Serialize};
use snookered_core::{Difficulty, EightBallGame};

use crate::game::{Controller, Match, ShotMark, ShotStats};

/// Bumped when the saved fields change (an older file is then ignored).
const VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedGame {
    version: u32,
    pub names: [String; 2],
    pub level: Difficulty,
    pub seed: u64,
    pub game: EightBallGame,
    pub stats: [ShotStats; 2],
    /// The shots so far (the end-of-game charts).
    #[serde(default)]
    pub timeline: Vec<ShotMark>,
    pub wins: [u32; 2],
    pub games_played: u32,
    /// Seconds per shot, if timed.
    pub shot_clock: Option<u32>,
    /// Unix seconds.
    pub saved_at: i64,
}

impl SavedGame {
    /// What to keep of `m`: only a game against the computer, between shots,
    /// still being played.
    pub fn of(m: &Match) -> Option<SavedGame> {
        let Controller::Computer(level) = m.controllers[1] else { return None };
        if m.controllers[0] != Controller::Human
            || m.is_online()
            || m.is_replay()
            || m.game.game_over().is_some()
            || !m.at_rest()
        {
            return None;
        }
        Some(SavedGame {
            version: VERSION,
            names: m.names.clone(),
            level,
            seed: m.seed(),
            game: m.game.clone(),
            stats: m.stats,
            timeline: m.timeline.clone(),
            wins: m.wins,
            games_played: m.games_played,
            shot_clock: m.clock.map(|(s, _)| s.round() as u32),
            saved_at: crate::game::unix_now(),
        })
    }

    /// Object balls still on the table (both groups and the 8).
    pub fn balls_left(&self) -> usize {
        self.game.balls().iter().filter(|b| b.id.is_object() && b.in_play()).count()
    }
}

fn path() -> Option<std::path::PathBuf> {
    crate::config::config_dir().map(|d| d.join("saved_game.json"))
}

pub fn load() -> Option<SavedGame> {
    let text = std::fs::read_to_string(path()?).ok()?;
    serde_json::from_str::<SavedGame>(&text).ok().filter(|s| s.version == VERSION)
}

pub fn save(s: &SavedGame) {
    let Some(p) = path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string(s) {
        // Written beside, then moved over: a crash never leaves half a file.
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }
}

pub fn delete() {
    if let Some(p) = path() {
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MatchMode, MatchSettings};

    #[test]
    fn a_game_against_the_computer_comes_back_as_it_was() {
        let settings = MatchSettings {
            mode: MatchMode::VsComputer(Difficulty::Medium),
            names: ["Ana".into(), "CPU".into()],
            guide: crate::config::AimGuide::Standard,
            shot_clock: Some(45),
        };
        let mut m = Match::new(&settings, 77);
        m.wins = [2, 1];
        m.timeline.push(ShotMark { shooter: 0, cue: (0.5, 0.1), made: 1, foul: false });
        let saved = SavedGame::of(&m).expect("a game at rest is saved");
        let text = serde_json::to_string(&saved).unwrap();
        let back: SavedGame = serde_json::from_str(&text).unwrap();
        assert_eq!(back.balls_left(), 15);
        let r = Match::resume(&back, settings.guide);
        assert_eq!(r.game, m.game);
        assert_eq!(r.timeline, m.timeline);
        assert_eq!((r.wins, r.names.clone()), ([2, 1], m.names.clone()));
        assert_eq!(r.controllers, m.controllers);
        assert_eq!(r.clock.map(|c| c.0), Some(45.0));
        // Two players on one computer: nothing to keep.
        let hot = MatchSettings { mode: MatchMode::HotSeat, ..settings };
        assert!(SavedGame::of(&Match::new(&hot, 1)).is_none());
    }
}
