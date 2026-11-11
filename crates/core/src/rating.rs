//! Elo rating, as used in chess.
//!
//! The expected score of A against B is
//! `E_A = 1 / (1 + 10^((R_B - R_A) / 400))` and after a game
//! `R_A' = R_A + K * (S_A - E_A)`, where `S_A` is 1 for a win and 0 for a loss.
//!
//! So two equally rated players exchange about `K/2` points, a favourite who wins
//! gains little, and an underdog who wins gains a lot. Like FIDE, the K-factor is
//! larger for new players (their rating settles quickly) and smaller at the top
//! (ratings become stable). The factors are deliberately small: a rating is
//! earned over many games, not a lucky evening.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EloConfig {
    pub initial_rating: i32,
    /// Players with fewer rated games than this use `k_provisional`.
    pub provisional_games: u32,
    pub k_provisional: f64,
    pub k_standard: f64,
    /// Players at or above `elite_threshold` use `k_elite`.
    pub k_elite: f64,
    pub elite_threshold: i32,
    /// Ratings never drop below this floor.
    pub floor: i32,
}

impl Default for EloConfig {
    fn default() -> Self {
        EloConfig {
            initial_rating: 1200,
            provisional_games: 10,
            k_provisional: 24.0,
            k_standard: 14.0,
            k_elite: 8.0,
            elite_threshold: 1900,
            floor: 100,
        }
    }
}

/// A player's rating record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rating {
    pub rating: i32,
    pub games: u32,
}

impl Rating {
    pub fn new(config: &EloConfig) -> Self {
        Rating { rating: config.initial_rating, games: 0 }
    }

    pub fn is_provisional(&self, config: &EloConfig) -> bool {
        self.games < config.provisional_games
    }

    fn k_factor(&self, config: &EloConfig) -> f64 {
        if self.games < config.provisional_games {
            config.k_provisional
        } else if self.rating >= config.elite_threshold {
            config.k_elite
        } else {
            config.k_standard
        }
    }
}

/// Probability that a player rated `a` beats a player rated `b`.
pub fn expected_score(a: i32, b: i32) -> f64 {
    1.0 / (1.0 + 10f64.powf((b - a) as f64 / 400.0))
}

/// Result of updating both players after a game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RatingChange {
    pub winner: Rating,
    pub loser: Rating,
    pub winner_delta: i32,
    pub loser_delta: i32,
}

/// Updates both ratings after `winner` beat `loser`.
pub fn apply_result(config: &EloConfig, winner: Rating, loser: Rating) -> RatingChange {
    let expected_w = expected_score(winner.rating, loser.rating);
    let expected_l = 1.0 - expected_w;
    let gain = (winner.k_factor(config) * (1.0 - expected_w)).round() as i32;
    let loss = (loser.k_factor(config) * expected_l).round() as i32;
    let new_w = Rating { rating: winner.rating + gain, games: winner.games + 1 };
    let new_l = Rating { rating: (loser.rating - loss).max(config.floor), games: loser.games + 1 };
    RatingChange {
        winner_delta: new_w.rating - winner.rating,
        loser_delta: new_l.rating - loser.rating,
        winner: new_w,
        loser: new_l,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn established(rating: i32) -> Rating {
        Rating { rating, games: 100 }
    }

    #[test]
    fn expected_scores() {
        assert!((expected_score(1500, 1500) - 0.5).abs() < 1e-12);
        assert!((expected_score(1800, 1400) - 0.909).abs() < 1e-3);
        assert!((expected_score(1400, 1800) + expected_score(1800, 1400) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn equal_players_exchange_half_k() {
        let c = EloConfig::default();
        let r = apply_result(&c, established(1500), established(1500));
        assert_eq!(r.winner_delta, 7);
        assert_eq!(r.loser_delta, -7);
    }

    #[test]
    fn favourite_gains_little_underdog_gains_a_lot() {
        let c = EloConfig::default();
        let fav = apply_result(&c, established(1800), established(1400));
        let upset = apply_result(&c, established(1400), established(1800));
        assert_eq!(fav.winner_delta, 1);
        assert_eq!(upset.winner_delta, 13);
        assert_eq!(upset.loser_delta, -13);
    }

    #[test]
    fn provisional_players_move_faster_and_floor_holds() {
        let c = EloConfig::default();
        let newbie = Rating::new(&c);
        let r = apply_result(&c, newbie, established(1200));
        assert_eq!(r.winner_delta, 12);
        assert_eq!(r.winner.games, 1);
        let r = apply_result(&c, established(110), Rating { rating: 105, games: 50 });
        assert_eq!(r.loser.rating, 100);
    }
}
