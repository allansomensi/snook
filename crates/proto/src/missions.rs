//! Daily and weekly missions: small goals for rated games that bring coins.
//!
//! The same missions for everyone on a given day (or week), picked from the
//! lists below. Progress counts only in rated games that also pay coins
//! (played to the end, not between two players farming each other).

use serde::{Deserialize, Serialize};
use snookered_core::stats::ShotStats;

/// What a mission counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Goal {
    /// Rated games played to the end.
    Play,
    Win,
    /// Object balls legally made.
    Balls,
    /// Balls made with side, top or bottom spin (or a raised cue).
    SpinPots,
    /// Visits to the table with three or more balls made in a row.
    Runs,
    /// Games won without a single foul.
    CleanWins,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mission {
    pub code: &'static str,
    /// English, with `{n}` for the target (translated by the game).
    pub text: &'static str,
    pub goal: Goal,
    pub target: u32,
    pub reward: u32,
    pub weekly: bool,
}

const fn m(
    code: &'static str,
    text: &'static str,
    goal: Goal,
    target: u32,
    reward: u32,
    weekly: bool,
) -> Mission {
    Mission { code, text, goal, target, reward, weekly }
}

pub const DAILY: &[Mission] = &[
    m("d_play", "Play {n} rated games", Goal::Play, 3, 12, false),
    m("d_win", "Win {n} rated games", Goal::Win, 2, 15, false),
    m("d_balls", "Pot {n} balls", Goal::Balls, 15, 10, false),
    m("d_spin", "Pot {n} balls with spin", Goal::SpinPots, 5, 12, false),
    m("d_run", "Pot 3 balls in a row in one visit", Goal::Runs, 1, 10, false),
    m("d_clean", "Win a game without a foul", Goal::CleanWins, 1, 15, false),
];

pub const WEEKLY: &[Mission] = &[
    m("w_play", "Play {n} rated games", Goal::Play, 20, 50, true),
    m("w_win", "Win {n} rated games", Goal::Win, 10, 70, true),
    m("w_balls", "Pot {n} balls", Goal::Balls, 100, 50, true),
    m("w_spin", "Pot {n} balls with spin", Goal::SpinPots, 40, 60, true),
    m("w_run", "Pot 3 balls in a row {n} times", Goal::Runs, 10, 50, true),
    m("w_clean", "Win {n} games without a foul", Goal::CleanWins, 5, 70, true),
];

/// Missions a day has, and a week.
pub const DAILY_COUNT: usize = 3;
pub const WEEKLY_COUNT: usize = 2;

pub fn mission(code: &str) -> Option<&'static Mission> {
    DAILY.iter().chain(WEEKLY).find(|m| m.code == code)
}

/// Days since 1970-01-01 (UTC).
pub fn day_of(t: i64) -> i64 {
    t.div_euclid(86_400)
}

/// Weeks since the Monday before 1970-01-01 (weeks start on Monday, UTC).
pub fn week_of(t: i64) -> i64 {
    (day_of(t) + 3).div_euclid(7)
}

/// When the day of `t` ends (Unix seconds).
pub fn day_ends(t: i64) -> i64 {
    (day_of(t) + 1) * 86_400
}

/// When the week of `t` ends (Unix seconds).
pub fn week_ends(t: i64) -> i64 {
    ((week_of(t) + 1) * 7 - 3) * 86_400
}

/// `count` different missions of `list`, the same for everyone in `period`.
fn pick(list: &'static [Mission], count: usize, period: i64) -> Vec<&'static Mission> {
    let mut order: Vec<usize> = (0..list.len()).collect();
    // A small shuffle seeded by the period (splitmix64).
    let mut x = (period as u64).wrapping_add(0x9E37_79B9_7F4A_7C15);
    for i in (1..order.len()).rev() {
        x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        order.swap(i, (z % (i as u64 + 1)) as usize);
    }
    order.into_iter().take(count).map(|i| &list[i]).collect()
}

/// Today's missions.
pub fn daily(t: i64) -> Vec<&'static Mission> {
    pick(DAILY, DAILY_COUNT, day_of(t))
}

/// This week's missions.
pub fn weekly(t: i64) -> Vec<&'static Mission> {
    pick(WEEKLY, WEEKLY_COUNT, week_of(t))
}

/// The period a mission's progress is kept under: `d<day>` or `w<week>`.
pub fn period(m: &Mission, t: i64) -> String {
    if m.weekly { format!("w{}", week_of(t)) } else { format!("d{}", day_of(t)) }
}

/// How far one game takes a player towards `goal`.
pub fn counts(goal: Goal, won: bool, stats: &ShotStats) -> u32 {
    match goal {
        Goal::Play => 1,
        Goal::Win => won as u32,
        Goal::Balls => stats.balls,
        Goal::SpinPots => stats.spin_pots,
        Goal::Runs => stats.runs,
        Goal::CleanWins => (won && stats.fouls == 0) as u32,
    }
}

/// One mission and how far the player is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionState {
    pub code: String,
    pub progress: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periods_and_picks() {
        // 2024-01-01 was a Monday.
        let monday = 1_704_067_200;
        assert_eq!(week_of(monday), week_of(monday + 6 * 86_400 + 86_399));
        assert_ne!(week_of(monday), week_of(monday - 1));
        assert_eq!(week_ends(monday), monday + 7 * 86_400);
        assert_eq!(day_ends(monday + 5), monday + 86_400);
        for d in 0..60 {
            let t = monday + d * 86_400;
            let today = daily(t);
            assert_eq!(today.len(), DAILY_COUNT);
            let mut codes: Vec<_> = today.iter().map(|m| m.code).collect();
            codes.dedup();
            assert_eq!(codes.len(), DAILY_COUNT, "different missions");
            assert_eq!(daily(t + 3600), today, "the same all day");
            assert!(weekly(t).iter().all(|m| m.weekly));
        }
        let all: std::collections::HashSet<_> =
            (0..30).flat_map(|d| daily(monday + d * 86_400)).map(|m| m.code).collect();
        assert_eq!(all.len(), DAILY.len(), "every mission comes up");
        assert!(DAILY.iter().chain(WEEKLY).all(|m| mission(m.code) == Some(m)));
    }
}
