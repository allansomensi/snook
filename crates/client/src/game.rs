//! Match flow: turns, shot playback, the computer opponent and messages.

use std::sync::{Arc, Mutex};

use ::glam::DVec2;
use snook_core::ai::{self, AiDecision};
use snook_core::{
    BallId, Difficulty, EightBallGame, Event, PendingShot, PocketId, Shot, ShotError, ShotReport,
};

use snook_proto::{AimState, ChatLine, ClientMsg, MatchResult, PlayerInfo, ServerMsg};

use crate::config::AimGuide;
use crate::i18n::tw;
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
    /// 0..=1, mapped linearly to cue speed.
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
    /// Online: the shot was sent and the server has not answered yet.
    Awaiting,
    /// Balls are rolling.
    Simulating(Box<PendingShot>),
    /// Short pause after a shot so the result can be read.
    Pause(f32),
    GameOver,
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
    /// Everything that happened this game, newest last (shown in the HUD log).
    pub log: Vec<(String, bool)>,
    pub last_report: Option<ShotReport>,
    pub guide: AimGuide,
    pub games_played: u32,
    pub wins: [u32; 2],
    /// Collisions to play as sounds (drained by the audio module).
    pub sounds: Vec<ImpactSound>,
    /// Set for matches played through the server.
    pub online: Option<OnlineMatch>,
    seed: u64,
    events_seen: usize,
    demo_timer: f32,
}

