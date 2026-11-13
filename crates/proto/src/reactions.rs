//! Reactions: small pictures a player can send during a match. A few are
//! free; others come with achievements (including event ones), and a set
//! is for VIP members while they are VIP.

/// A reaction. `art` names its picture in the game's atlas (a Twemoji code
/// point); `unlock` is the achievement that grants it (`None`: everyone,
/// [`VIP`]: VIP members).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reaction {
    pub code: &'static str,
    pub art: &'static str,
    pub unlock: Option<&'static str>,
}

/// The `unlock` of the reactions VIP members have while they are VIP.
pub const VIP: &str = "vip";

const fn r(code: &'static str, art: &'static str, unlock: Option<&'static str>) -> Reaction {
    Reaction { code, art, unlock }
}

/// Every reaction, in the order the picker shows them.
pub const REACTIONS: &[Reaction] = &[
    r("thumbs_up", "1f44d", None),
    r("clap", "1f44f", None),
    r("wow", "1f62e", None),
    r("handshake", "1f91d", None),
    // Everything else takes real play: rated games only.
    r("thinking", "1f914", Some("games_100")),
    r("phew", "1f605", Some("wins_50")),
    r("heart", "2764", Some("friend_10")),
    r("cool", "1f60e", Some("wins_100")),
    r("laugh", "1f602", Some("games_500")),
    r("ice", "1f976", Some("shutout_10")),
    r("eight", "1f3b1", Some("eight_100")),
    r("fire", "1f525", Some("streak_10")),
    r("target", "1f3af", Some("runout_10")),
    r("shark", "1f988", Some("wins_500")),
    r("muscle", "1f4aa", Some("games_1000")),
    r("trophy", "1f3c6", Some("tournament_win_5")),
    r("crown", "1f451", Some("rating_2100")),
    r("pumpkin", "1f383", Some("event_halloween")),
    r("tree", "1f384", Some("event_winter")),
    r("fireworks", "1f386", Some("event_new_year")),
    // For VIP members, while they are VIP.
    r("star_eyes", "1f929", Some(VIP)),
    r("gem", "1f48e", Some(VIP)),
    r("rocket", "1f680", Some(VIP)),
    r("party", "1f389", Some(VIP)),
    r("hundred", "1f4af", Some(VIP)),
    r("sparkles", "2728", Some(VIP)),
];

/// The reaction with this code.
pub fn reaction(code: &str) -> Option<&'static Reaction> {
    REACTIONS.iter().find(|r| r.code == code)
}

impl Reaction {
    /// Whether a player with these achievements may send it (VIP
    /// reactions: never; see [`Reaction::available`]).
    pub fn unlocked<S: AsRef<str>>(&self, achievements: &[S]) -> bool {
        !self.is_vip() && self.unlock.is_none_or(|a| achievements.iter().any(|c| c.as_ref() == a))
    }

    /// Whether a player with these achievements, VIP or not, may send it.
    pub fn available<S: AsRef<str>>(&self, achievements: &[S], vip: bool) -> bool {
        (vip && self.is_vip()) || self.unlocked(achievements)
    }

    /// One of the reactions for VIP members.
    pub fn is_vip(&self) -> bool {
        self.unlock == Some(VIP)
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
        for r in REACTIONS.iter().filter(|r| !r.is_vip()) {
            if let Some(a) = r.unlock {
                assert!(crate::ACHIEVEMENTS.iter().any(|x| x.code == a), "{a}");
            }
        }
        let none: [&str; 0] = [];
        assert!(reaction("thumbs_up").unwrap().unlocked(&none));
        assert!(!reaction("crown").unwrap().unlocked(&none));
        assert!(reaction("crown").unwrap().unlocked(&["rating_2100"]));
        let gem = reaction("gem").unwrap();
        assert!(gem.available(&none, true) && !gem.available(&none, false));
        assert!(!gem.unlocked(&[VIP]), "VIP comes from the wallet, not achievements");
        assert!(reaction("crown").unwrap().available(&["rating_2100"], false));
        // Few are free.
        assert!(REACTIONS.iter().filter(|r| r.unlock.is_none()).count() <= 4);
    }
}
