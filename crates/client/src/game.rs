//! Match flow: turns, shot playback, the computer opponent, online play
//! (with the shooter's shot predicted locally) and the replay of the last shot.

use std::sync::{Arc, Mutex};

use ::glam::DVec2;
use snook_core::ai::{self, AiDecision};
use snook_core::{
    BallId, Difficulty, EightBallGame, Event, GameOverReason, PendingShot, PocketId, Shot,
    ShotError, ShotReport, Simulation,
};

use snook_proto::{
    AimState, ChatKind, ChatLine, ClientMsg, MatchResult, PlayerInfo, SPECTATOR_SEAT, ServerMsg,
};

use crate::config::AimGuide;
use crate::i18n::{t, tw};
use crate::ui::{FeedKind, FeedLine};
use crate::{MatchMode, MatchSettings};

/// Who controls a seat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Controller {
    Human,
    Computer(Difficulty),
    /// The opponent in an online match.
    Remote,
}

/// The player's cue settings.
#[derive(Debug, Clone, Copy)]
pub struct Aim {
    pub yaw: f64,
    pub elevation: f64,
    pub tip: DVec2,
    /// 0..=1, mapped to cue speed.
    pub power: f32,
    pub charging: bool,
}

impl Default for Aim {
    fn default() -> Self {
        Aim { yaw: 0.0, elevation: 0.0, tip: DVec2::ZERO, power: 0.0, charging: false }
    }
}

/// A computer move being computed on a worker thread, then acted out.
pub struct ComputerTurn {
    result: Arc<Mutex<Option<AiDecision>>>,
    decision: Option<AiDecision>,
    elapsed: f32,
    start_yaw: f64,
    /// How long the computer "studies the table" before moving the cue.
    think: f32,
    waited: f32,
    /// A small first aiming error, corrected before the stroke (like a person).
    first_miss: f64,
}

pub enum Phase {
    /// The current player is lining up a shot.
    Turn {
        placing_cue_ball: bool,
        computer: Option<ComputerTurn>,
    },
    /// Balls are rolling.
    Simulating(Box<PendingShot>),
    /// Online: the animation is over and the server's verdict is on its way
    /// (the balls stay where the animation left them).
    Settling(Box<PendingShot>),
    /// Short pause after a shot so the result can be read.
    Pause(f32),
    GameOver,
}

/// The last shot, kept so it can be watched again.
#[derive(Clone)]
pub struct ReplayShot {
    pub start: EightBallGame,
    pub shot: Shot,
    pub called: Option<PocketId>,
}

/// A replay being shown (the real game is untouched).
pub struct Replay {
    pub simulation: Simulation,
    /// Time since the replay ended (it closes itself shortly after).
    pub done: f32,
    events_seen: usize,
}

pub struct Notice {
    pub text: String,
    pub important: bool,
    pub ttl: f32,
}

pub struct Match {
    pub game: EightBallGame,
    pub names: [String; 2],
    pub controllers: [Controller; 2],
    pub phase: Phase,
    pub aim: Aim,
    pub called_pocket: Option<PocketId>,
    pub notices: Vec<Notice>,
    /// Everything that happened this game, newest last (shown in the HUD).
    pub log: Vec<(String, bool)>,
    pub last_report: Option<ShotReport>,
    pub guide: AimGuide,
    pub games_played: u32,
    pub wins: [u32; 2],
    /// Collisions to play as sounds (drained by the audio module).
    pub sounds: Vec<ImpactSound>,
    /// Set for matches played through a server.
    pub online: Option<OnlineMatch>,
    /// Offline shot clock: (seconds per shot, seconds left).
    pub clock: Option<(f32, f32)>,
    pub last_shot: Option<ReplayShot>,
    pub replay: Option<Replay>,
    seed: u64,
    events_seen: usize,
    demo_timer: f32,
}

/// State of a match played on a server.
pub struct OnlineMatch {
    pub seat: usize,
    /// Whether the game is rated (kept with the match state).
    #[allow(dead_code)]
    pub rated: bool,
    pub ratings: [i32; 2],
    pub countries: [String; 2],
    pub tournament: Option<u64>,
    pub shot_clock: f32,
    /// Seconds left for the current shot (counts down during a turn).
    pub clock_left: f32,
    /// Messages for the server (drained by the network code).
    pub outbox: Vec<ClientMsg>,
    pub result: Option<MatchResult>,
    pub opponent_connected: bool,
    pub opponent_left: bool,
    pub rematch_offered: bool,
    pub rematch_requested: bool,
    /// Chat with the opponent and what happened in the game, in order.
    pub feed: Vec<FeedLine>,
    /// Chat lines the player has not looked at.
    pub unread: usize,
    /// The newest unread line and how long ago it came (a preview by the
    /// closed chat for a few seconds).
    pub preview: Option<(FeedLine, f32)>,
    /// Achievements of this player (they unlock reactions).
    pub unlocked: Vec<String>,
    /// Reactions on screen: who sent which (a seat, or [`SPECTATOR_SEAT`]),
    /// and for how long it has shown.
    pub reactions: Vec<(usize, &'static str, f32)>,
    /// Watching a friend's game (`seat` is the friend's).
    pub spectator: bool,
    /// How many people are watching.
    pub spectators: u32,
    /// The opponent has sent their aim this turn (until then their cue is hidden).
    pub remote_aimed: bool,
    /// The opponent's latest aim; the cue glides towards it.
    remote_target: Option<AimState>,
    /// The turn being animated (or awaited).
    sim_turn: Option<u32>,
    /// This player's own shot, played before the server confirmed it.
    predicted: Option<(u32, Shot)>,
    /// The table before the predicted shot (to roll back if refused).
    turn_start: Option<EightBallGame>,
    /// The server's result for the shot being animated.
    pending_end: Option<(EightBallGame, ShotReport)>,
    /// The match ended while a shot was still being shown.
    ended_pending: bool,
    aim_timer: f32,
    last_aim: Option<AimState>,
    last_cue: Option<DVec2>,
    /// Shot clock for the next turn, from the server.
    next_clock: f32,
}

/// A sound-worthy collision during playback.
#[derive(Debug, Clone, Copy)]
pub struct ImpactSound {
    pub kind: ImpactKind,
    /// Impact speed in m/s.
    pub speed: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImpactKind {
    Cue,
    Ball,
    Cushion,
    Pocket,
}

impl Match {
    pub fn new(settings: &MatchSettings, seed: u64) -> Self {
        let level_name = |d| tw("Computer ({level})", &[("level", &crate::i18n::difficulty(d))]);
        let (names, controllers) = match settings.mode {
            MatchMode::VsComputer(d) => (
                [settings.names[0].clone(), level_name(d)],
                [Controller::Human, Controller::Computer(d)],
            ),
            MatchMode::HotSeat => (settings.names.clone(), [Controller::Human, Controller::Human]),
            MatchMode::Demo => (
                [level_name(Difficulty::Hard), level_name(Difficulty::Medium)],
                [Controller::Computer(Difficulty::Hard), Controller::Computer(Difficulty::Medium)],
            ),
        };
        let mut m = Match {
            game: EightBallGame::new(seed, 0),
            names,
            controllers,
            phase: Phase::GameOver,
            aim: Aim::default(),
            called_pocket: None,
            notices: Vec::new(),
            log: Vec::new(),
            last_report: None,
            guide: settings.guide,
            games_played: 0,
            wins: [0, 0],
            sounds: Vec::new(),
            online: None,
            clock: settings.shot_clock.map(|s| (s as f32, s as f32)),
            last_shot: None,
            replay: None,
            seed,
            events_seen: 0,
            demo_timer: 0.0,
        };
        m.begin_turn();
        m
    }

