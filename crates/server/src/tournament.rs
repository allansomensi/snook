//! Single-elimination brackets.
//!
//! Players are seeded by rating so the strongest meet as late as possible;
//! empty seats are byes, and a player whose opponent is a bye goes through
//! at once.

use snook_proto::BracketMatch;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    /// Lower-case usernames.
    pub players: [Option<String>; 2],
    /// A seat that will never be filled (a bye, or the end of an empty branch).
    pub dead: [bool; 2],
    pub winner: Option<usize>,
    /// The match room playing this slot.
    pub room: Option<u64>,
}

impl Slot {
    fn empty() -> Slot {
        Slot { players: [None, None], dead: [false, false], winner: None, room: None }
    }

    /// Both seats are filled and the match has not been decided.
    pub fn playable(&self) -> bool {
        self.winner.is_none() && self.players[0].is_some() && self.players[1].is_some()
    }

    /// Nobody will come out of this slot.
    fn is_dead(&self) -> bool {
        self.dead[0] && self.dead[1]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bracket {
    pub rounds: Vec<Vec<Slot>>,
}

/// Seat order for a bracket of `size` (a power of two): seed numbers
/// (0-based) in bracket order, so seed 0 and seed 1 can only meet in the final.
fn seeding(size: usize) -> Vec<usize> {
    let mut order = vec![0usize];
    while order.len() < size {
        let n = order.len() * 2;
        order = order.iter().flat_map(|&s| [s, n - 1 - s]).collect();
    }
    order
}

impl Bracket {
    /// A bracket for `players`, strongest first.
    pub fn new(players: &[String]) -> Bracket {
        let size = players.len().max(2).next_power_of_two();
        let order = seeding(size);
        let mut first = Vec::with_capacity(size / 2);
        for pair in order.chunks(2) {
            let mut slot = Slot::empty();
            for (seat, &seed) in pair.iter().enumerate() {
                match players.get(seed) {
                    Some(p) => slot.players[seat] = Some(p.clone()),
                    None => slot.dead[seat] = true,
                }
            }
            first.push(slot);
        }
        let mut rounds = vec![first];
        while rounds.last().is_some_and(|r| r.len() > 1) {
            let n = rounds.last().map(|r| r.len() / 2).unwrap_or(0);
            rounds.push(vec![Slot::empty(); n]);
        }
        let mut b = Bracket { rounds };
        b.resolve();
        b
    }

    /// Moves winners forward and settles byes until nothing changes.
    pub fn resolve(&mut self) {
        loop {
            let mut changed = false;
            for r in 0..self.rounds.len() {
                for i in 0..self.rounds[r].len() {
                    let slot = &mut self.rounds[r][i];
                    if slot.winner.is_none() && !slot.is_dead() {
                        match (&slot.players, slot.dead) {
                            ([Some(_), None], [false, true]) => {
                                slot.winner = Some(0);
                                changed = true;
                            }
                            ([None, Some(_)], [true, false]) => {
                                slot.winner = Some(1);
                                changed = true;
                            }
                            _ => {}
                        }
                    }
                    if r + 1 < self.rounds.len() {
                        let (winner, dead) = {
                            let s = &self.rounds[r][i];
                            (s.winner.and_then(|w| s.players[w].clone()), s.is_dead())
                        };
                        let next = &mut self.rounds[r + 1][i / 2];
                        let seat = i % 2;
                        if let Some(w) = winner
                            && next.players[seat].is_none()
                        {
                            next.players[seat] = Some(w);
                            changed = true;
                        }
                        if dead && !next.dead[seat] {
                            next.dead[seat] = true;
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }

    /// Records the winner of a slot.
    pub fn set_winner(&mut self, round: usize, index: usize, seat: usize) {
        if let Some(slot) = self.rounds.get_mut(round).and_then(|r| r.get_mut(index))
            && slot.winner.is_none()
            && slot.players[seat].is_some()
        {
            slot.winner = Some(seat);
            slot.room = None;
            self.resolve();
        }
    }

    /// The champion, once the final is decided.
    pub fn champion(&self) -> Option<&String> {
        let last = self.rounds.last()?.first()?;
        last.winner.and_then(|w| last.players[w].as_ref())
    }

    /// Players still in the running.
    pub fn alive(&self, player: &str) -> bool {
        if self.champion().is_some() {
            return false;
        }
        // Eliminated players lost a decided slot.
        !self.rounds.iter().flatten().any(|s| {
            s.winner.is_some_and(|w| s.players[1 - w].as_deref() == Some(player))
        })
    }

    /// (round, index) of every slot ready to be played.
    pub fn playable(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for (r, round) in self.rounds.iter().enumerate() {
            for (i, s) in round.iter().enumerate() {
                if s.playable() {
                    out.push((r, i));
                }
            }
        }
        out
    }

    pub fn to_info(&self, display: impl Fn(&str) -> String) -> Vec<Vec<BracketMatch>> {
        self.rounds
            .iter()
            .map(|round| {
                round
                    .iter()
                    .map(|s| BracketMatch {
                        players: [
                            s.players[0].as_deref().map(&display),
                            s.players[1].as_deref().map(&display),
                        ],
                        winner: s.winner,
                        live: s.room.is_some(),
                    })
                    .collect()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("p{i}")).collect()
    }

    #[test]
    fn seeding_keeps_top_seeds_apart() {
        assert_eq!(seeding(4), vec![0, 3, 1, 2]);
        assert_eq!(seeding(8), vec![0, 7, 3, 4, 1, 6, 2, 5]);
    }

    #[test]
    fn full_bracket_plays_to_a_champion() {
        let mut b = Bracket::new(&names(4));
        assert_eq!(b.playable(), vec![(0, 0), (0, 1)]);
        b.set_winner(0, 0, 0);
        b.set_winner(0, 1, 1);
        assert_eq!(b.rounds[1][0].players, [Some("p0".into()), Some("p2".into())]);
        assert!(!b.alive("p3"));
        b.set_winner(1, 0, 1);
        assert_eq!(b.champion().map(String::as_str), Some("p2"));
    }

    #[test]
    fn byes_advance_players() {
        // 5 players in a bracket of 8: the top 3 seeds get byes.
        let b = Bracket::new(&names(5));
        let first_round_games = b.rounds[0].iter().filter(|s| s.playable()).count();
        assert_eq!(first_round_games, 1);
        assert_eq!(b.rounds[1][0].players[0].as_deref(), Some("p0"));
        // 3 players: seed 0 waits in the final.
        let b = Bracket::new(&names(3));
        assert_eq!(b.rounds[1][0].players[0].as_deref(), Some("p0"));
        assert_eq!(b.playable(), vec![(0, 1)]);
    }

    #[test]
    fn two_players_is_just_a_final() {
        let mut b = Bracket::new(&names(2));
        assert_eq!(b.rounds.len(), 1);
        b.set_winner(0, 0, 0);
        assert_eq!(b.champion().map(String::as_str), Some("p0"));
    }
}
