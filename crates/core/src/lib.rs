//! # snook_core
//!
//! Engine-agnostic core of **Snook**: ball physics, table geometry, the
//! eight-ball rules, the computer opponent and the rating system.
//!
//! This crate has no rendering or networking dependencies so that the desktop
//! client and the game server run exactly the same simulation.

pub mod ball;
pub mod math;
pub mod physics;
pub mod table;

pub use ball::{Ball, BallId, BallKind, BallStatus, Motion};
pub use physics::{Event, PhysicsParams, Shot, Simulation, StrikeError};
pub use table::{Pocket, PocketId, Table, TableSpec};
pub mod rng;
pub mod rules;

pub use rules::{
    BallInHand, EightBallGame, Foul, GameOver, GameOverReason, Group, PendingShot, ShotError,
    ShotReport,
};
pub mod ai;
pub mod rating;
pub mod stats;

pub use ai::{AiDecision, Difficulty};