    /// A match on the server, from its `MatchStarted` message.
    #[allow(clippy::too_many_arguments)]
    pub fn online(
        match_id: u64,
        seat: usize,
        players: &[PlayerInfo; 2],
        rated: bool,
        game: EightBallGame,
        shot_clock_secs: u32,
        clock_left: f32,
        tournament: Option<u64>,
        guide: AimGuide,
        spectating: bool,
    ) -> Self {
        let mut controllers = [Controller::Remote; 2];
        // Development aid: SNOOK_AUTOPLAY=1 lets the computer play this seat
        // (two such games can play a whole online match by themselves).
        // A spectator only watches: both seats are played from the server.
        if !spectating {
            controllers[seat] = if crate::dev::flag("SNOOK_AUTOPLAY") {
                Controller::Computer(Difficulty::Hard)
            } else {
                Controller::Human
            };
        }
        let mut m = Match {
            game,
            names: [players[0].username.clone(), players[1].username.clone()],
            controllers,
            phase: Phase::GameOver,
            aim: Aim::default(),
            called_pocket: None,
            notices: Vec::new(),
            log: Vec::new(),
            last_report: None,
            // No full guide against people.
            guide: guide.online(),
            games_played: 0,
            wins: [0, 0],
            sounds: Vec::new(),
            online: Some(OnlineMatch {
                seat,
                rated,
                ratings: [players[0].rating, players[1].rating],
                countries: [players[0].country.clone(), players[1].country.clone()],
                tournament,
                shot_clock: shot_clock_secs as f32,
                clock_left,
                outbox: Vec::new(),
                result: None,
                opponent_connected: true,
                opponent_left: false,
                rematch_offered: false,
                rematch_requested: false,
                feed: Vec::new(),
                unread: 0,
                preview: None,
                unlocked: Vec::new(),
                reactions: Vec::new(),
                spectator: spectating,
                spectators: 0,
                remote_aimed: false,
                remote_target: None,
                sim_turn: None,
                predicted: None,
                turn_start: None,
                pending_end: None,
                ended_pending: false,
                aim_timer: 0.0,
                last_aim: None,
                last_cue: None,
                next_clock: clock_left,
            }),
            clock: None,
            last_shot: None,
            replay: None,
            seed: match_id,
            events_seen: 0,
            demo_timer: 0.0,
        };
        m.begin_turn();
        if let Some(o) = m.online.as_mut() {
            o.clock_left = clock_left;
        }
        m
    }

    /// Starts the next online game with the same opponent (a rematch, or the
    /// same game after reconnecting), keeping the score and the chat.
    pub fn continue_online(&mut self, next: Match, same_game: bool) {
        let wins = self.wins;
        let games = self.games_played;
        let feed = self.online.as_ref().map(|o| o.feed.clone()).unwrap_or_default();
        let log = std::mem::take(&mut self.log);
        let guide = self.guide;
        let swapped = next.names[0] != self.names[0];
        *self = next;
        self.wins = if swapped { [wins[1], wins[0]] } else { wins };
        self.games_played = games;
        if same_game {
            self.log = log;
        }
        self.guide = guide;
        if let Some(o) = self.online.as_mut() {
            o.feed = feed;
        }
    }

    pub fn is_online(&self) -> bool {
        self.online.is_some()
    }

    /// The server's id of an online match.
    pub fn match_id(&self) -> u64 {
        self.seed
    }

    /// Starts a new game; the players alternate the break. Online, this asks
    /// the opponent for a rematch.
    pub fn rematch(&mut self) {
        if let Some(o) = self.online.as_mut() {
            if !o.rematch_requested && !o.opponent_left && o.tournament.is_none() {
                o.rematch_requested = true;
                o.outbox.push(ClientMsg::Rematch);
            }
            return;
        }
        self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let breaker = (self.games_played % 2) as usize;
        self.game = EightBallGame::new(self.seed, breaker);
        self.last_report = None;
        self.last_shot = None;
        self.replay = None;
        self.notices.clear();
        self.log.clear();
        self.begin_turn();
    }

    pub fn current_controller(&self) -> Controller {
        self.controllers[self.game.current_player()]
    }

    pub fn is_human_turn(&self) -> bool {
        matches!(self.phase, Phase::Turn { .. })
            && self.current_controller() == Controller::Human
            && self.replay.is_none()
    }

    pub fn is_placing_cue_ball(&self) -> bool {
        matches!(self.phase, Phase::Turn { placing_cue_ball: true, .. })
    }

    pub fn set_placing(&mut self, placing: bool) {
        if let Phase::Turn { placing_cue_ball, .. } = &mut self.phase {
            *placing_cue_ball = placing && self.game.ball_in_hand().is_some();
        }
    }

