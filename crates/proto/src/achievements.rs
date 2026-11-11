//! The achievement catalogue, shared by the server (which unlocks them) and
//! the game (which shows them).

use serde::{Deserialize, Serialize};

/// Lifetime numbers of an account, used for achievements and profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Stats {
    /// Rated games.
    pub games: u32,
    /// Rated wins.
    pub wins: u32,
    /// Rated wins by legally pocketing the 8-ball.
    pub eight_wins: u32,
    pub casual_games: u32,
    pub peak_rating: i32,
    /// Current run of rated wins.
    pub streak: u32,
    pub best_streak: u32,
    /// Games won from the break without the opponent getting a turn.
    pub runouts: u32,
    /// Games won before the opponent pocketed a single ball of their group.
    pub shutouts: u32,
    pub friends: u32,
    pub tournaments_played: u32,
    pub tournaments_won: u32,
    /// Days since the account was created.
    pub account_days: u32,
    /// The date of a rated win just recorded, as month * 100 + day, UTC
    /// (0: none): event achievements need a rated win during the event.
    #[serde(default)]
    pub today: u16,
}

/// Which number an achievement looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stat {
    Games,
    Wins,
    EightWins,
    CasualGames,
    PeakRating,
    BestStreak,
    Runouts,
    Shutouts,
    Friends,
    TournamentsPlayed,
    TournamentsWon,
    AccountDays,
    /// 1 while today (month * 100 + day) is in this range, both ends
    /// included; a range that wraps (Dec–Jan) works.
    Season(u16, u16),
}

impl Stat {
    /// Event achievements are unlocked by being there, not by numbers.
    pub fn is_event(self) -> bool {
        matches!(self, Stat::Season(..))
    }
}

impl Stats {
    pub fn get(&self, stat: Stat) -> i64 {
        match stat {
            Stat::Games => self.games as i64,
            Stat::Wins => self.wins as i64,
            Stat::EightWins => self.eight_wins as i64,
            Stat::CasualGames => self.casual_games as i64,
            Stat::PeakRating => self.peak_rating as i64,
            Stat::BestStreak => self.best_streak as i64,
            Stat::Runouts => self.runouts as i64,
            Stat::Shutouts => self.shutouts as i64,
            Stat::Friends => self.friends as i64,
            Stat::TournamentsPlayed => self.tournaments_played as i64,
            Stat::TournamentsWon => self.tournaments_won as i64,
            Stat::AccountDays => self.account_days as i64,
            Stat::Season(from, to) => {
                let d = self.today;
                let inside = if from <= to { d >= from && d <= to } else { d >= from || d <= to };
                (d != 0 && inside) as i64
            }
        }
    }

