use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// Full 3D pose. Angles in radians.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Pose3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub roll: f64,
    pub pitch: f64,
    pub yaw: f64,
}

/// Mount pose of a sensor relative to the robot center (robot frame: +x forward,
/// +y left, +z up). `yaw`/`pitch` aim the sensor's optical axis.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct MountPose {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub yaw: f64,
    pub pitch: f64,
}

/// Grid cell address.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Default,
    Serialize,
    Deserialize,
    JsonSchema,
)]
pub struct TileXY {
    pub col: u32,
    pub row: u32,
}

/// Compass side of a tile. North is +y.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Dir {
    North,
    East,
    South,
    West,
}

impl Dir {
    pub const ALL: [Dir; 4] = [Dir::North, Dir::East, Dir::South, Dir::West];

    pub fn opposite(self) -> Dir {
        match self {
            Dir::North => Dir::South,
            Dir::East => Dir::West,
            Dir::South => Dir::North,
            Dir::West => Dir::East,
        }
    }

    /// Unit step in grid coordinates.
    pub fn delta(self) -> (i32, i32) {
        match self {
            Dir::North => (0, 1),
            Dir::East => (1, 0),
            Dir::South => (0, -1),
            Dir::West => (-1, 0),
        }
    }

    /// World yaw of the outward direction.
    pub fn yaw(self) -> f64 {
        use std::f64::consts::FRAC_PI_2;
        match self {
            Dir::East => 0.0,
            Dir::North => FRAC_PI_2,
            Dir::West => 2.0 * FRAC_PI_2,
            Dir::South => -FRAC_PI_2,
        }
    }
}

impl TileXY {
    pub fn new(col: u32, row: u32) -> Self {
        Self { col, row }
    }

    /// Neighbour in `dir`, or `None` when it would leave a `w x h` grid.
    pub fn step(self, dir: Dir, w: u32, h: u32) -> Option<TileXY> {
        let (dx, dy) = dir.delta();
        let c = self.col as i64 + dx as i64;
        let r = self.row as i64 + dy as i64;
        (c >= 0 && r >= 0 && c < w as i64 && r < h as i64).then(|| TileXY::new(c as u32, r as u32))
    }

    /// Tile center in meters.
    pub fn center(self, tile_m: f64) -> Vec2 {
        Vec2 {
            x: (self.col as f64 + 0.5) * tile_m,
            y: (self.row as f64 + 0.5) * tile_m,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_stays_in_grid() {
        let t = TileXY::new(0, 0);
        assert_eq!(t.step(Dir::West, 3, 3), None);
        assert_eq!(t.step(Dir::North, 3, 3), Some(TileXY::new(0, 1)));
        assert_eq!(TileXY::new(2, 2).step(Dir::East, 3, 3), None);
    }

    #[test]
    fn opposite_round_trips() {
        for d in Dir::ALL {
            assert_eq!(d.opposite().opposite(), d);
        }
    }
}