    /// Balls as currently displayed (a replay, mid-shot or at rest).
    pub fn balls(&self) -> &[snook_core::Ball] {
        if let Some(r) = &self.replay {
            return &r.simulation.balls;
        }
        match &self.phase {
            Phase::Simulating(p) | Phase::Settling(p) => &p.simulation.balls,
            _ => self.game.balls(),
        }
    }

    /// True while balls are moving or the result is not in yet.
    pub fn shot_in_progress(&self) -> bool {
        matches!(self.phase, Phase::Simulating(_) | Phase::Settling(_))
    }

    pub fn notify(&mut self, text: impl Into<String>, important: bool) {
        self.announce(text.into(), important, 4.0);
    }

    fn announce(&mut self, text: String, important: bool, ttl: f32) {
        announce_into(&mut self.notices, &mut self.log, self.online.as_mut(), text, important, ttl);
    }

    /// The shot clock for the current turn: (seconds left, seconds per shot).
    pub fn shot_clock(&self) -> Option<(f32, f32)> {
        if let Some(o) = &self.online {
            return Some((o.clock_left, o.shot_clock));
        }
        self.clock.map(|(total, left)| (left, total))
    }

    /// Watches the last shot again (not while a shot is being played).
    pub fn start_replay(&mut self) {
        if self.shot_in_progress() {
            return;
        }
        let Some(last) = &self.last_shot else { return };
        if let Ok(pending) = last.start.prepare_shot(&last.shot, last.called) {
            self.sounds.push(ImpactSound { kind: ImpactKind::Cue, speed: last.shot.speed as f32 });
            self.replay =
                Some(Replay { simulation: pending.simulation, done: 0.0, events_seen: 0 });
        }
    }

    pub fn stop_replay(&mut self) {
        self.replay = None;
    }

    fn begin_turn(&mut self) {
        let placing = self.game.ball_in_hand().is_some();
        let computer = match self.current_controller() {
            Controller::Computer(level) => {
                let result = Arc::new(Mutex::new(None));
                let slot = result.clone();
                let game = self.game.clone();
                let seed = self.seed ^ ((self.game.shots_taken() as u64) << 32);
                std::thread::spawn(move || {
                    // Never leave the computer "thinking" forever: if the search
                    // fails, play a plain shot at a legal ball.
                    let decision = std::panic::catch_unwind(|| ai::decide(&game, level, seed))
                        .unwrap_or_else(|_| fallback_decision(&game));
                    *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(decision);
                });
                // Pseudo-random pacing from the seed: 1.0–2.8 s of thinking.
                let r = |k: u64| {
                    ((seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> k) & 0xffff) as f32 / 65535.0
                };
                Some(ComputerTurn {
                    result,
                    decision: None,
                    elapsed: 0.0,
                    start_yaw: 0.0,
                    think: 1.0 + 1.8 * r(3),
                    waited: 0.0,
                    first_miss: ((r(19) - 0.5) * 6.0).to_radians() as f64,
                })
            }
            Controller::Human | Controller::Remote => None,
        };
        if let Some(o) = self.online.as_mut() {
            o.clock_left = o.next_clock.min(o.shot_clock);
            o.next_clock = o.shot_clock;
            o.last_aim = None;
            o.last_cue = None;
            o.remote_aimed = false;
            o.remote_target = None;
            o.sim_turn = None;
            o.predicted = None;
            o.turn_start = None;
        }
        if let Some((total, left)) = &mut self.clock {
            *left = *total;
        }
        let computer_or_remote =
            computer.is_some() || self.current_controller() == Controller::Remote;
        self.aim = Aim { yaw: self.game.default_aim(), ..Aim::default() };
        // A pocket called for the 8 in an earlier visit still stands.
        self.called_pocket = self.game.called_pocket_of(self.game.current_player());
        self.phase = Phase::Turn { placing_cue_ball: placing && !computer_or_remote, computer };
    }

    pub fn shot_from_aim(&self) -> Shot {
        let max = self.game.params().max_cue_speed;
        Shot {
            yaw: self.aim.yaw,
            elevation: self.aim.elevation,
            tip: self.aim.tip,
            speed: (self.aim.power as f64).powf(1.3) * max,
        }
    }

    /// Strikes the cue ball with the current aim. Returns false (with a notice) if
    /// the shot is not allowed yet. Online, the shot plays at once and the
    /// server's verdict is applied when the balls stop.
    pub fn shoot(&mut self) -> bool {
        if !matches!(self.phase, Phase::Turn { .. }) {
            return false;
        }
        self.replay = None;
        let shot = self.shot_from_aim();
        match self.game.prepare_shot(&shot, self.called_pocket) {
            Ok(pending) => {
                if let Some(o) = self.online.as_mut() {
                    let turn = self.game.shots_taken();
                    let cue_ball =
                        self.game.ball_in_hand().map(|_| self.game.ball(BallId::CUE).xy());
                    o.outbox.push(ClientMsg::Shoot {
                        turn,
                        shot,
                        called_pocket: self.called_pocket,
                        cue_ball,
                    });
                    o.predicted = Some((turn, shot));
                    o.sim_turn = Some(turn);
                    o.turn_start = Some(self.game.clone());
                    o.pending_end = None;
                }
                self.start_animation(pending, shot);
                true
            }
            Err(e) => {
                self.shot_error(e);
                false
            }
        }
    }

    fn start_animation(&mut self, pending: PendingShot, shot: Shot) {
        self.last_shot =
            Some(ReplayShot { start: self.game.clone(), shot, called: pending.called_pocket });
        self.replay = None;
        self.sounds.push(ImpactSound { kind: ImpactKind::Cue, speed: shot.speed as f32 });
        self.events_seen = 0;
        self.aim.charging = false;
        self.phase = Phase::Simulating(Box::new(pending));
    }

    fn shot_error(&mut self, e: ShotError) {
        match e {
            ShotError::MustCallPocket => {
                self.notify(t("Call a pocket for the 8-ball first: click a pocket."), true)
            }
            e => self.notify(tw("Can't shoot: {e}.", &[("e", &crate::i18n::shot_error(e))]), true),
        }
    }

