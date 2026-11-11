//! Eight-ball rules (based on the WPA world standardized rules, simplified where
//! noted).
//!
//! The game is driven in two phases so that clients can animate the shot:
//! 1. [`EightBallGame::prepare_shot`] validates the stroke and returns a
//!    [`PendingShot`] holding a running [`Simulation`];
//! 2. once the simulation has settled, [`EightBallGame::finish_shot`] applies the
//!    rules and returns a [`ShotReport`].
//!
//! [`EightBallGame::play_shot`] does both at once (used by the AI and the server).

use glam::DVec2;
use serde::{Deserialize, Serialize};

use crate::ball::{Ball, BallId, BallKind, BallStatus};
use crate::physics::{Event, PhysicsParams, Shot, Simulation};
use crate::rng::Rng;
use crate::table::{PocketId, Table};

/// The two groups of object balls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Group {
    Solids,
    Stripes,
}

impl Group {
    pub fn contains(self, id: BallId) -> bool {
        matches!(
            (self, id.kind()),
            (Group::Solids, BallKind::Solid) | (Group::Stripes, BallKind::Stripe)
        )
    }

    pub fn of(id: BallId) -> Option<Group> {
        match id.kind() {
            BallKind::Solid => Some(Group::Solids),
            BallKind::Stripe => Some(Group::Stripes),
            _ => None,
        }
    }

    pub fn opposite(self) -> Group {
        match self {
            Group::Solids => Group::Stripes,
            Group::Stripes => Group::Solids,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Group::Solids => "solids",
            Group::Stripes => "stripes",
        }
    }
}

/// Where the incoming player may place the cue ball.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BallInHand {
    /// Behind the head string (after a break foul, and for the break itself).
    Kitchen,
    Anywhere,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Foul {
    CueBallPocketed,
    CueBallOffTable,
    NoContact,
    WrongBallFirst(BallId),
    NoRailAfterContact,
    ObjectBallOffTable(BallId),
    /// No ball pocketed and fewer than four object balls driven to a cushion.
    IllegalBreak,
}

impl std::fmt::Display for Foul {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Foul::CueBallPocketed => write!(f, "Scratch: the cue ball was pocketed"),
            Foul::CueBallOffTable => write!(f, "The cue ball left the table"),
            Foul::NoContact => write!(f, "The cue ball didn't hit any ball"),
            Foul::WrongBallFirst(id) => write!(f, "Wrong ball hit first ({id})"),
            Foul::NoRailAfterContact => write!(f, "No ball reached a cushion after contact"),
            Foul::ObjectBallOffTable(id) => write!(f, "The {id} left the table"),
            Foul::IllegalBreak => write!(f, "Illegal break"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameOverReason {
    /// The winner legally pocketed the eight ball in the called pocket.
    PocketedEight,
    /// The loser pocketed the eight before clearing their group.
    EightPocketedEarly,
    /// The loser pocketed the eight in a pocket other than the one called.
    EightInWrongPocket,
    /// The loser pocketed the eight on a foul.
    EightPocketedOnFoul,
    /// The loser knocked the eight off the table.
    EightOffTable,
    Resignation,
    /// The loser left an online game and did not come back in time.
    Abandoned,
}

impl GameOverReason {
    pub fn description(self) -> &'static str {
        match self {
            GameOverReason::PocketedEight => "pocketed the 8-ball",
            GameOverReason::EightPocketedEarly => "opponent pocketed the 8-ball too early",
            GameOverReason::EightInWrongPocket => "opponent sank the 8-ball in the wrong pocket",
            GameOverReason::EightPocketedOnFoul => "opponent fouled on the 8-ball",
            GameOverReason::EightOffTable => "opponent knocked the 8-ball off the table",
            GameOverReason::Resignation => "opponent resigned",
            GameOverReason::Abandoned => "opponent left the game",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameOver {
    pub winner: usize,
    pub reason: GameOverReason,
}

/// Outcome of a shot, for display and for the AI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShotReport {
    pub shooter: usize,
    pub was_break: bool,
    pub first_contact: Option<BallId>,
    pub pocketed: Vec<(BallId, PocketId)>,
    pub fouls: Vec<Foul>,
    /// Balls returned to the table (object balls that left it, or the 8 on the break).
    pub spotted: Vec<BallId>,
    /// Set when this shot decided the groups; the group is the shooter's.
    pub group_assigned: Option<Group>,
    /// True if the shooter keeps the table.
    pub continues: bool,
    pub game_over: Option<GameOver>,
}

impl ShotReport {
    pub fn is_foul(&self) -> bool {
        !self.fouls.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShotError {
    GameIsOver,
    /// The cue ball is in hand and its current position is not allowed.
    IllegalCueBallPosition,
    /// The shooter is on the eight and must nominate a pocket.
    MustCallPocket,
}

impl std::fmt::Display for ShotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShotError::GameIsOver => write!(f, "the game is over"),
            ShotError::IllegalCueBallPosition => write!(f, "the cue ball cannot be placed there"),
            ShotError::MustCallPocket => write!(f, "call a pocket for the 8-ball"),
        }
    }
}

impl std::error::Error for ShotError {}

/// A validated shot whose simulation is still running.
#[derive(Debug, Clone)]
pub struct PendingShot {
    pub simulation: Simulation,
    pub shot: Shot,
    pub called_pocket: Option<PocketId>,
}

/// Complete state of an eight-ball game between players `0` and `1`.
///
/// Serializable so the game server can send authoritative snapshots. The
/// table and the physics parameters are not part of a snapshot (they are the
/// standard ones on every machine), which keeps snapshots small: a
/// deserialized game always uses [`Table::default`] and
/// [`PhysicsParams::default`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EightBallGame {
    #[serde(skip)]
    table: Table,
    #[serde(skip)]
    params: PhysicsParams,
    balls: Vec<Ball>,
    current: usize,
    groups: [Option<Group>; 2],
    is_break: bool,
    ball_in_hand: Option<BallInHand>,
    game_over: Option<GameOver>,
    shots_taken: u32,
}

impl EightBallGame {
    /// A freshly racked game on the default table. `seed` randomises the rack.
    pub fn new(seed: u64, breaker: usize) -> Self {
        Self::with_table(Table::default(), PhysicsParams::default(), seed, breaker)
    }

