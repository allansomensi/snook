//! Numbers about how a player played a game: potting success, balls made,
//! runs, fouls. Kept by the game for the end-of-game panel and by the server
//! for profiles.

use serde::{Deserialize, Serialize};

use crate::ShotReport;

/// One player's numbers for the current game (the end-of-game panel).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShotStats {
    pub shots: u32,
    /// Shots that legally made at least one ball.
    pub potting_shots: u32,
    /// Object balls legally made.
    pub balls: u32,
    pub fouls: u32,
    /// Most balls made in one visit to the table.
    pub best_run: u32,
    /// Balls made in the current visit.
    #[serde(skip)]
    pub run: u32,
    /// Shots taken with the cue ball in hand.
    pub in_hand: u32,
}

impl ShotStats {
    /// Updates the numbers with a shot by this player.
    pub fn add(&mut self, report: &ShotReport, in_hand: bool) {
        self.shots += 1;
        self.in_hand += in_hand as u32;
        if report.is_foul() {
            self.fouls += 1;
        }
        let made = if report.is_foul() {
            0
        } else {
            report.pocketed.iter().filter(|(b, _)| b.is_object()).count() as u32
        };
        if made > 0 {
            self.potting_shots += 1;
            self.balls += made;
            self.run += made;
            self.best_run = self.best_run.max(self.run);
        }
        if !report.continues {
            self.run = 0;
        }
    }

    /// Potting success, 0..=100.
    pub fn pot_rate(&self) -> u32 {
        (self.potting_shots * 100).checked_div(self.shots).unwrap_or(0)
    }

    /// Share of shots with ball in hand, 0..=100.
    pub fn in_hand_rate(&self) -> u32 {
        (self.in_hand * 100).checked_div(self.shots).unwrap_or(0)
    }
}