    fn report_shot(&mut self, report: &ShotReport) {
        let shooter = self.names[report.shooter].clone();
        for foul in &report.fouls {
            self.notify(
                tw(
                    "Foul by {shooter}: {foul}.",
                    &[("shooter", &shooter), ("foul", &crate::i18n::foul(foul))],
                ),
                true,
            );
        }
        if let Some(g) = report.group_assigned {
            self.notify(
                tw(
                    "{shooter} takes {group}.",
                    &[("shooter", &shooter), ("group", &crate::i18n::group(g))],
                ),
                false,
            );
        }
        let potted: Vec<String> = report
            .pocketed
            .iter()
            .filter(|(b, _)| b.is_object())
            .map(|(b, _)| crate::i18n::ball(*b))
            .collect();
        if !potted.is_empty() {
            self.notify(
                tw(
                    "{shooter} pocketed {balls}.",
                    &[("shooter", &shooter), ("balls", &potted.join(", "))],
                ),
                false,
            );
        }
        for b in &report.spotted {
            self.notify(tw("The {ball} was spotted.", &[("ball", &crate::i18n::ball(*b))]), false);
        }
        if let Some(over) = report.game_over {
            let winner = self.names[over.winner].clone();
            let how = crate::i18n::result_line(over.reason, &winner, &self.names[1 - over.winner]);
            self.notify(tw("{winner} wins! ({how})", &[("winner", &winner), ("how", &how)]), true);
        } else if !report.continues {
            let next = self.names[self.game.current_player()].clone();
            if let Some(bih) = self.game.ball_in_hand() {
                let where_ = match bih {
                    snook_core::BallInHand::Kitchen => t(" behind the head string"),
                    snook_core::BallInHand::Anywhere => "",
                };
                self.notify(
                    tw("{next} has ball in hand{where}.", &[("next", &next), ("where", &where_)]),
                    false,
                );
            }
        }
    }
}

/// The computer's pacing, so its turns read like a person's: turn the cue
/// towards the shot, refine the aim, pause, then draw the cue back and shoot.
const AI_AIM_TIME: f32 = 1.3;
const AI_REFINE_TIME: f32 = 0.9;
const AI_SETTLE_TIME: f32 = 0.5;
const AI_STROKE_TIME: f32 = 1.1;

impl Match {
    /// Advances the match by `dt` seconds of real time.
    pub fn update(&mut self, dt: f32) {
        self.run_online(dt);
        self.run_offline_clock(dt);
        self.run_computer_turn(dt);
        self.run_simulation(dt);
        self.run_replay(dt);
        if let Phase::Pause(t) = &mut self.phase {
            *t -= dt;
            if *t <= 0.0 {
                self.begin_turn();
            }
        }
        for n in &mut self.notices {
            n.ttl -= dt;
        }
        self.notices.retain(|n| n.ttl > 0.0);
        // Computer-vs-computer matches restart by themselves.
        let all_computers = self.controllers.iter().all(|c| matches!(c, Controller::Computer(_)));
        if all_computers && matches!(self.phase, Phase::GameOver) {
            self.demo_timer += dt;
            if self.demo_timer > 4.0 {
                self.demo_timer = 0.0;
                self.rematch();
            }
        }
    }

    /// The current player concedes the game (online: this player).
    pub fn resign(&mut self) {
        if self.game.game_over().is_some() {
            return;
        }
        if let Some(o) = self.online.as_mut() {
            // The server ends the game and tells both players.
            o.outbox.push(ClientMsg::Resign);
            return;
        }
        let player = self.game.current_player();
        self.game.resign(player);
        self.replay = None;
        if let Some(over) = self.game.game_over() {
            self.wins[over.winner] += 1;
            self.games_played += 1;
            let name = self.names[player].clone();
            self.notify(tw("{name} resigned.", &[("name", &name)]), true);
        }
        self.phase = Phase::GameOver;
    }

    /// Offline shot clock (two players, or the human against the computer).
    fn run_offline_clock(&mut self, dt: f32) {
        if self.replay.is_some() {
            return;
        }
        let human = self.current_controller() == Controller::Human;
        let turn = matches!(self.phase, Phase::Turn { .. });
        let Some((_, left)) = &mut self.clock else { return };
        if !(human && turn) {
            return;
        }
        *left -= dt;
        if *left > 0.0 {
            return;
        }
        let who = self.names[self.game.current_player()].clone();
        self.game.time_foul();
        self.notify(
            tw("{who} ran out of time: ball in hand for the opponent.", &[("who", &who)]),
            true,
        );
        self.begin_turn();
    }

    fn run_computer_turn(&mut self, dt: f32) {
        let max_speed = self.game.params().max_cue_speed;
        let current_yaw = self.aim.yaw;
        if self.replay.is_some() {
            return;
        }
        let Phase::Turn { computer: Some(turn), .. } = &mut self.phase else {
            return;
        };
        turn.waited += dt;
        if turn.decision.is_none() {
            if turn.waited < turn.think {
                return;
            }
            let ready = turn.result.lock().unwrap_or_else(|e| e.into_inner()).take();
            match ready {
                Some(decision) => {
                    if let (Some(pocket), true) =
                        (decision.called_pocket, self.game.may_call_pocket())
                    {
                        let name = self.names[self.game.current_player()].clone();
                        let text = tw(
                            "{name} calls the 8-ball in the {pocket} pocket.",
                            &[("name", &name), ("pocket", &crate::i18n::pocket(pocket))],
                        );
                        announce_into(&mut self.notices, &mut self.log, None, text, true, 5.0);
                    }
                    turn.decision = Some(decision);
                    turn.start_yaw = current_yaw;
                    turn.elapsed = 0.0;
                }
                None => return,
            }
        }
        let decision = turn.decision.expect("set above");
        turn.elapsed += dt;
        let t = turn.elapsed;
        let start_yaw = turn.start_yaw;

        if let Some(p) = decision.cue_ball
            && self.game.ball(BallId::CUE).xy() != p
        {
            self.game.place_cue_ball(p);
        }
        let target_power = ((decision.shot.speed / max_speed).powf(1.0 / 1.3)) as f32;
        let first_miss = turn.first_miss;
        let ease = |k: f32| {
            let k = k.clamp(0.0, 1.0);
            (k * k * (3.0 - 2.0 * k)) as f64
        };
        let delta = angle_delta(start_yaw, decision.shot.yaw);
        // Swing the cue near the line, then refine it slowly onto the shot.
        let rough = start_yaw + (delta + first_miss) * ease(t / AI_AIM_TIME);
        let fine = ease((t - AI_AIM_TIME) / AI_REFINE_TIME);
        self.aim.yaw = rough - first_miss * fine;
        self.aim.tip = decision.shot.tip;
        self.aim.elevation = decision.shot.elevation;
        self.called_pocket = decision.called_pocket;
        let stroke_start = AI_AIM_TIME + AI_REFINE_TIME + AI_SETTLE_TIME;
        if t > stroke_start {
            self.aim.charging = true;
            let k = ((t - stroke_start) / AI_STROKE_TIME).clamp(0.0, 1.0);
            self.aim.power = target_power * (k * k * (3.0 - 2.0 * k));
        }
        if t > stroke_start + AI_STROKE_TIME + 0.35 {
            self.aim.yaw = decision.shot.yaw;
            self.aim.power = target_power;
            if !self.shoot() {
                self.called_pocket = Some(PocketId::FootLeft);
                self.shoot();
            }
        }
    }