    pub fn with_table(table: Table, params: PhysicsParams, seed: u64, breaker: usize) -> Self {
        let balls = rack(&table, seed);
        EightBallGame {
            table,
            params,
            balls,
            current: breaker % 2,
            groups: [None, None],
            is_break: true,
            ball_in_hand: Some(BallInHand::Kitchen),
            game_over: None,
            shots_taken: 0,
        }
    }

    pub fn table(&self) -> &Table {
        &self.table
    }

    pub fn params(&self) -> &PhysicsParams {
        &self.params
    }

    pub fn balls(&self) -> &[Ball] {
        &self.balls
    }

    pub fn ball(&self, id: BallId) -> &Ball {
        &self.balls[id.index()]
    }

    pub fn current_player(&self) -> usize {
        self.current
    }

    pub fn group_of(&self, player: usize) -> Option<Group> {
        self.groups[player]
    }

    pub fn is_break(&self) -> bool {
        self.is_break
    }

    pub fn is_open_table(&self) -> bool {
        self.groups[0].is_none()
    }

    pub fn ball_in_hand(&self) -> Option<BallInHand> {
        self.ball_in_hand
    }

    pub fn game_over(&self) -> Option<GameOver> {
        self.game_over
    }

    pub fn shots_taken(&self) -> u32 {
        self.shots_taken
    }

    /// Object balls of `group` still on the table.
    pub fn remaining(&self, group: Group) -> Vec<BallId> {
        self.balls.iter().filter(|b| b.in_play() && group.contains(b.id)).map(|b| b.id).collect()
    }

    /// True if `player` has cleared their group and must now pocket the eight.
    pub fn is_on_eight(&self, player: usize) -> bool {
        match self.groups[player] {
            Some(g) => self.remaining(g).is_empty(),
            None => false,
        }
    }

    /// Balls the current player may legally hit first.
    pub fn legal_targets(&self) -> Vec<BallId> {
        let player = self.current;
        let in_play = |id: BallId| self.ball(id).in_play();
        if self.is_break {
            return BallId::all().filter(|id| id.is_object() && in_play(*id)).collect();
        }
        if self.is_on_eight(player) {
            return vec![BallId::EIGHT];
        }
        let targets: Vec<BallId> = match self.groups[player] {
            Some(g) => self.remaining(g),
            None => BallId::all().filter(|id| Group::of(*id).is_some() && in_play(*id)).collect(),
        };
        if targets.is_empty() { vec![BallId::EIGHT] } else { targets }
    }

