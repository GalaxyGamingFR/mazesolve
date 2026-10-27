//! Maze description as produced by `mm-maze` and consumed by `mm-sim` and the frontend.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Dir, TileXY, Vec2, Vec3};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MazeGraph {
    pub header: MazeHeader,
    pub grid: GridSize,
    /// Row-major: index = row * w + col.
    pub tiles: Vec<Tile>,
    pub walls: Vec<Wall>,
    pub victims: Vec<Victim>,
    pub obstacles: Vec<Obstacle>,
    pub zones: Zones,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MazeHeader {
    pub id: Option<String>,
    pub name: Option<String>,
    pub rules_profile: String,
    pub seed: u64,
    pub tile_m: f64,
    pub tolerance_frac: f64,
    pub generator_version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GridSize {
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TileKind {
    Normal,
    Start,
    /// Hole: never entered, acts as a virtual wall.
    Black,
    /// Checkpoint (reflective floor).
    Silver,
    /// Puddle: robot must stop for `blue_stop_s`.
    Blue,
    /// Entrance of the Dangerous Zone.
    Red,
}

/// Linear tiles touch a wall chain connected to the perimeter; floating ones do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TileClass {
    Linear,
    Floating,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Tile {
    pub at: TileXY,
    pub kind: TileKind,
    pub class: TileClass,
    pub in_dz: bool,
    /// Floor height of the tile's low edge, meters.
    pub height_ref: f64,
    pub features: Vec<TileFeature>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TileFeature {
    /// Straight ramp rising toward `up_dir`, ending `rise_m` higher.
    Ramp {
        up_dir: Dir,
        angle_deg: f64,
        rise_m: f64,
    },
    /// Stairs rising toward `up_dir`.
    Stairs {
        up_dir: Dir,
        step_m: f64,
        tread_m: f64,
        steps: u32,
    },
    /// Speed bump across the tile, perpendicular to `across`.
    SpeedBump {
        across: Dir,
        height_m: f64,
        width_m: f64,
    },
}

/// Wall segment in world meters. `edge` ties it back to the grid when it sits on one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Wall {
    pub a: Vec2,
    pub b: Vec2,
    pub z0: f64,
    pub height_m: f64,
    pub edge: Option<WallEdge>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub struct WallEdge {
    pub tile: TileXY,
    pub side: Dir,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VictimStatus {
    Harmed,
    Stable,
    Unharmed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RingColor {
    Black,
    Red,
    Yellow,
    Green,
    Blue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VictimKind {
    /// Emoji face: sad = harmed, neutral = stable, happy = unharmed.
    Smiley { status: VictimStatus },
    /// Five concentric rings, innermost first; the colour values sum to the status.
    Cognitive { rings: [RingColor; 5] },
    /// Looks like a victim, is not one. Identifying it is a misidentification.
    Decoy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VictimTier {
    Linear,
    Floating,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Victim {
    pub id: u32,
    pub kind: VictimKind,
    /// Ground-truth status (`None` for decoys).
    pub status: Option<VictimStatus>,
    /// Center of the victim on the wall face, world meters.
    pub pos: Vec3,
    /// Yaw of the wall's outward normal (the direction the victim faces).
    pub facing_yaw: f64,
    /// In-plane rotation of the printed target, radians.
    pub rotation: f64,
    pub tile: TileXY,
    pub wall_side: Dir,
    pub tier: Option<VictimTier>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Obstacle {
    pub id: u32,
    pub center: Vec2,
    /// Footprint as a circle; enough for kinematic collision.
    pub radius_m: f64,
    pub height_m: f64,
    pub tile: TileXY,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Zones {
    pub start: TileXY,
    pub start_yaw: f64,
    pub checkpoints: Vec<TileXY>,
    pub dangerous_zone: Option<DangerousZone>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DangerousZone {
    pub entrance: TileXY,
    pub tiles: Vec<TileXY>,
}

impl MazeGraph {
    pub fn idx(&self, t: TileXY) -> Option<usize> {
        (t.col < self.grid.w && t.row < self.grid.h).then(|| (t.row * self.grid.w + t.col) as usize)
    }

    pub fn tile(&self, t: TileXY) -> Option<&Tile> {
        self.idx(t).and_then(|i| self.tiles.get(i))
    }

    /// Tile containing a world point, if inside the grid.
    pub fn tile_at(&self, p: Vec2) -> Option<TileXY> {
        if p.x < 0.0 || p.y < 0.0 {
            return None;
        }
        let t = TileXY::new(
            (p.x / self.header.tile_m) as u32,
            (p.y / self.header.tile_m) as u32,
        );
        self.idx(t).map(|_| t)
    }

    pub fn has_wall(&self, t: TileXY, side: Dir) -> bool {
        let other = t.step(side, self.grid.w, self.grid.h);
        self.walls.iter().any(|w| match w.edge {
            Some(e) => {
                (e.tile == t && e.side == side)
                    || other.is_some_and(|o| e.tile == o && e.side == side.opposite())
            }
            None => false,
        })
    }
}