    fn collect_sounds(events: &[Event], from: usize, out: &mut Vec<ImpactSound>) {
        for e in &events[from.min(events.len())..] {
            let sound = match *e {
                Event::BallBall { speed, .. } => Some((ImpactKind::Ball, speed)),
                Event::Cushion { speed, .. } => Some((ImpactKind::Cushion, speed)),
                Event::Slate { speed, .. } => Some((ImpactKind::Cushion, speed * 0.5)),
                Event::Pocketed { .. } => Some((ImpactKind::Pocket, 1.0)),
                Event::OffTable { .. } => None,
            };
            if let Some((kind, speed)) = sound {
                out.push(ImpactSound { kind, speed: speed as f32 });
            }
        }
    }

    fn run_simulation(&mut self, dt: f32) {
        // Real time, so shots never play in slow motion on slow computers.
        let dt = (dt as f64).min(0.1);
        let events_seen = self.events_seen;
        let Phase::Simulating(pending) = &mut self.phase else {
            return;
        };
        let moving = pending.simulation.advance(dt);
        Self::collect_sounds(pending.simulation.events(), events_seen, &mut self.sounds);
        self.events_seen = pending.simulation.events().len();
        if !moving {
            self.end_animation();
        }
    }

    /// The balls stopped: apply the result (online, the server's).
    fn end_animation(&mut self) {
        let phase = std::mem::replace(&mut self.phase, Phase::Pause(0.7));
        let (Phase::Simulating(pending) | Phase::Settling(pending)) = phase else {
            self.phase = phase;
            return;
        };
        let report = match self.online.as_mut() {
            Some(o) => match o.pending_end.take() {
                Some((end, report)) => {
                    if crate::dev::flag("SNOOK_NET_DEBUG") {
                        let mut local = self.game.clone();
                        local.finish_shot((*pending).clone());
                        let same = local == end;
                        eprintln!(
                            "shot {}: prediction {}",
                            end.shots_taken(),
                            if same { "matched the server" } else { "DIFFERED from the server" }
                        );
                    }
                    // The server's result is authoritative.
                    self.game = end;
                    report
                }
                None => {
                    // Wait for it, showing the balls where they stopped.
                    self.phase = Phase::Settling(pending);
                    return;
                }
            },
            None => self.game.finish_shot(*pending),
        };
        self.report_shot(&report);
        self.after_shot(&report);
    }

    /// Plays any shot still on screen to its end at once (a newer message
    /// from the server needs the table at rest).
    fn fast_forward(&mut self) {
        if let Phase::Simulating(p) = &mut self.phase {
            p.simulation.run_to_rest();
            self.end_animation();
        }
    }

    fn after_shot(&mut self, report: &ShotReport) {
        if let Some(over) = report.game_over {
            self.wins[over.winner] += 1;
            self.games_played += 1;
            self.phase = Phase::GameOver;
        } else if !matches!(self.phase, Phase::Pause(_)) {
            self.phase = Phase::Pause(0.7);
        }
        self.last_report = Some(report.clone());
        let ended = self.online.as_mut().is_some_and(|o| std::mem::take(&mut o.ended_pending));
        if ended && let Some(result) = self.online.as_ref().and_then(|o| o.result.clone()) {
            self.apply_result(&result);
        }
    }

    fn run_replay(&mut self, dt: f32) {
        let Some(r) = self.replay.as_mut() else { return };
        let seen = r.events_seen;
        let moving = r.simulation.advance((dt as f64).min(0.1));
        Self::collect_sounds(r.simulation.events(), seen, &mut self.sounds);
        r.events_seen = r.simulation.events().len();
        if !moving {
            r.done += dt;
            if r.done > 1.2 {
                self.replay = None;
            }
        }
    }