    /// The shooter must call a pocket for this shot.
    /// A sensible aim when the player has not chosen one: the foot spot on
    /// the break, otherwise the nearest ball they may hit.
    pub fn default_aim(&self) -> f64 {
        let cue = self.ball(BallId::CUE).xy();
        if self.is_break {
            let d = self.table.foot_spot() - cue;
            return d.y.atan2(d.x);
        }
        self.legal_targets()
            .into_iter()
            .map(|id| self.ball(id).xy() - cue)
            .min_by(|a, b| a.length().total_cmp(&b.length()))
            .map(|d| d.y.atan2(d.x))
            .unwrap_or(0.0)
    }

    /// The pocket closest to `ball` (used as a default called pocket).
    pub fn nearest_pocket(&self, ball: BallId) -> PocketId {
        let p = self.ball(ball).xy();
        PocketId::ALL
            .into_iter()
            .min_by(|a, b| {
                let da = self.table.pocket(*a).mouth_center.distance(p);
                let db = self.table.pocket(*b).mouth_center.distance(p);
                da.total_cmp(&db)
            })
            .unwrap_or(PocketId::FootLeft)
    }

    pub fn needs_called_pocket(&self) -> bool {
        !self.is_break && self.is_on_eight(self.current)
    }

    /// True if the cue ball may be placed (or left) at `p` given the ball-in-hand rule.
    pub fn can_place_cue_ball(&self, p: DVec2) -> bool {
        let Some(bih) = self.ball_in_hand else {
            return false;
        };
        if !self.table.is_inside_playing_area(p) {
            return false;
        }
        if bih == BallInHand::Kitchen && p.x > self.table.head_string_x() {
            return false;
        }
        let min_dist = 2.0 * self.table.spec.ball_radius + 1e-4;
        self.balls
            .iter()
            .filter(|b| b.id != BallId::CUE && b.in_play())
            .all(|b| b.xy().distance(p) >= min_dist)
    }

    /// Moves the cue ball while it is in hand.
    pub fn place_cue_ball(&mut self, p: DVec2) -> bool {
        if !self.can_place_cue_ball(p) {
            return false;
        }
        let r = self.table.spec.ball_radius;
        self.balls[0] = Ball::at_rest(BallId::CUE, p, r);
        true
    }

    /// Validates a shot and starts simulating it.
    pub fn prepare_shot(
        &self,
        shot: &Shot,
        called_pocket: Option<PocketId>,
    ) -> Result<PendingShot, ShotError> {
        if self.game_over.is_some() {
            return Err(ShotError::GameIsOver);
        }
        if self.ball_in_hand.is_some() && !self.can_place_cue_ball(self.ball(BallId::CUE).xy()) {
            return Err(ShotError::IllegalCueBallPosition);
        }
        if self.needs_called_pocket() && called_pocket.is_none() {
            return Err(ShotError::MustCallPocket);
        }
        let shot = shot.sanitized(&self.params);
        let mut simulation = Simulation::new(self.table.clone(), self.params, self.balls.clone());
        simulation
            .strike(BallId::CUE, &shot)
            .expect("cue ball is in play and the table is at rest between shots");
        Ok(PendingShot { simulation, shot, called_pocket })
    }

    /// Validates, simulates and applies a shot in one call.
    pub fn play_shot(
        &mut self,
        shot: &Shot,
        called_pocket: Option<PocketId>,
    ) -> Result<ShotReport, ShotError> {
        let mut pending = self.prepare_shot(shot, called_pocket)?;
        pending.simulation.run_to_rest();
        Ok(self.finish_shot(pending))
    }

