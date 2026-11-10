//! Table geometry: playing surface, cushions, pockets and reference spots.
//!
//! Coordinate system (right-handed, SI units):
//! - `x` runs along the length of the table (head rail at `-x`, foot rail at `+x`),
//! - `y` runs along the width,
//! - `z` points up; the slate surface is at `z = 0`.
//!
//! The playing area is the rectangle bounded by the cushion noses:
//! `[-length/2, length/2] x [-width/2, width/2]`.

use glam::DVec2;
use serde::{Deserialize, Serialize};

/// Physical dimensions of a table and its balls.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TableSpec {
    /// Playing-surface length between cushion noses (m).
    pub length: f64,
    /// Playing-surface width between cushion noses (m).
    pub width: f64,
    /// Ball radius (m).
    pub ball_radius: f64,
    /// Ball mass (kg).
    pub ball_mass: f64,
    /// Distance between the cushion noses at the mouth of a corner pocket (m).
    pub corner_mouth: f64,
    /// Distance between the cushion noses at the mouth of a side pocket (m).
    pub side_mouth: f64,
    /// Height of the cushion nose above the slate (m).
    pub cushion_height: f64,
    /// Width of the wooden rail outside the cushion nose (m). Balls that land
    /// beyond it are off the table.
    pub rail_width: f64,
    /// Length of the pocket jaws (m).
    pub jaw_length: f64,
    /// Depth of the corner-pocket shelf: how far past the mouth line the
    /// hole begins (m). WPA: 1"–2 1/4".
    pub corner_shelf: f64,
    /// Depth of the side-pocket shelf (m). WPA: 0–3/8".
    pub side_shelf: f64,
    /// How much each corner-pocket facing turns in towards the pocket axis
    /// (radians); 7° gives the regulation 142° facing angle, so the throat
    /// narrows and balls that hit a point rattle out.
    pub corner_facing_turn: f64,
    /// How much each side-pocket facing opens out from the perpendicular
    /// (radians); 14° gives the regulation 104° facing angle.
    pub side_facing_open: f64,
    /// Radius of the corner and side holes in the slate (m).
    pub corner_hole_radius: f64,
    pub side_hole_radius: f64,
}

impl TableSpec {
    /// A classic 7-foot "bar box" table with regulation 2 1/4" balls.
    pub const BAR_BOX_7FT: TableSpec = TableSpec {
        length: 1.981,
        width: 0.991,
        ball_radius: 0.028_575,
        ball_mass: 0.170,
        corner_mouth: 0.110,
        side_mouth: 0.124,
        cushion_height: 0.036_5,
        rail_width: 0.115,
        jaw_length: 0.075,
        corner_shelf: 0.030,
        side_shelf: 0.006,
        corner_facing_turn: 7.0 * std::f64::consts::PI / 180.0,
        side_facing_open: 14.0 * std::f64::consts::PI / 180.0,
        corner_hole_radius: 0.057,
        side_hole_radius: 0.0625,
    };
}

impl Default for TableSpec {
    fn default() -> Self {
        Self::BAR_BOX_7FT
    }
}

/// Identifies one of the six pockets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PocketId {
    HeadLeft,
    HeadRight,
    SideLeft,
    SideRight,
    FootLeft,
    FootRight,
}

impl PocketId {
    pub fn is_side(self) -> bool {
        matches!(self, PocketId::SideLeft | PocketId::SideRight)
    }

    pub const ALL: [PocketId; 6] = [
        PocketId::HeadLeft,
        PocketId::HeadRight,
        PocketId::SideLeft,
        PocketId::SideRight,
        PocketId::FootLeft,
        PocketId::FootRight,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn name(self) -> &'static str {
        match self {
            PocketId::HeadLeft => "head left corner",
            PocketId::HeadRight => "head right corner",
            PocketId::SideLeft => "left side",
            PocketId::SideRight => "right side",
            PocketId::FootLeft => "foot left corner",
            PocketId::FootRight => "foot right corner",
        }
    }
}