    /// Simulation time of the shot in progress (for the cue follow-through).
    pub fn shot_time(&self) -> Option<f64> {
        if let Some(r) = &self.replay {
            return Some(r.simulation.time());
        }
        match &self.phase {
            Phase::Simulating(p) => Some(p.simulation.time()),
            _ => None,
        }
    }
}

impl Match {
    /// Development aid (`--on-eight`): clears the solids so player 0 is on the
    /// 8-ball, to try the pocket call.
    pub fn debug_on_eight(&mut self) {
        let Ok(mut v) = serde_json::to_value(&self.game) else { return };
        if let Some(balls) = v["balls"].as_array_mut() {
            for b in balls.iter_mut() {
                let id = b["id"].as_u64().unwrap_or(0);
                if (1..=7).contains(&id) || (9..=12).contains(&id) {
                    b["status"] = serde_json::json!({ "Pocketed": "FootLeft" });
                }
            }
        }
        v["groups"] = serde_json::json!(["Solids", "Stripes"]);
        v["is_break"] = serde_json::json!(false);
        v["ball_in_hand"] = serde_json::json!("Anywhere");
        v["current"] = serde_json::json!(0);
        if let Ok(g) = serde_json::from_value(v) {
            self.game = g;
            self.begin_turn();
        }
    }
}

/// The signed difference `to - from`, in (-π, π].
pub fn angle_delta(from: f64, to: f64) -> f64 {
    let mut d = (to - from).rem_euclid(std::f64::consts::TAU);
    if d > std::f64::consts::PI {
        d -= std::f64::consts::TAU;
    }
    d
}

/// A simple legal-looking shot for when the AI search fails.
fn fallback_decision(game: &EightBallGame) -> AiDecision {
    let cue = game.ball(BallId::CUE).xy();
    let target = game.legal_targets().into_iter().next().map(|id| game.ball(id).xy());
    let d = target.map(|t| t - cue).unwrap_or(DVec2::X);
    let called = game.needs_called_pocket().then_some(PocketId::FootLeft);
    AiDecision { cue_ball: None, shot: Shot::new(d.y.atan2(d.x), 2.0), called_pocket: called }
}

/// Adds a message to the log (and online, to the chat), and shows it for
/// `ttl` seconds.
fn announce_into(
    notices: &mut Vec<Notice>,
    log: &mut Vec<(String, bool)>,
    online: Option<&mut OnlineMatch>,
    text: String,
    important: bool,
    ttl: f32,
) {
    log.push((text.clone(), important));
    if log.len() > 60 {
        log.remove(0);
    }
    if let Some(o) = online {
        o.push_feed(FeedLine::event(&text, important));
    }
    notices.push(Notice { text, important, ttl });
    if notices.len() > 5 {
        notices.remove(0);
    }
}

/// A random seed from the clock.
pub fn clock_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(7)
}

/// How often the aim is streamed to the opponent.
const AIM_INTERVAL: f32 = 0.05;
/// How fast the opponent's cue glides to their latest aim (1/s).
const REMOTE_AIM_RATE: f64 = 16.0;

impl OnlineMatch {
    fn push_feed(&mut self, line: FeedLine) {
        self.feed.push(line);
        if self.feed.len() > 150 {
            self.feed.remove(0);
        }
    }
}

impl Match {
    /// Online bookkeeping: the shot clock, streaming the aim and smoothing
    /// the opponent's cue.
    fn run_online(&mut self, dt: f32) {
        let my_turn = self.is_human_turn();
        let placing = self.is_placing_cue_ball();
        let remote_turn = self.current_controller() == Controller::Remote
            && matches!(self.phase, Phase::Turn { .. });
        let cue = self.game.ball(BallId::CUE).xy();
        let aim = AimState {
            yaw: self.aim.yaw,
            elevation: self.aim.elevation,
            tip: self.aim.tip,
            power: if self.aim.charging { self.aim.power } else { 0.0 },
            called_pocket: self.called_pocket,
        };
        let Some(o) = self.online.as_mut() else { return };
        if matches!(self.phase, Phase::Turn { .. }) {
            o.clock_left = (o.clock_left - dt).max(0.0);
        } else {
            // The server's clock for the next turn already runs during playback.
            o.next_clock = (o.next_clock - dt).max(0.0);
        }
        if remote_turn && let Some(target) = o.remote_target {
            let k = 1.0 - (-(dt as f64) * REMOTE_AIM_RATE).exp();
            self.aim.yaw += angle_delta(self.aim.yaw, target.yaw) * k;
            self.aim.elevation += (target.elevation - self.aim.elevation) * k;
            self.aim.tip += (target.tip - self.aim.tip) * k;
            self.aim.power += (target.power - self.aim.power) * k as f32;
            self.aim.charging = target.power > 0.0;
        }
        if !my_turn {
            return;
        }
        o.aim_timer -= dt;
        if o.aim_timer > 0.0 {
            return;
        }
        o.aim_timer = AIM_INTERVAL;
        if placing {
            if o.last_cue != Some(cue) {
                o.last_cue = Some(cue);
                o.outbox.push(ClientMsg::PlaceCueBall { position: cue });
            }
        } else if o.last_aim != Some(aim) {
            o.last_aim = Some(aim);
            o.outbox.push(ClientMsg::Aim { aim });
        }
    }

    /// Ends the game with the server's result (resignation, abandonment…).
    fn apply_result(&mut self, result: &MatchResult) {
        if self.game.game_over().is_some() {
            return;
        }
        let loser = 1 - result.winner;
        let name = self.names[loser].clone();
        let text = match result.reason {
            GameOverReason::Abandoned => tw("{name} left the game.", &[("name", &name)]),
            GameOverReason::Resignation => tw("{name} resigned.", &[("name", &name)]),
            reason => {
                let winner = &self.names[result.winner];
                let how = crate::i18n::result_line(reason, winner, &name);
                tw("{winner} wins! ({how})", &[("winner", winner), ("how", &how)])
            }
        };
        self.game.conclude(result.winner, result.reason);
        self.notify(text, true);
        self.wins[result.winner] += 1;
        self.games_played += 1;
        self.replay = None;
        self.phase = Phase::GameOver;
    }

