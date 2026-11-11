//! Reactions: small pictures a player can send during a match. A few are
//! free; the others come with achievements (including event ones).

/// A reaction. `art` names its picture in the game's atlas (a Twemoji code
/// point); `unlock` is the achievement that grants it (`None`: everyone).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reaction {
    pub code: &'static str,
    pub art: &'static str,
    pub unlock: Option<&'static str>,
}

const fn r(code: &'static str, art: &'static str, unlock: Option<&'static str>) -> Reaction {
    Reaction { code, art, unlock }
}

/// Every reaction, in the order the picker shows them.
pub const REACTIONS: &[Reaction] = &[
    r("thumbs_up", "1f44d", None),
    r("clap", "1f44f", None),
    r("wow", "1f62e", None),
    r("phew", "1f605", None),
    r("thinking", "1f914", None),
    r("handshake", "1f91d", None),
    r("laugh", "1f602", Some("games_10")),
    r("cool", "1f60e", Some("wins_10")),
    r("heart", "2764", Some("friend_1")),
    r("ice", "1f976", Some("shutout_1")),
    r("eight", "1f3b1", Some("eight_10")),
    r("fire", "1f525", Some("streak_5")),
    r("target", "1f3af", Some("runout_1")),
    r("shark", "1f988", Some("wins_50")),
    r("muscle", "1f4aa", Some("wins_100")),
    r("trophy", "1f3c6", Some("tournament_win")),
    r("crown", "1f451", Some("rating_1900")),
    r("star", "2b50", Some("event_founder")),
    r("pumpkin", "1f383", Some("event_halloween")),
    r("tree", "1f384", Some("event_winter")),
    r("fireworks", "1f386", Some("event_new_year")),
];

/// The reaction with this code.
pub fn reaction(code: &str) -> Option<&'static Reaction> {
    REACTIONS.iter().find(|r| r.code == code)
}

impl Reaction {
    /// Whether a player with these achievements may send it.
    pub fn unlocked<S: AsRef<str>>(&self, achievements: &[S]) -> bool {
        self.unlock.is_none_or(|a| achievements.iter().any(|c| c.as_ref() == a))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reaction_is_unique_and_reachable() {
        let mut codes: Vec<_> = REACTIONS.iter().map(|r| r.code).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), REACTIONS.len());
        for r in REACTIONS {
            if let Some(a) = r.unlock {
                assert!(crate::ACHIEVEMENTS.iter().any(|x| x.code == a), "{a}");
            }
        }
        let none: [&str; 0] = [];
        assert!(reaction("thumbs_up").unwrap().unlocked(&none));
        assert!(!reaction("crown").unwrap().unlocked(&none));
        assert!(reaction("crown").unwrap().unlocked(&["rating_1900"]));
    }
}