    /// Applies the rules to a settled shot and advances the game.
    pub fn finish_shot(&mut self, mut pending: PendingShot) -> ShotReport {
        if !pending.simulation.is_settled() {
            pending.simulation.run_to_rest();
        }
        let sim = &pending.simulation;
        let shooter = self.current;
        let was_break = self.is_break;
        let on_eight = self.is_on_eight(shooter);
        let events = sim.events();

        let first_contact_event = events.iter().find_map(|e| match *e {
            Event::BallBall { time, a, b, .. } if a == BallId::CUE => Some((time, b)),
            Event::BallBall { time, a, b, .. } if b == BallId::CUE => Some((time, a)),
            _ => None,
        });
        let first_contact = first_contact_event.map(|(_, id)| id);
        let pocketed: Vec<(BallId, PocketId)> = events
            .iter()
            .filter_map(|e| match *e {
                Event::Pocketed { ball, pocket, .. } => Some((ball, pocket)),
                _ => None,
            })
            .collect();
        let off_table: Vec<BallId> = events
            .iter()
            .filter_map(|e| match *e {
                Event::OffTable { ball, .. } => Some(ball),
                _ => None,
            })
            .collect();
        let object_pocketed: Vec<BallId> =
            pocketed.iter().map(|(b, _)| *b).filter(|b| b.is_object()).collect();
        let rail_after_contact = first_contact_event.is_some_and(|(t0, _)| {
            events.iter().any(|e| matches!(e, Event::Cushion { time, .. } if *time >= t0))
        });

        let mut fouls = Vec::new();
        if pocketed.iter().any(|(b, _)| *b == BallId::CUE) {
            fouls.push(Foul::CueBallPocketed);
        }
        if off_table.contains(&BallId::CUE) {
            fouls.push(Foul::CueBallOffTable);
        }
        for &b in off_table.iter().filter(|b| b.is_object()) {
            fouls.push(Foul::ObjectBallOffTable(b));
        }

        let mut game_over = None;
        let mut group_assigned = None;
        let mut spotted = Vec::new();
        let eight_pocket = pocketed.iter().find(|(b, _)| *b == BallId::EIGHT).map(|(_, p)| *p);
        let eight_off = off_table.contains(&BallId::EIGHT);

        if was_break {
            let rails_hit: std::collections::BTreeSet<BallId> = events
                .iter()
                .filter_map(|e| match *e {
                    Event::Cushion { ball, .. } if ball.is_object() => Some(ball),
                    _ => None,
                })
                .collect();
            if first_contact.is_none() || (object_pocketed.is_empty() && rails_hit.len() < 4) {
                fouls.push(Foul::IllegalBreak);
            }
            if eight_pocket.is_some() || eight_off {
                spotted.push(BallId::EIGHT);
            }
        } else {
            match first_contact {
                None => fouls.push(Foul::NoContact),
                Some(hit) => {
                    let legal = if on_eight {
                        hit == BallId::EIGHT
                    } else {
                        match self.groups[shooter] {
                            Some(g) => g.contains(hit),
                            None => hit != BallId::EIGHT,
                        }
                    };
                    if !legal {
                        fouls.push(Foul::WrongBallFirst(hit));
                    }
                    if pocketed.is_empty() && !rail_after_contact {
                        fouls.push(Foul::NoRailAfterContact);
                    }
                }
            }
            let opponent = 1 - shooter;
            if eight_off {
                game_over =
                    Some(GameOver { winner: opponent, reason: GameOverReason::EightOffTable });
            } else if let Some(pocket) = eight_pocket {
                let reason = if !on_eight {
                    Some(GameOverReason::EightPocketedEarly)
                } else if !fouls.is_empty() {
                    Some(GameOverReason::EightPocketedOnFoul)
                } else if pending.called_pocket != Some(pocket) {
                    Some(GameOverReason::EightInWrongPocket)
                } else {
                    None
                };
                game_over = Some(match reason {
                    Some(reason) => GameOver { winner: opponent, reason },
                    None => GameOver { winner: shooter, reason: GameOverReason::PocketedEight },
                });
            }
            if self.groups[shooter].is_none()
                && fouls.is_empty()
                && let Some(g) = object_pocketed.iter().find_map(|b| Group::of(*b))
            {
                self.groups[shooter] = Some(g);
                self.groups[opponent] = Some(g.opposite());
                group_assigned = Some(g);
            }
        }
        for &b in off_table.iter().filter(|b| b.is_object() && **b != BallId::EIGHT) {
            spotted.push(b);
        }

        let continues = game_over.is_none()
            && fouls.is_empty()
            && if was_break {
                !object_pocketed.is_empty()
            } else {
                match self.groups[shooter] {
                    Some(g) => object_pocketed.iter().any(|b| g.contains(*b)),
                    None => false,
                }
            };

        // Commit the new ball positions.
        self.balls = sim.balls.clone();
        self.balls.sort_by_key(|b| b.id);
        for &b in &spotted {
            self.spot_ball(b);
        }
        self.shots_taken += 1;
        self.is_break = false;
        self.game_over = game_over;
        self.ball_in_hand = None;
        if game_over.is_none() {
            if !fouls.is_empty() {
                self.ball_in_hand =
                    Some(if was_break { BallInHand::Kitchen } else { BallInHand::Anywhere });
                if !self.ball(BallId::CUE).in_play() {
                    self.respot_cue_ball();
                }
            }
            if !continues {
                self.current = 1 - shooter;
            }
        }

        ShotReport {
            shooter,
            was_break,
            first_contact,
            pocketed,
            fouls,
            spotted,
            group_assigned,
            continues,
            game_over,
        }
    }