/// A pocket opening.
///
/// Between the two cushion noses (the mouth) a throat bounded by the jaw
/// facings leads over a short shelf to the hole. A ball drops only once its
/// centre is over the hole; until then the facings can still throw it out.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Pocket {
    pub id: PocketId,
    /// Midpoint of the mouth line (between the two cushion noses).
    pub mouth_center: DVec2,
    /// Unit vector pointing out of the playing area, through the pocket.
    pub outward: DVec2,
    /// Half of the mouth width.
    pub half_mouth: f64,
    /// Distance from the mouth line to the edge of the hole.
    pub shelf: f64,
    /// Centre of the hole in the slate.
    pub hole_center: DVec2,
    /// Radius of the hole: a ball whose centre is inside it drops.
    pub hole_radius: f64,
}

impl Pocket {
    /// Returns true if a ball centred at `p` has dropped into this pocket.
    pub fn captures(&self, p: DVec2) -> bool {
        p.distance(self.hole_center) < self.hole_radius
    }

    /// True if `p` is past the mouth line, in this pocket's throat or hole.
    pub fn in_throat(&self, p: DVec2) -> bool {
        let rel = p - self.mouth_center;
        let depth = rel.dot(self.outward);
        let lateral = rel.dot(self.outward.perp()).abs();
        depth > -1e-9 && lateral < self.half_mouth + 0.4 * depth.max(0.0) + 0.01
            || p.distance(self.hole_center) < self.hole_radius + 0.03
    }

    /// The point a shooter should aim the object ball at (the middle of the
    /// throat, a little past the mouth).
    pub fn aim_point(&self) -> DVec2 {
        self.mouth_center + self.outward * (self.shelf * 0.5 + 0.01)
    }
}

/// A straight cushion section (a cushion nose line or a pocket jaw).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CushionSegment {
    pub a: DVec2,
    pub b: DVec2,
}

impl CushionSegment {
    /// Closest point on the segment to `p`.
    pub fn closest_point(&self, p: DVec2) -> DVec2 {
        let ab = self.b - self.a;
        let t = ((p - self.a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
        self.a + ab * t
    }
}

/// Fully built table geometry used by the simulation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Table {
    pub spec: TableSpec,
    pub cushions: Vec<CushionSegment>,
    pub pockets: Vec<Pocket>,
}

impl Table {
    pub fn new(spec: TableSpec) -> Self {
        let hl = spec.length / 2.0;
        let hw = spec.width / 2.0;
        let corner_cut = spec.corner_mouth / std::f64::consts::SQRT_2;
        let half_side = spec.side_mouth / 2.0;
        let jaw = spec.jaw_length;

        let mut cushions = Vec::with_capacity(18);
        let mut pockets = Vec::with_capacity(6);

        // Corner pockets.
        for (id, sx, sy) in [
            (PocketId::HeadLeft, -1.0, 1.0),
            (PocketId::HeadRight, -1.0, -1.0),
            (PocketId::FootLeft, 1.0, 1.0),
            (PocketId::FootRight, 1.0, -1.0),
        ] {
            let on_long = DVec2::new(sx * (hl - corner_cut), sy * hw);
            let on_short = DVec2::new(sx * hl, sy * (hw - corner_cut));
            let outward = DVec2::new(sx, sy).normalize();
            let mouth_center = (on_long + on_short) / 2.0;
            // Each facing turns in towards the pocket axis, narrowing the throat.
            for start in [on_long, on_short] {
                let side = (start - mouth_center).dot(outward.perp()).signum();
                let turn = -side * spec.corner_facing_turn;
                let dir = DVec2::from_angle(turn).rotate(outward);
                cushions.push(CushionSegment { a: start, b: start + dir * jaw });
            }
            let hole_radius = spec.corner_hole_radius;
            pockets.push(Pocket {
                id,
                mouth_center,
                outward,
                half_mouth: spec.corner_mouth / 2.0,
                shelf: spec.corner_shelf,
                hole_center: mouth_center + outward * (spec.corner_shelf + hole_radius),
                hole_radius,
            });
        }

        // Side pockets.
        for (id, sy) in [(PocketId::SideLeft, 1.0), (PocketId::SideRight, -1.0)] {
            let left = DVec2::new(-half_side, sy * hw);
            let right = DVec2::new(half_side, sy * hw);
            let outward = DVec2::new(0.0, sy);
            let open = spec.side_facing_open.tan();
            cushions.push(CushionSegment {
                a: left,
                b: left + DVec2::new(-open, sy).normalize() * jaw,
            });
            cushions.push(CushionSegment {
                a: right,
                b: right + DVec2::new(open, sy).normalize() * jaw,
            });
            let hole_radius = spec.side_hole_radius;
            pockets.push(Pocket {
                id,
                mouth_center: DVec2::new(0.0, sy * hw),
                outward,
                half_mouth: half_side,
                shelf: spec.side_shelf,
                hole_center: DVec2::new(0.0, sy * (hw + spec.side_shelf + hole_radius)),
                hole_radius,
            });
        }

        // Long rails (split by the side pockets).
        for sy in [1.0, -1.0] {
            cushions.push(CushionSegment {
                a: DVec2::new(-(hl - corner_cut), sy * hw),
                b: DVec2::new(-half_side, sy * hw),
            });
            cushions.push(CushionSegment {
                a: DVec2::new(half_side, sy * hw),
                b: DVec2::new(hl - corner_cut, sy * hw),
            });
        }
        // Short rails.
        for sx in [1.0, -1.0] {
            cushions.push(CushionSegment {
                a: DVec2::new(sx * hl, -(hw - corner_cut)),
                b: DVec2::new(sx * hl, hw - corner_cut),
            });
        }

        pockets.sort_by_key(|p| p.id);
        Table { spec, cushions, pockets }
    }