/// State of a match played on the server.
pub struct OnlineMatch {
    pub seat: usize,
    pub rated: bool,
    pub ratings: [i32; 2],
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
    pub chat: Vec<ChatLine>,
    /// Chat lines the player has not looked at.
    pub unread: usize,
    /// The opponent has sent their aim this turn (until then their cue is hidden).
    pub remote_aimed: bool,
    /// The server's result for the shot being animated.
    pending_end: Option<(EightBallGame, ShotReport)>,
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
        let (names, controllers) = match settings.mode {
            MatchMode::VsComputer(d) => (
                [
                    settings.player_name.clone(),
                    crate::i18n::tw(
                        "Computer ({level})",
                        &[("level", &crate::i18n::difficulty(d))],
                    ),
                ],
                [Controller::Human, Controller::Computer(d)],
            ),
            MatchMode::HotSeat => (
                [settings.player_name.clone(), settings.second_player_name.clone()],
                [Controller::Human, Controller::Human],
            ),
            MatchMode::Demo => (
                [
                    tw(
                        "Computer ({level})",
                        &[("level", &crate::i18n::difficulty(Difficulty::Hard))],
                    ),
                    tw(
                        "Computer ({level})",
                        &[("level", &crate::i18n::difficulty(Difficulty::Medium))],
                    ),
                ],
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
        guide: AimGuide,
    ) -> Self {
        let mut controllers = [Controller::Remote; 2];
        controllers[seat] = Controller::Human;
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
            // Rated games only show a short aiming line.
            guide: if rated { AimGuide::Short } else { guide },
            games_played: 0,
            wins: [0, 0],
            sounds: Vec::new(),
            online: Some(OnlineMatch {
                seat,
                rated,
                ratings: [players[0].rating, players[1].rating],
                shot_clock: shot_clock_secs as f32,
                clock_left,
                outbox: Vec::new(),
                result: None,
                opponent_connected: true,
                opponent_left: false,
                rematch_offered: false,
                rematch_requested: false,
                chat: Vec::new(),
                unread: 0,
                remote_aimed: false,
                pending_end: None,
                aim_timer: 0.0,
                last_aim: None,
                last_cue: None,
                next_clock: clock_left,
            }),
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

    /// Starts the next online game with the same opponent (a rematch), keeping
    /// the score and the chat.
    pub fn continue_online(&mut self, next: Match) {
        let wins = self.wins;
        let games = self.games_played;
        let chat = self.online.as_ref().map(|o| o.chat.clone()).unwrap_or_default();
        let guide = self.guide;
        let swapped = next.names[0] != self.names[0];
        *self = next;
        self.wins = if swapped { [wins[1], wins[0]] } else { wins };
        self.games_played = games;
        if !self.online.as_ref().is_some_and(|o| o.rated) {
            self.guide = guide;
        }
        if let Some(o) = self.online.as_mut() {
            o.chat = chat;
        }
    }

    pub fn is_online(&self) -> bool {
        self.online.is_some()
    }

    /// Starts a new game; the players alternate the break. Online, this asks
    /// the opponent for a rematch.
    pub fn rematch(&mut self) {
        if let Some(o) = self.online.as_mut() {
            if !o.rematch_requested && !o.opponent_left {
                o.rematch_requested = true;
                o.outbox.push(ClientMsg::Rematch);
            }
            return;
        }
        self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let breaker = (self.games_played % 2) as usize;
        self.game = EightBallGame::new(self.seed, breaker);
        self.last_report = None;
        self.notices.clear();
        self.log.clear();
        self.begin_turn();
    }

    pub fn current_controller(&self) -> Controller {
        self.controllers[self.game.current_player()]
    }

    pub fn is_human_turn(&self) -> bool {
        matches!(self.phase, Phase::Turn { .. }) && self.current_controller() == Controller::Human
    }

    pub fn is_placing_cue_ball(&self) -> bool {
        matches!(self.phase, Phase::Turn { placing_cue_ball: true, .. })
    }

    pub fn set_placing(&mut self, placing: bool) {
        if let Phase::Turn { placing_cue_ball, .. } = &mut self.phase {
            *placing_cue_ball = placing && self.game.ball_in_hand().is_some();
        }
    }

    /// Balls as currently displayed (mid-shot or at rest).
    pub fn balls(&self) -> &[snook_core::Ball] {
        match &self.phase {
            Phase::Simulating(p) => &p.simulation.balls,
            _ => self.game.balls(),
        }
    }

    pub fn notify(&mut self, text: impl Into<String>, important: bool) {
        self.announce(text.into(), important, 4.0);
    }

    fn announce(&mut self, text: String, important: bool, ttl: f32) {
        announce_into(&mut self.notices, &mut self.log, text, important, ttl);
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
        }
        let computer_or_remote =
            computer.is_some() || self.current_controller() == Controller::Remote;
        self.aim = Aim { yaw: self.default_yaw(), ..Aim::default() };
        self.called_pocket = None;
        self.phase = Phase::Turn { placing_cue_ball: placing && !computer_or_remote, computer };
    }

    /// Points the cue at the nearest ball the player may hit.
    fn default_yaw(&self) -> f64 {
        self.game.default_aim()
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
    /// the shot is not allowed yet.
    pub fn shoot(&mut self) -> bool {
        if !matches!(self.phase, Phase::Turn { .. }) {
            return false;
        }
        let shot = self.shot_from_aim();
        if self.online.is_some() {
            // Check the shot locally, then let the server play it.
            let check = self.game.clone().prepare_shot(&shot, self.called_pocket).map(|_| ());
            return match check {
                Ok(()) => {
                    let cue_ball =
                        self.game.ball_in_hand().map(|_| self.game.ball(BallId::CUE).xy());
                    let msg =
                        ClientMsg::Shoot { shot, called_pocket: self.called_pocket, cue_ball };
                    if let Some(o) = self.online.as_mut() {
                        o.outbox.push(msg);
                    }
                    self.aim.charging = false;
                    self.phase = Phase::Awaiting;
                    true
                }
                Err(e) => {
                    self.shot_error(e);
                    false
                }
            };
        }
        match self.game.prepare_shot(&shot, self.called_pocket) {
            Ok(pending) => {
                self.sounds.push(ImpactSound { kind: ImpactKind::Cue, speed: shot.speed as f32 });
                self.events_seen = 0;
                self.aim.charging = false;
                self.phase = Phase::Simulating(Box::new(pending));
                true
            }
            Err(e) => {
                self.shot_error(e);
                false
            }
        }
    }

    fn shot_error(&mut self, e: ShotError) {
        match e {
            ShotError::MustCallPocket => self.notify(
                crate::i18n::t("Call a pocket for the 8-ball first: click a pocket."),
                true,
            ),
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
            self.notify(
                tw(
                    "{winner} wins: {reason}!",
                    &[("winner", &winner), ("reason", &crate::i18n::reason(over.reason))],
                ),
                true,
            );
        } else if !report.continues {
            let next = self.names[self.game.current_player()].clone();
            if let Some(bih) = self.game.ball_in_hand() {
                let where_ = match bih {
                    snook_core::BallInHand::Kitchen => crate::i18n::t(" behind the head string"),
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
        self.run_computer_turn(dt);
        self.run_simulation(dt);
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

    /// The current player concedes the game.
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
        if let Some(over) = self.game.game_over() {
            self.wins[over.winner] += 1;
            self.games_played += 1;
            let name = self.names[player].clone();
            self.notify(tw("{name} resigned.", &[("name", &name)]), true);
        }
        self.phase = Phase::GameOver;
    }

    fn run_computer_turn(&mut self, dt: f32) {
        let max_speed = self.game.params().max_cue_speed;
        let current_yaw = self.aim.yaw;
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
                        (decision.called_pocket, self.game.needs_called_pocket())
                    {
                        let name = self.names[self.game.current_player()].clone();
                        let text = tw(
                            "{name} calls the 8-ball in the {pocket} pocket.",
                            &[("name", &name), ("pocket", &crate::i18n::pocket(pocket))],
                        );
                        announce_into(&mut self.notices, &mut self.log, text, true, 5.0);
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
        let mut delta = (decision.shot.yaw - start_yaw).rem_euclid(std::f64::consts::TAU);
        if delta > std::f64::consts::PI {
            delta -= std::f64::consts::TAU;
        }
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

    fn run_simulation(&mut self, dt: f32) {
        // Real time, so shots never play in slow motion on slow computers.
        let dt = (dt as f64).min(0.1);
        let events_seen = self.events_seen;
        let Phase::Simulating(pending) = &mut self.phase else {
            return;
        };
        let moving = pending.simulation.advance(dt);
        let events = pending.simulation.events();
        let mut new_sounds = Vec::new();
        for e in &events[events_seen.min(events.len())..] {
            let sound = match *e {
                Event::BallBall { speed, .. } => Some((ImpactKind::Ball, speed)),
                Event::Cushion { speed, .. } => Some((ImpactKind::Cushion, speed)),
                Event::Slate { speed, .. } => Some((ImpactKind::Cushion, speed * 0.5)),
                Event::Pocketed { .. } => Some((ImpactKind::Pocket, 1.0)),
                Event::OffTable { .. } => None,
            };
            if let Some((kind, speed)) = sound {
                new_sounds.push(ImpactSound { kind, speed: speed as f32 });
            }
        }
        self.events_seen = events.len();
        self.sounds.extend(new_sounds);
        if moving {
            return;
        }
        let Phase::Simulating(pending) = std::mem::replace(&mut self.phase, Phase::Pause(0.7))
        else {
            unreachable!("checked above");
        };
        let report = match self.online.as_mut().and_then(|o| o.pending_end.take()) {
            Some((end, report)) => {
                // Online: the server's result is authoritative.
                self.game = end;
                report
            }
            None => self.game.finish_shot(*pending),
        };
        self.report_shot(&report);
        self.after_shot(&report);
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
    }

    /// Simulation time of the shot in progress (for the cue follow-through).
    pub fn shot_time(&self) -> Option<f64> {
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

/// A simple legal-looking shot for when the AI search fails.
fn fallback_decision(game: &EightBallGame) -> AiDecision {
    let cue = game.ball(BallId::CUE).xy();
    let target = game.legal_targets().into_iter().next().map(|id| game.ball(id).xy());
    let d = target.map(|t| t - cue).unwrap_or(DVec2::X);
    let called = game.needs_called_pocket().then_some(PocketId::FootLeft);
    AiDecision { cue_ball: None, shot: Shot::new(d.y.atan2(d.x), 2.0), called_pocket: called }
}

/// Adds a message to the log and shows it for `ttl` seconds.
fn announce_into(
    notices: &mut Vec<Notice>,
    log: &mut Vec<(String, bool)>,
    text: String,
    important: bool,
    ttl: f32,
) {
    log.push((text.clone(), important));
    if log.len() > 60 {
        log.remove(0);
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
const AIM_INTERVAL: f32 = 0.1;

impl Match {
    /// Online bookkeeping: the shot clock and streaming the aim.
    fn run_online(&mut self, dt: f32) {
        let my_turn = self.is_human_turn();
        let placing = self.is_placing_cue_ball();
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
            if o.opponent_connected {
                o.clock_left = (o.clock_left - dt).max(0.0);
            }
        } else {
            // The server's clock for the next turn already runs during playback.
            o.next_clock = (o.next_clock - dt).max(0.0);
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

    /// Applies a match message from the server.
    pub fn handle_server(&mut self, msg: ServerMsg) {
        let remote_turn = self.current_controller() == Controller::Remote
            && matches!(self.phase, Phase::Turn { .. });
        let Some(o) = self.online.as_mut() else { return };
        let opponent = self.names[1 - o.seat].clone();
        match msg {
            ServerMsg::OpponentAim { aim } => {
                let finite = aim.yaw.is_finite()
                    && aim.elevation.is_finite()
                    && aim.tip.is_finite()
                    && aim.power.is_finite();
                if remote_turn && finite {
                    o.remote_aimed = true;
                    self.aim.yaw = aim.yaw;
                    self.aim.elevation = aim.elevation.clamp(0.0, self.game.params().max_elevation);
                    self.aim.tip =
                        if aim.tip.length() > 1.0 { aim.tip.normalize() } else { aim.tip };
                    self.aim.power = aim.power.clamp(0.0, 1.0);
                    self.aim.charging = aim.power > 0.0;
                }
            }
            ServerMsg::OpponentCueBall { position } => {
                if remote_turn {
                    self.game.place_cue_ball(position);
                }
            }
            ServerMsg::ShotPlayed {
                seat,
                shot,
                called_pocket,
                start,
                end,
                report,
                clock_left,
                timeout,
            } => {
                if timeout {
                    let text = tw(
                        "Time's up: {name}'s shot was played at full power.",
                        &[("name", &self.names[seat])],
                    );
                    announce_into(&mut self.notices, &mut self.log, text, true, 5.0);
                }
                o.next_clock = clock_left;
                o.rematch_offered = false;
                if let Some(pocket) = called_pocket
                    && seat != o.seat
                {
                    let text = tw(
                        "{name} calls the 8-ball in the {pocket} pocket.",
                        &[("name", &self.names[seat]), ("pocket", &crate::i18n::pocket(pocket))],
                    );
                    announce_into(&mut self.notices, &mut self.log, text, true, 5.0);
                }
                let game = start;
                match game.prepare_shot(&shot, called_pocket) {
                    Ok(pending) => {
                        o.pending_end = Some((end, report));
                        self.game = game;
                        self.aim = Aim {
                            yaw: shot.yaw,
                            elevation: shot.elevation,
                            tip: shot.tip,
                            power: 0.0,
                            charging: false,
                        };
                        self.called_pocket = called_pocket;
                        self.sounds
                            .push(ImpactSound { kind: ImpactKind::Cue, speed: shot.speed as f32 });
                        self.events_seen = 0;
                        self.phase = Phase::Simulating(Box::new(pending));
                    }
                    Err(_) => {
                        // Should not happen: jump straight to the result.
                        self.game = end;
                        self.report_shot(&report);
                        self.after_shot(&report);
                    }
                }
                let _ = seat;
            }
            ServerMsg::TimeFoul { seat, game, clock_left } => {
                o.next_clock = clock_left;
                let who = self.names[seat].clone();
                self.game = game;
                self.notify(
                    tw("{who} ran out of time: ball in hand for the opponent.", &[("who", &who)]),
                    true,
                );
                self.begin_turn();
            }
            ServerMsg::MatchEnded { result } => {
                let winner = result.winner;
                for (r, d) in o.ratings.iter_mut().zip(result.new_ratings) {
                    if result.rated {
                        *r = d;
                    }
                }
                o.result = Some(result.clone());
                if self.game.game_over().is_none() {
                    // Resignation or abandonment: the table is left as it is.
                    let loser = 1 - winner;
                    match result.reason {
                        snook_core::GameOverReason::Abandoned => self.game.abandon(loser),
                        _ => self.game.resign(loser),
                    }
                    let name = self.names[loser].clone();
                    let text = match result.reason {
                        snook_core::GameOverReason::Abandoned => {
                            tw("{name} left the game.", &[("name", &name)])
                        }
                        _ => tw("{name} resigned.", &[("name", &name)]),
                    };
                    self.notify(text, true);
                    self.wins[winner] += 1;
                    self.games_played += 1;
                    self.phase = Phase::GameOver;
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
                o.unread += 1;
                o.chat.push(line);
                if o.chat.len() > 100 {
                    o.chat.remove(0);
                }
            }
            ServerMsg::Error { message } => {
                if matches!(self.phase, Phase::Awaiting) {
                    // The shot was refused; let the player try again.
                    self.phase = Phase::Turn { placing_cue_ball: false, computer: None };
                }
                self.notify(crate::i18n::server_text(&message), true);
            }
            _ => {}
        }
    }

    /// Sends a chat line to the opponent.
    pub fn send_chat(&mut self, text: &str) {
        let text = snook_proto::clean_chat(text);
        if let Some(o) = self.online.as_mut()
            && !text.is_empty()
        {
            o.outbox.push(ClientMsg::MatchChat { text });
        }
    }
}