    /// Applies a match message from the server.
    pub fn handle_server(&mut self, msg: ServerMsg) {
        let remote_turn = self.current_controller() == Controller::Remote
            && matches!(self.phase, Phase::Turn { .. });
        let in_progress = self.shot_in_progress();
        let Some(o) = self.online.as_mut() else { return };
        let opponent = self.names[1 - o.seat].clone();
        match msg {
            ServerMsg::OpponentAim { aim } => {
                if remote_turn && aim.is_finite() {
                    let max_elevation = self.game.params().max_elevation;
                    let target = AimState {
                        yaw: aim.yaw,
                        elevation: aim.elevation.clamp(0.0, max_elevation),
                        tip: if aim.tip.length() > 1.0 { aim.tip.normalize() } else { aim.tip },
                        power: aim.power.clamp(0.0, 1.0),
                        called_pocket: aim.called_pocket,
                    };
                    if !o.remote_aimed {
                        // First aim this turn: jump there.
                        self.aim.yaw = target.yaw;
                        self.aim.elevation = target.elevation;
                        self.aim.tip = target.tip;
                    }
                    o.remote_aimed = true;
                    o.remote_target = Some(target);
                    self.called_pocket = target.called_pocket;
                }
            }
            ServerMsg::OpponentCueBall { position } => {
                if remote_turn && position.is_finite() {
                    self.game.place_cue_ball(position);
                }
            }
            ServerMsg::ShotTaken { seat, turn, shot, called_pocket, cue_ball, timeout } => {
                // Our own shot, already playing.
                if o.predicted.is_some_and(|(t, s)| t == turn && s == shot)
                    && seat == o.seat
                    && !timeout
                {
                    return;
                }
                let replace_prediction = o.predicted.is_some_and(|(t, _)| t == turn);
                let base = if replace_prediction { o.turn_start.take() } else { None };
                o.predicted = None;
                if timeout {
                    let text = tw(
                        "Time's up: {name}'s shot was played at full power.",
                        &[("name", &self.names[seat])],
                    );
                    self.announce(text, true, 5.0);
                }
                if let Some(pocket) = called_pocket
                    && self.online.as_ref().is_some_and(|o| o.seat != seat)
                    && self.game.called_pocket_of(seat).is_none()
                {
                    let text = tw(
                        "{name} calls the 8-ball in the {pocket} pocket.",
                        &[("name", &self.names[seat]), ("pocket", &crate::i18n::pocket(pocket))],
                    );
                    self.announce(text, false, 4.0);
                }
                match base {
                    Some(start) => self.game = start,
                    None => self.fast_forward(),
                }
                if let Some(p) = cue_ball {
                    self.game.place_cue_ball(p);
                }
                let o = self.online.as_mut().expect("online");
                o.sim_turn = Some(turn);
                o.pending_end = None;
                o.rematch_offered = false;
                self.aim = Aim {
                    yaw: shot.yaw,
                    elevation: shot.elevation,
                    tip: shot.tip,
                    power: 0.0,
                    charging: false,
                };
                self.called_pocket = called_pocket;
                match self.game.prepare_shot(&shot, called_pocket) {
                    Ok(pending) => self.start_animation(pending, shot),
                    // Should not happen: the server's result will be adopted
                    // as it arrives, without an animation.
                    Err(_) => {
                        if let Some(o) = self.online.as_mut() {
                            o.sim_turn = None;
                        }
                    }
                }
            }
            ServerMsg::ShotResult { turn, end, report, clock_left } => {
                o.next_clock = clock_left;
                if o.sim_turn == Some(turn) {
                    o.pending_end = Some((end, report));
                    if matches!(self.phase, Phase::Settling(_)) {
                        self.end_animation();
                    }
                } else {
                    // Nothing on screen for this shot (e.g. just reconnected).
                    self.fast_forward();
                    self.game = end;
                    self.report_shot(&report);
                    self.after_shot(&report);
                }
            }
            ServerMsg::ShotRejected { turn, reason, game } => {
                if o.predicted.is_some_and(|(t, _)| t == turn) {
                    o.predicted = None;
                    o.sim_turn = None;
                    o.turn_start = None;
                    o.pending_end = None;
                    self.game = game;
                    self.phase = Phase::Turn { placing_cue_ball: false, computer: None };
                    self.replay = None;
                } else if matches!(self.phase, Phase::Turn { .. }) {
                    self.game = game;
                }
                self.notify(crate::i18n::server_text(&reason), true);
            }
            ServerMsg::TimeFoul { seat, game, clock_left } => {
                o.next_clock = clock_left;
                let who = self.names[seat].clone();
                self.fast_forward();
                self.game = game;
                self.notify(
                    tw("{who} ran out of time: ball in hand for the opponent.", &[("who", &who)]),
                    true,
                );
                self.begin_turn();
            }
            ServerMsg::MatchEnded { result } => {
                for (r, d) in o.ratings.iter_mut().zip(result.new_ratings) {
                    if result.rated {
                        *r = d;
                    }
                }
                o.result = Some(result.clone());
                if in_progress {
                    // Finish showing the shot first (it may be what decided the game).
                    o.ended_pending = true;
                } else if self.game.game_over().is_none() {
                    self.apply_result(&result);
                }
            }
            ServerMsg::RematchOffered => {
                o.rematch_offered = true;
                self.notify(tw("{opponent} wants a rematch.", &[("opponent", &opponent)]), true);
            }
            ServerMsg::OpponentLeft => {
                o.opponent_left = true;
                self.notify(
                    tw("{opponent} went back to the lounge.", &[("opponent", &opponent)]),
                    false,
                );
            }
            ServerMsg::OpponentConnection { connected, grace_secs } => {
                o.opponent_connected = connected;
                if connected {
                    self.notify(tw("{opponent} is back.", &[("opponent", &opponent)]), false);
                } else {
                    self.notify(
                        tw(
                            "{opponent} lost the connection. Waiting up to {secs} s…",
                            &[("opponent", &opponent), ("secs", &grace_secs)],
                        ),
                        true,
                    );
                }
            }
            ServerMsg::MatchChat { line } => {
                let feed = FeedLine::chat(&line);
                if !line.from.eq_ignore_ascii_case(&self.names[o.seat]) {
                    o.unread += 1;
                    o.preview = Some((feed.clone(), 0.0));
                }
                o.push_feed(feed);
            }
            ServerMsg::Reaction { seat, code } => {
                if let Some(r) = snook_proto::reaction(&code) {
                    if seat < 2 {
                        // One at a time per player: a new one replaces the last.
                        o.reactions.retain(|(s, ..)| *s != seat);
                    } else {
                        // The crowd: a few at most, the oldest go first.
                        let crowd = o.reactions.iter().filter(|(s, ..)| *s >= 2).count();
                        if crowd >= 4
                            && let Some(i) = o.reactions.iter().position(|(s, ..)| *s >= 2)
                        {
                            o.reactions.remove(i);
                        }
                    }
                    o.reactions.push((seat.min(SPECTATOR_SEAT), r.code, 0.0));
                }
            }
            ServerMsg::Spectators { count } => o.spectators = count,
            ServerMsg::Error { message } => {
                self.notify(crate::i18n::server_text(&message), true);
            }
            _ => {}
        }
    }