    /// `player` concedes.
    pub fn resign(&mut self, player: usize) {
        self.end_with(1 - player, GameOverReason::Resignation);
    }

    /// `player` abandoned an online game.
    pub fn abandon(&mut self, player: usize) {
        self.end_with(1 - player, GameOverReason::Abandoned);
    }

    /// Ends the game with a result decided elsewhere (a client applying the
    /// server's verdict). Does nothing if the game is already over.
    pub fn conclude(&mut self, winner: usize, reason: GameOverReason) {
        self.end_with(winner % 2, reason);
    }

    fn end_with(&mut self, winner: usize, reason: GameOverReason) {
        if self.game_over.is_none() {
            self.game_over = Some(GameOver { winner, reason });
        }
    }

    /// The shot clock ran out: the turn passes and the opponent gets ball in hand.
    pub fn time_foul(&mut self) {
        if self.game_over.is_some() {
            return;
        }
        if !self.ball(BallId::CUE).in_play() {
            self.respot_cue_ball();
        }
        self.ball_in_hand =
            Some(if self.is_break { BallInHand::Kitchen } else { BallInHand::Anywhere });
        self.current = 1 - self.current;
    }

    fn is_spot_free(&self, p: DVec2, ignore: BallId) -> bool {
        let min_dist = 2.0 * self.table.spec.ball_radius + 1e-4;
        self.balls
            .iter()
            .filter(|b| b.id != ignore && b.in_play())
            .all(|b| b.xy().distance(p) >= min_dist)
    }

    /// Places a ball on the foot spot, or the nearest free point on the long
    /// string towards the foot rail (then towards the head rail).
    fn spot_ball(&mut self, id: BallId) {
        let r = self.table.spec.ball_radius;
        let start = self.table.foot_spot();
        let limit = self.table.spec.length / 2.0 - r;
        let step = r / 4.0;
        let mut candidates = (0..).map(|i| start.x + i as f64 * step).take_while(|x| *x <= limit);
        let mut pos = candidates.find(|x| self.is_spot_free(DVec2::new(*x, 0.0), id));
        if pos.is_none() {
            pos = (0..)
                .map(|i| start.x - i as f64 * step)
                .take_while(|x| *x >= -limit)
                .find(|x| self.is_spot_free(DVec2::new(*x, 0.0), id));
        }
        let x = pos.unwrap_or(start.x);
        self.balls[id.index()] = Ball::at_rest(id, DVec2::new(x, 0.0), r);
    }

    fn respot_cue_ball(&mut self) {
        let r = self.table.spec.ball_radius;
        let head = self.table.head_spot();
        let mut p = head;
        'search: for ring in 0..40 {
            for k in 0..16 {
                let angle = k as f64 / 16.0 * std::f64::consts::TAU;
                let cand = head + DVec2::from_angle(angle) * (ring as f64 * r);
                if self.table.is_inside_playing_area(cand)
                    && cand.x <= self.table.head_string_x()
                    && self.is_spot_free(cand, BallId::CUE)
                {
                    p = cand;
                    break 'search;
                }
            }
        }
        self.balls[0] = Ball::at_rest(BallId::CUE, p, r);
    }
}

