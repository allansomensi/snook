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
    /// Balls legally made with spin or a raised cue (see [`ShotStats::add_spin`]).
    #[serde(default)]
    pub spin_pots: u32,
    /// Visits to the table with three or more balls made.
    #[serde(default)]
    pub runs: u32,
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
            if self.run < 3 && self.run + made >= 3 {
                self.runs += 1;
            }
            self.run += made;
            self.best_run = self.best_run.max(self.run);
        }
        if !report.continues {
            self.run = 0;
        }
    }

    /// Counts the balls `report` legally made if `shot` had spin: the tip
    /// clearly off centre, or the cue raised (a jump or a massé).
    pub fn add_spin(&mut self, shot: &crate::Shot, report: &ShotReport) {
        if report.is_foul() || (shot.tip.length() < 0.3 && shot.elevation < 0.2) {
            return;
        }
        self.spin_pots += report.pocketed.iter().filter(|(b, _)| b.is_object()).count() as u32;
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