    /// Sends a reaction to the table.
    pub fn send_reaction(&mut self, code: &str) {
        if let Some(o) = self.online.as_mut() {
            o.outbox.push(ClientMsg::React { code: code.to_string() });
        }
    }

    /// Watching someone else's game.
    pub fn is_spectating(&self) -> bool {
        self.online.as_ref().is_some_and(|o| o.spectator)
    }

    /// Sends a chat line to the opponent.
    pub fn send_chat(&mut self, text: &str) {
        let text = snook_proto::clean_chat(text);
        if let Some(o) = self.online.as_mut()
            && !o.spectator
            && !text.is_empty()
        {
            o.outbox.push(ClientMsg::MatchChat { text });
        }
    }
}

impl FeedLine {
    pub fn chat(line: &ChatLine) -> FeedLine {
        match line.kind {
            ChatKind::Player => FeedLine {
                from: Some((line.from.clone(), line.rating)),
                text: line.text.clone(),
                kind: FeedKind::Player,
            },
            ChatKind::Announcement => {
                FeedLine { from: None, text: line.text.clone(), kind: FeedKind::Announcement }
            }
            ChatKind::System => {
                FeedLine { from: None, text: line.text.clone(), kind: FeedKind::Event }
            }
        }
    }

    pub fn event(text: &str, important: bool) -> FeedLine {
        FeedLine {
            from: None,
            text: text.to_string(),
            kind: if important { FeedKind::Alert } else { FeedKind::Event },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn players() -> [PlayerInfo; 2] {
        let p = |n: &str| PlayerInfo {
            username: n.into(),
            rating: 1200,
            games: 0,
            wins: 0,
            status: snook_proto::PlayerStatus::Playing,
            country: String::new(),
            role: snook_proto::Role::Player,
            guest: false,
        };
        [p("me"), p("them")]
    }

    fn online_match(seat: usize) -> Match {
        let game = EightBallGame::new(4, 0);
        Match::online(1, seat, &players(), false, game, 30, 30.0, None, AimGuide::Full, false)
    }

    fn finish_animation(m: &mut Match) {
        for _ in 0..2000 {
            m.update(0.05);
            if !matches!(m.phase, Phase::Simulating(_)) {
                break;
            }
        }
    }

    #[test]
    fn opponent_winning_on_the_eight_is_not_a_resignation() {
        // The bug: the match ended while the deciding shot was still being
        // shown, and the game reported a resignation instead.
        let mut m = online_match(1);
        let shot = Shot::new(0.0, 6.0);
        m.handle_server(ServerMsg::ShotTaken {
            seat: 0,
            turn: 0,
            shot,
            called_pocket: None,
            cue_ball: None,
            timeout: false,
        });
        assert!(matches!(m.phase, Phase::Simulating(_)));
        let mut end = EightBallGame::new(4, 0);
        let report = end.play_shot(&shot, None).unwrap();
        // Stand-in for a game the 8-ball decided.
        end.conclude(0, GameOverReason::PocketedEight);
        m.handle_server(ServerMsg::ShotResult { turn: 0, end, report, clock_left: 30.0 });
        m.handle_server(ServerMsg::MatchEnded {
            result: MatchResult {
                winner: 0,
                reason: GameOverReason::PocketedEight,
                rated: true,
                rating_delta: [10, -10],
                new_ratings: [1210, 1190],
            },
        });
        assert!(m.game.game_over().is_none(), "still animating");
        finish_animation(&mut m);
        let over = m.game.game_over().expect("game over");
        assert_eq!(over.reason, GameOverReason::PocketedEight);
        assert!(!m.log.iter().any(|(l, _)| l.contains("resigned")), "{:?}", m.log);
    }

    #[test]
    fn own_shot_plays_at_once_and_adopts_the_server_result() {
        let mut m = online_match(0);
        m.aim.power = 0.8;
        m.aim.yaw = 0.0;
        assert!(m.shoot());
        assert!(matches!(m.phase, Phase::Simulating(_)), "no waiting for the server");
        let o = m.online.as_ref().unwrap();
        let Some(ClientMsg::Shoot { turn, shot, .. }) = o.outbox.last().cloned() else {
            panic!("shot not sent")
        };
        // The server echoes it (ignored) and sends the result.
        m.handle_server(ServerMsg::ShotTaken {
            seat: 0,
            turn,
            shot,
            called_pocket: None,
            cue_ball: None,
            timeout: false,
        });
        let mut server = EightBallGame::new(4, 0);
        let report = server.play_shot(&shot, None).unwrap();
        finish_animation(&mut m);
        assert!(matches!(m.phase, Phase::Settling(_)), "waits for the verdict");
        m.handle_server(ServerMsg::ShotResult {
            turn,
            end: server.clone(),
            report,
            clock_left: 30.0,
        });
        assert_eq!(m.game, server);
    }

    #[test]
    fn refused_shot_rolls_back() {
        let mut m = online_match(0);
        m.aim.power = 0.5;
        let before = m.game.clone();
        assert!(m.shoot());
        m.handle_server(ServerMsg::ShotRejected {
            turn: 0,
            reason: "It's not your turn.".into(),
            game: before.clone(),
        });
        assert!(matches!(m.phase, Phase::Turn { .. }));
        assert_eq!(m.game, before);
    }

    #[test]
    fn replay_does_not_touch_the_game() {
        let settings = MatchSettings {
            mode: MatchMode::HotSeat,
            names: ["a".into(), "b".into()],
            guide: AimGuide::Full,
            shot_clock: None,
        };
        let mut m = Match::new(&settings, 3);
        m.aim.power = 0.9;
        assert!(m.shoot());
        for _ in 0..2000 {
            m.update(0.05);
            if matches!(m.phase, Phase::Turn { .. } | Phase::GameOver) {
                break;
            }
        }
        let after = m.game.clone();
        m.start_replay();
        assert!(m.replay.is_some());
        assert!(!m.is_human_turn(), "no aiming during a replay");
        for _ in 0..2000 {
            m.update(0.05);
            if m.replay.is_none() {
                break;
            }
        }
        assert_eq!(m.game, after);
    }
}