/// Builds a standard eight-ball rack: apex on the foot spot, the 8 in the
/// centre, one solid and one stripe on the back corners, the rest shuffled.
pub fn rack(table: &Table, seed: u64) -> Vec<Ball> {
    let r = table.spec.ball_radius;
    let gap = 1e-4;
    let spacing = 2.0 * r + gap;
    let row_dx = spacing * (3f64).sqrt() / 2.0;
    let apex = table.foot_spot();

    let mut rng = Rng::new(seed);
    let mut solids: Vec<u8> = (1..=7).collect();
    let mut stripes: Vec<u8> = (9..=15).collect();
    rng.shuffle(&mut solids);
    rng.shuffle(&mut stripes);
    let (corner_a, corner_b) = if rng.below(2) == 0 {
        (solids.pop().unwrap(), stripes.pop().unwrap())
    } else {
        (stripes.pop().unwrap(), solids.pop().unwrap())
    };
    let mut rest: Vec<u8> = solids.into_iter().chain(stripes).collect();
    rng.shuffle(&mut rest);

    let mut balls = vec![Ball::at_rest(BallId::CUE, table.head_spot(), r)];
    let mut slots = Vec::with_capacity(15);
    for row in 0..5 {
        for k in 0..=row {
            let x = apex.x + row as f64 * row_dx;
            let y = (k as f64 - row as f64 / 2.0) * spacing;
            slots.push((row, k, DVec2::new(x, y)));
        }
    }
    let mut rest = rest.into_iter();
    for (row, k, p) in slots {
        let number = match (row, k) {
            (2, 1) => 8,
            (4, 0) => corner_a,
            (4, 4) => corner_b,
            _ => rest.next().expect("fifteen balls"),
        };
        balls.push(Ball::at_rest(BallId(number), p, r));
    }
    balls.sort_by_key(|b| b.id);
    debug_assert!(balls.iter().all(|b| b.status == BallStatus::InPlay));
    balls
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ball::BallStatus;

    /// Puts the table into an arbitrary layout (test helper).
    fn layout(game: &mut EightBallGame, positions: &[(u8, DVec2)]) {
        let r = game.table.spec.ball_radius;
        for b in &mut game.balls {
            b.status = BallStatus::Pocketed(PocketId::FootLeft);
        }
        for &(n, p) in positions {
            game.balls[n as usize] = Ball::at_rest(BallId(n), p, r);
        }
    }

    fn straight_shot(game: &EightBallGame, target: BallId, pocket: PocketId, speed: f64) -> Shot {
        let r = game.table.spec.ball_radius;
        let aim = game.table.pocket(pocket).aim_point();
        let t = game.ball(target).xy();
        let ghost = t - (aim - t).normalize() * 2.0 * r;
        let d = ghost - game.ball(BallId::CUE).xy();
        Shot::new(d.y.atan2(d.x), speed)
    }

    fn after_break(seed: u64) -> EightBallGame {
        let mut g = EightBallGame::new(seed, 0);
        g.is_break = false;
        g.ball_in_hand = None;
        g
    }

    #[test]
    fn rack_has_sixteen_non_overlapping_balls_with_eight_in_centre() {
        let t = Table::default();
        let balls = rack(&t, 7);
        assert_eq!(balls.len(), 16);
        for (i, a) in balls.iter().enumerate() {
            assert_eq!(a.id.index(), i);
            for b in &balls[i + 1..] {
                assert!(a.xy().distance(b.xy()) >= 2.0 * t.spec.ball_radius);
            }
        }
        let eight = balls[8].xy();
        let row_dx = (2.0 * t.spec.ball_radius + 1e-4) * 3f64.sqrt() / 2.0;
        assert!((eight.x - (t.foot_spot().x + 2.0 * row_dx)).abs() < 1e-9);
        assert!(eight.y.abs() < 1e-9);
    }

    #[test]
    fn break_shot_is_handled() {
        let mut g = EightBallGame::new(1, 0);
        assert!(g.is_break());
        let report = g.play_shot(&Shot::new(0.0, 7.0), None).unwrap();
        assert!(report.was_break);
        assert!(!g.is_break());
        assert!(report.first_contact.is_some());
        assert!(g.balls().iter().all(|b| b.status != BallStatus::OffTable));
    }

    #[test]
    fn soft_break_is_illegal() {
        let mut g = EightBallGame::new(1, 0);
        let report = g.play_shot(&Shot::new(0.0, 0.6), None).unwrap();
        assert!(report.fouls.contains(&Foul::IllegalBreak));
        assert_eq!(g.current_player(), 1);
        assert_eq!(g.ball_in_hand(), Some(BallInHand::Kitchen));
    }

    #[test]
    fn pocketing_assigns_groups_and_keeps_turn() {
        let mut g = after_break(3);
        layout(
            &mut g,
            &[
                (0, DVec2::new(-0.2, 0.0)),
                (3, DVec2::new(0.3, 0.2)),
                (12, DVec2::new(0.0, -0.3)),
                (8, DVec2::new(-0.6, -0.3)),
            ],
        );
        let shot = straight_shot(&g, BallId(3), PocketId::FootLeft, 2.0);
        let report = g.play_shot(&shot, None).unwrap();
        assert_eq!(report.fouls, vec![]);
        assert_eq!(report.group_assigned, Some(Group::Solids));
        assert!(report.continues);
        assert_eq!(g.group_of(0), Some(Group::Solids));
        assert_eq!(g.group_of(1), Some(Group::Stripes));
        assert_eq!(g.current_player(), 0);
    }

    #[test]
    fn hitting_opponent_ball_first_is_a_foul() {
        let mut g = after_break(3);
        g.groups = [Some(Group::Stripes), Some(Group::Solids)];
        layout(
            &mut g,
            &[
                (0, DVec2::new(-0.2, 0.0)),
                (3, DVec2::new(0.3, 0.2)),
                (12, DVec2::new(0.0, -0.3)),
                (8, DVec2::new(-0.6, -0.3)),
            ],
        );
        let shot = straight_shot(&g, BallId(3), PocketId::FootLeft, 2.0);
        let report = g.play_shot(&shot, None).unwrap();
        assert!(report.fouls.contains(&Foul::WrongBallFirst(BallId(3))));
        assert_eq!(g.current_player(), 1);
        assert_eq!(g.ball_in_hand(), Some(BallInHand::Anywhere));
    }

    #[test]
    fn scratch_gives_ball_in_hand_and_respots_cue_ball() {
        let mut g = after_break(3);
        g.groups = [Some(Group::Solids), Some(Group::Stripes)];
        let t = g.table.clone();
        let aim = t.pocket(PocketId::FootLeft).aim_point();
        let start = aim - DVec2::new(1.0, 0.5).normalize() * 0.6;
        layout(
            &mut g,
            &[
                (0, start),
                (2, DVec2::new(-0.5, 0.0)),
                (10, DVec2::new(-0.5, 0.3)),
                (8, DVec2::new(-0.6, -0.3)),
            ],
        );
        let d = aim - start;
        let report = g.play_shot(&Shot::new(d.y.atan2(d.x), 2.0), None).unwrap();
        assert!(report.fouls.contains(&Foul::CueBallPocketed));
        assert!(g.ball(BallId::CUE).in_play());
        assert_eq!(g.current_player(), 1);
        assert_eq!(g.ball_in_hand(), Some(BallInHand::Anywhere));
    }

    #[test]
    fn eight_requires_called_pocket_and_wins() {
        let mut g = after_break(3);
        g.groups = [Some(Group::Solids), Some(Group::Stripes)];
        layout(
            &mut g,
            &[(0, DVec2::new(-0.2, 0.0)), (8, DVec2::new(0.3, 0.2)), (12, DVec2::new(0.0, -0.3))],
        );
        let shot = straight_shot(&g, BallId::EIGHT, PocketId::FootLeft, 2.0);
        assert_eq!(g.play_shot(&shot, None), Err(ShotError::MustCallPocket));
        let report = g.play_shot(&shot, Some(PocketId::FootLeft)).unwrap();
        assert_eq!(
            report.game_over,
            Some(GameOver { winner: 0, reason: GameOverReason::PocketedEight })
        );
    }

    #[test]
    fn eight_in_wrong_pocket_loses() {
        let mut g = after_break(3);
        g.groups = [Some(Group::Solids), Some(Group::Stripes)];
        layout(
            &mut g,
            &[(0, DVec2::new(-0.2, 0.0)), (8, DVec2::new(0.3, 0.2)), (12, DVec2::new(0.0, -0.3))],
        );
        let shot = straight_shot(&g, BallId::EIGHT, PocketId::FootLeft, 2.0);
        let report = g.play_shot(&shot, Some(PocketId::FootRight)).unwrap();
        assert_eq!(
            report.game_over,
            Some(GameOver { winner: 1, reason: GameOverReason::EightInWrongPocket })
        );
    }

    #[test]
    fn early_eight_loses() {
        let mut g = after_break(3);
        g.groups = [Some(Group::Solids), Some(Group::Stripes)];
        layout(
            &mut g,
            &[
                (0, DVec2::new(-0.2, 0.0)),
                (8, DVec2::new(0.3, 0.2)),
                (2, DVec2::new(-0.6, 0.3)),
                (12, DVec2::new(0.0, -0.3)),
            ],
        );
        let shot = straight_shot(&g, BallId::EIGHT, PocketId::FootLeft, 2.0);
        let report = g.play_shot(&shot, None).unwrap();
        assert_eq!(report.game_over.map(|o| o.winner), Some(1));
    }

    #[test]
    fn ball_in_hand_placement_rules() {
        let mut g = EightBallGame::new(5, 0);
        assert!(g.place_cue_ball(DVec2::new(-0.6, 0.2)));
        assert!(!g.place_cue_ball(DVec2::new(0.0, 0.0)), "kitchen only on the break");
        assert!(!g.place_cue_ball(DVec2::new(-2.0, 0.0)), "outside the table");
        g.ball_in_hand = Some(BallInHand::Anywhere);
        assert!(g.place_cue_ball(DVec2::new(0.0, 0.3)));
        let eight = g.ball(BallId::EIGHT).xy();
        assert!(!g.place_cue_ball(eight), "overlapping a ball");
    }

    #[test]
    fn time_foul_passes_turn_with_ball_in_hand() {
        let mut g = after_break(4);
        g.time_foul();
        assert_eq!(g.current_player(), 1);
        assert_eq!(g.ball_in_hand(), Some(BallInHand::Anywhere));
        g.abandon(1);
        assert_eq!(g.game_over(), Some(GameOver { winner: 0, reason: GameOverReason::Abandoned }));
        g.time_foul();
        assert_eq!(g.current_player(), 1, "no changes after the game ended");
    }

    #[test]
    fn game_state_round_trips_through_json() {
        let mut g = EightBallGame::new(9, 1);
        g.play_shot(&Shot::new(0.0, 7.0), None).unwrap();
        let json = serde_json::to_string(&g).unwrap();
        let back: EightBallGame = serde_json::from_str(&json).unwrap();
        assert_eq!(g, back);
    }

    #[test]
    fn spotting_finds_free_place() {
        let mut g = after_break(9);
        g.spot_ball(BallId(5));
        let p = g.ball(BallId(5)).xy();
        assert!(g.is_spot_free(p, BallId(5)));
        assert_eq!(p.y, 0.0, "spotted balls sit on the long string");
        assert!(g.table.is_inside_playing_area(p));
    }

    #[test]
    fn full_random_game_terminates() {
        // Plays a game with simple straight-in attempts to make sure the state
        // machine never gets stuck.
        let mut g = EightBallGame::new(11, 0);
        g.play_shot(&Shot::new(0.0, 7.0), None).unwrap();
        let mut rng = Rng::new(5);
        for _ in 0..400 {
            if g.game_over().is_some() {
                break;
            }
            if g.ball_in_hand().is_some() {
                for _ in 0..200 {
                    let p = DVec2::new(rng.next_f64() * 1.8 - 0.9, rng.next_f64() * 0.8 - 0.4);
                    if g.place_cue_ball(p) {
                        break;
                    }
                }
            }
            let target = g.legal_targets()[0];
            let pocket = PocketId::ALL[rng.below(6)];
            let shot = straight_shot(&g, target, pocket, 1.0 + rng.next_f64() * 3.0);
            let called = g.needs_called_pocket().then_some(pocket);
            g.play_shot(&shot, called).unwrap();
        }
        assert!(g.game_over().is_some(), "game should finish");
    }
}
