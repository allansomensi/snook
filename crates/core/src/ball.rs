//! Balls and their identities.

use glam::{DQuat, DVec2, DVec3};
use serde::{Deserialize, Serialize};

use crate::table::PocketId;

/// Ball number: `0` is the cue ball, `1..=7` solids, `8` the eight ball, `9..=15` stripes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BallId(pub u8);

impl BallId {
    pub const CUE: BallId = BallId(0);
    pub const EIGHT: BallId = BallId(8);

    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub fn kind(self) -> BallKind {
        match self.0 {
            0 => BallKind::Cue,
            1..=7 => BallKind::Solid,
            8 => BallKind::Eight,
            _ => BallKind::Stripe,
        }
    }

    pub fn is_object(self) -> bool {
        self.0 != 0
    }

    /// All sixteen balls in numeric order.
    pub fn all() -> impl Iterator<Item = BallId> {
        (0..16).map(BallId)
    }
}

impl std::fmt::Display for BallId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            0 => write!(f, "cue ball"),
            n => write!(f, "{n}-ball"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BallKind {
    Cue,
    Solid,
    Eight,
    Stripe,
}

/// Where a ball currently is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BallStatus {
    InPlay,
    Pocketed(PocketId),
    /// The ball left the table (e.g. a jump shot that cleared the rail).
    OffTable,
}

/// Instantaneous motion regime of a ball on the cloth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Motion {
    Stationary,
    /// Only rotating about the vertical axis.
    Spinning,
    /// The contact point slips over the cloth (kinetic friction).
    Sliding,
    /// Rolling without slipping (rolling resistance).
    Rolling,
    /// Not touching the slate.
    Airborne,
}

/// Full rigid-body state of one ball.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Ball {
    pub id: BallId,
    pub status: BallStatus,
    /// Centre position. A ball resting on the slate has `z == radius`.
    pub position: DVec3,
    pub velocity: DVec3,
    pub angular_velocity: DVec3,
    /// Orientation, used for rendering the numbers and stripes.
    pub orientation: DQuat,
    pub motion: Motion,
}

impl Ball {
    /// A ball at rest on the slate.
    pub fn at_rest(id: BallId, xy: DVec2, radius: f64) -> Self {
        Ball {
            id,
            status: BallStatus::InPlay,
            position: xy.extend(radius),
            velocity: DVec3::ZERO,
            angular_velocity: DVec3::ZERO,
            orientation: DQuat::IDENTITY,
            motion: Motion::Stationary,
        }
    }

    pub fn xy(&self) -> DVec2 {
        self.position.truncate()
    }

    pub fn in_play(&self) -> bool {
        self.status == BallStatus::InPlay
    }

    pub fn is_moving(&self) -> bool {
        self.in_play() && self.motion != Motion::Stationary
    }
}