    pub fn pocket(&self, id: PocketId) -> &Pocket {
        &self.pockets[id.index()]
    }

    /// The foot spot, where the apex ball of the rack sits.
    pub fn foot_spot(&self) -> DVec2 {
        DVec2::new(self.spec.length / 4.0, 0.0)
    }

    /// The head spot.
    pub fn head_spot(&self) -> DVec2 {
        DVec2::new(-self.spec.length / 4.0, 0.0)
    }

    /// The x coordinate of the head string; the "kitchen" is `x <= head_string_x`.
    pub fn head_string_x(&self) -> f64 {
        -self.spec.length / 4.0
    }

    /// True if a resting ball centred at `p` lies fully inside the playing area.
    pub fn is_inside_playing_area(&self, p: DVec2) -> bool {
        let r = self.spec.ball_radius;
        p.x.abs() <= self.spec.length / 2.0 - r && p.y.abs() <= self.spec.width / 2.0 - r
    }

    /// True if `p` is beyond the outer edge of the wooden rails.
    pub fn is_off_table(&self, p: DVec2) -> bool {
        p.x.abs() > self.spec.length / 2.0 + self.spec.rail_width
            || p.y.abs() > self.spec.width / 2.0 + self.spec.rail_width
    }
}

impl Default for Table {
    fn default() -> Self {
        Table::new(TableSpec::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_six_pockets_and_eighteen_cushions() {
        let t = Table::default();
        assert_eq!(t.pockets.len(), 6);
        assert_eq!(t.cushions.len(), 18);
        for (i, p) in t.pockets.iter().enumerate() {
            assert_eq!(p.id.index(), i);
        }
    }

    #[test]
    fn corner_mouth_has_requested_width() {
        let t = Table::default();
        let jaws: Vec<_> = t.cushions[..2].to_vec();
        let w = (jaws[0].a - jaws[1].a).length();
        assert!((w - t.spec.corner_mouth).abs() < 1e-9);
    }

    #[test]
    fn pocket_capture() {
        let t = Table::default();
        let p = t.pocket(PocketId::FootLeft);
        assert!(!p.captures(DVec2::ZERO));
        // On the shelf the ball has not dropped yet; over the hole it has.
        assert!(!p.captures(p.mouth_center + p.outward * 0.015));
        assert!(p.captures(p.mouth_center + p.outward * (p.shelf + 0.02)));
        let side = t.pocket(PocketId::SideRight);
        assert!(side.captures(DVec2::new(0.0, -(t.spec.width / 2.0 + 0.03))));
        assert!(!side.captures(DVec2::new(0.3, -(t.spec.width / 2.0 - 0.03))));
    }
}