    /// Codes of every achievement these numbers have earned.
    pub fn earned(&self) -> impl Iterator<Item = &'static str> + '_ {
        ACHIEVEMENTS.iter().filter(|a| self.get(a.stat) >= a.goal).map(|a| a.code)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Achievement {
    pub code: &'static str,
    /// English title and description (translated by the game).
    pub title: &'static str,
    pub description: &'static str,
    pub stat: Stat,
    pub goal: i64,
    /// 0 bronze, 1 silver, 2 gold, 3 platinum.
    pub level: u8,
}

const fn a(
    code: &'static str,
    title: &'static str,
    description: &'static str,
    stat: Stat,
    goal: i64,
    level: u8,
) -> Achievement {
    Achievement { code, title, description, stat, goal, level }
}

pub const ACHIEVEMENTS: &[Achievement] = &[
    a("games_1", "First break", "Play your first rated game.", Stat::Games, 1, 0),
    a("games_10", "Regular", "Play 10 rated games.", Stat::Games, 10, 0),
    a("games_100", "Hustler", "Play 100 rated games.", Stat::Games, 100, 1),
    a("games_500", "Pool hall legend", "Play 500 rated games.", Stat::Games, 500, 2),
    a("games_1000", "Lives at the table", "Play 1,000 rated games.", Stat::Games, 1000, 3),
    a("wins_1", "First blood", "Win a rated game.", Stat::Wins, 1, 0),
    a("wins_10", "On a roll", "Win 10 rated games.", Stat::Wins, 10, 0),
    a("wins_50", "Shark", "Win 50 rated games.", Stat::Wins, 50, 1),
    a("wins_100", "Centurion", "Win 100 rated games.", Stat::Wins, 100, 2),
    a("wins_500", "Unstoppable", "Win 500 rated games.", Stat::Wins, 500, 3),
    a("eight_10", "Eight is enough", "Win 10 rated games on the 8-ball.", Stat::EightWins, 10, 0),
    a("eight_50", "Black ball", "Win 50 rated games on the 8-ball.", Stat::EightWins, 50, 1),
    a("eight_100", "Closer", "Win 100 rated games on the 8-ball.", Stat::EightWins, 100, 2),
    a("eight_250", "Executioner", "Win 250 rated games on the 8-ball.", Stat::EightWins, 250, 3),
    a("rating_1300", "Skilled", "Reach a rating of 1300.", Stat::PeakRating, 1300, 0),
    a("rating_1500", "Expert", "Reach a rating of 1500.", Stat::PeakRating, 1500, 1),
    a("rating_1700", "Master", "Reach a rating of 1700.", Stat::PeakRating, 1700, 2),
    a("rating_1900", "Grandmaster", "Reach a rating of 1900.", Stat::PeakRating, 1900, 3),
    a("rating_2100", "Legend", "Reach a rating of 2100.", Stat::PeakRating, 2100, 3),
    a("streak_3", "Hat trick", "Win 3 rated games in a row.", Stat::BestStreak, 3, 0),
    a("streak_5", "Hot hand", "Win 5 rated games in a row.", Stat::BestStreak, 5, 1),
    a("streak_10", "On fire", "Win 10 rated games in a row.", Stat::BestStreak, 10, 3),
    a(
        "runout_1",
        "Break and run",
        "Win a rated game from the break without your opponent getting a shot.",
        Stat::Runouts,
        1,
        1,
    ),
    a("runout_10", "Table runner", "Break and run 10 times.", Stat::Runouts, 10, 3),
    a(
        "shutout_1",
        "Shutout",
        "Win a rated game before your opponent sinks a ball of their group.",
        Stat::Shutouts,
        1,
        0,
    ),
    a("shutout_10", "Whitewash", "Win 10 rated shutouts.", Stat::Shutouts, 10, 2),
    a("casual_25", "Just for fun", "Play 25 casual games.", Stat::CasualGames, 25, 0),
    a("friend_1", "Good company", "Make a friend.", Stat::Friends, 1, 0),
    a("friend_10", "Popular", "Have 10 friends.", Stat::Friends, 10, 1),
    a("tournament_play", "Competitor", "Play in a tournament.", Stat::TournamentsPlayed, 1, 0),
    a("tournament_win", "Champion", "Win a tournament.", Stat::TournamentsWon, 1, 2),
    a("tournament_win_5", "Dynasty", "Win 5 tournaments.", Stat::TournamentsWon, 5, 3),
    a("veteran_30", "Familiar face", "Have an account for 30 days.", Stat::AccountDays, 30, 0),
    a("veteran_365", "Old timer", "Have an account for a year.", Stat::AccountDays, 365, 2),
    // Events: being online at the right time. Each one unlocks a reaction.
    a(
        "event_halloween",
        "Haunted table",
        "Win a rated game during Halloween (Oct 24 – Nov 2).",
        Stat::Season(1024, 1102),
        1,
        1,
    ),
    a(
        "event_winter",
        "Winter league",
        "Win a rated game over the holidays (Dec 15 – Jan 6).",
        Stat::Season(1215, 106),
        1,
        1,
    ),
    a(
        "event_new_year",
        "Fireworks",
        "Win a rated game at the turn of the year (Dec 31 – Jan 1).",
        Stat::Season(1231, 101),
        1,
        2,
    ),
];

/// Month * 100 + day of a Unix time, in UTC.
pub fn month_day(unix: i64) -> u16 {
    // Civil-from-days (Howard Hinnant), enough for any date after 1970.
    let z = unix.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (m * 100 + d) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_unique_and_earned_by_numbers() {
        let mut codes: Vec<_> = ACHIEVEMENTS.iter().map(|a| a.code).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), ACHIEVEMENTS.len());
        let s = Stats { games: 12, wins: 1, peak_rating: 1310, ..Stats::default() };
        let earned: Vec<_> = s.earned().collect();
        assert!(earned.contains(&"games_10") && earned.contains(&"rating_1300"));
        assert!(!earned.contains(&"games_100"));
        assert!(!earned.iter().any(|c| c.starts_with("event_")));
    }

    #[test]
    fn events_follow_the_calendar() {
        // 2026-10-31 and 2027-01-01, noon UTC.
        assert_eq!(month_day(1_793_448_000), 1031);
        assert_eq!(month_day(1_798_804_800), 101);
        assert_eq!(month_day(0), 101);
        let at = |today| Stats { today, ..Stats::default() };
        let has = |s: Stats, code| s.earned().any(|c| c == code);
        assert!(has(at(1031), "event_halloween") && !has(at(1103), "event_halloween"));
        assert!(has(at(1220), "event_winter") && has(at(106), "event_winter"));
        assert!(!has(at(1214), "event_winter") && !has(at(107), "event_winter"));
        assert!(has(at(1231), "event_new_year") && !has(at(102), "event_new_year"));
    }
}
