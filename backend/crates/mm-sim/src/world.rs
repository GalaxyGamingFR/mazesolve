//! The perturbed maze instance: jittered walls, heightfield, raycasts and collision queries.

use std::f64::consts::PI;

use mm_core::{
    Dir, ErrorProfile, MazeGraph, Obstacle, RulesProfile, TileFeature, TileKind, Vec2, Vec3,
};

use crate::rng::SimRng;

/// Width of the tile-edge lip ridge, meters.
const LIP_WIDTH_M: f64 = 0.008;

pub(crate) fn sub(a: Vec2, b: Vec2) -> Vec2 {
    Vec2 {
        x: a.x - b.x,
        y: a.y - b.y,
    }
}
pub(crate) fn dot(a: Vec2, b: Vec2) -> f64 {
    a.x * b.x + a.y * b.y
}
pub(crate) fn cross(a: Vec2, b: Vec2) -> f64 {
    a.x * b.y - a.y * b.x
}

/// 3x3 rotation: `Rz(yaw) * Ry(-pitch) * Rx(roll)`. Pitch positive = nose up,
/// roll positive = left side up.
#[derive(Debug, Clone, Copy)]
pub struct Rot3 {
    m: [[f64; 3]; 3],
}

impl Rot3 {
    pub fn from_ypr(yaw: f64, pitch: f64, roll: f64) -> Rot3 {
        let (sy, cy) = yaw.sin_cos();
        let (sp, cp) = (-pitch).sin_cos();
        let (sr, cr) = roll.sin_cos();
        let rz = [[cy, -sy, 0.0], [sy, cy, 0.0], [0.0, 0.0, 1.0]];
        let ry = [[cp, 0.0, sp], [0.0, 1.0, 0.0], [-sp, 0.0, cp]];
        let rx = [[1.0, 0.0, 0.0], [0.0, cr, -sr], [0.0, sr, cr]];
        let mul = |a: [[f64; 3]; 3], b: [[f64; 3]; 3]| {
            let mut o = [[0.0; 3]; 3];
            for (i, row) in o.iter_mut().enumerate() {
                for (j, cell) in row.iter_mut().enumerate() {
                    *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
                }
            }
            o
        };
        Rot3 {
            m: mul(mul(rz, ry), rx),
        }
    }

    pub fn apply(&self, v: [f64; 3]) -> [f64; 3] {
        let m = &self.m;
        [
            m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
            m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
            m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
        ]
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Seg {
    pub a: Vec2,
    pub b: Vec2,
    pub z0: f64,
    pub top: f64,
    min: Vec2,
    max: Vec2,
}

impl Seg {
    fn new(a: Vec2, b: Vec2, z0: f64, h: f64) -> Seg {
        Seg {
            a,
            b,
            z0,
            top: z0 + h,
            min: Vec2 {
                x: a.x.min(b.x),
                y: a.y.min(b.y),
            },
            max: Vec2 {
                x: a.x.max(b.x),
                y: a.y.max(b.y),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitKind {
    Wall,
    Obstacle,
    Floor,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub dist: f64,
    pub kind: HitKind,
}

/// Penetration of the robot circle into a wall or obstacle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Contact {
    /// Unit vector pointing from the obstacle toward the robot centre.
    pub normal: Vec2,
    pub depth: f64,
    /// Closest point on the obstacle surface.
    pub point: Vec2,
}

/// The perturbed maze instance a run happens in.
#[derive(Debug, Clone)]
pub struct World {
    /// Maze with the jittered walls (what the run actually uses).
    pub maze: MazeGraph,
    pub segs: Vec<Seg>,
    pub tile_m: f64,
    /// Tolerance fraction actually applied (rules tolerance x `tolerance_use`).
    pub applied_tolerance: f64,
    lip_m: f64,
}

impl World {
    /// Jitter the grid vertices (so walls stay connected) and wall heights by up to
    /// +-`applied_tolerance` of the nominal size.
    pub fn new(
        maze: &MazeGraph,
        rules: &RulesProfile,
        errors: &ErrorProfile,
        rng: &SimRng,
    ) -> World {
        let applied = (rules.field.tolerance_frac * errors.field.tolerance_use).max(0.0);
        let mut maze = maze.clone();
        let tile = maze.header.tile_m;
        let nx = maze.grid.w as usize + 1;
        let ny = maze.grid.h as usize + 1;
        let amp = applied * tile;
        let mut vs = rng.stream("world/vertex");
        let offsets: Vec<Vec2> = (0..nx * ny)
            .map(|_| Vec2 {
                x: vs.range(-1.0, 1.0) * amp,
                y: vs.range(-1.0, 1.0) * amp,
            })
            .collect();
        let snap = |p: Vec2| -> Vec2 {
            let ix = ((p.x / tile).round().max(0.0) as usize).min(nx - 1);
            let iy = ((p.y / tile).round().max(0.0) as usize).min(ny - 1);
            let o = offsets[iy * nx + ix];
            Vec2 {
                x: p.x + o.x,
                y: p.y + o.y,
            }
        };
        let mut hs = rng.stream("world/height");
        for w in &mut maze.walls {
            w.a = snap(w.a);
            w.b = snap(w.b);
            w.height_m *= 1.0 + hs.range(-1.0, 1.0) * applied;
        }
        let segs = maze
            .walls
            .iter()
            .map(|w| Seg::new(w.a, w.b, w.z0, w.height_m))
            .collect();
        World {
            segs,
            tile_m: tile,
            applied_tolerance: applied,
            lip_m: rules.field.tile_lip_max_m * errors.field.tolerance_use,
            maze,
        }
    }

    pub fn obstacles(&self) -> &[Obstacle] {
        &self.maze.obstacles
    }

    /// Surface kind under a point (`Normal` outside the grid).
    pub fn surface_at(&self, x: f64, y: f64) -> TileKind {
        self.maze
            .tile_at(Vec2 { x, y })
            .and_then(|t| self.maze.tile(t))
            .map_or(TileKind::Normal, |t| t.kind)
    }

    /// True when the point is on a tile carrying a ramp.
    pub fn on_ramp(&self, x: f64, y: f64) -> bool {
        self.maze
            .tile_at(Vec2 { x, y })
            .and_then(|t| self.maze.tile(t))
            .is_some_and(|t| {
                t.features
                    .iter()
                    .any(|f| matches!(f, TileFeature::Ramp { .. }))
            })
    }

    /// Floor height and unit normal at a point: flat tiles, ramps (linear), stairs
    /// (steps), speed bumps (cosine profile across the tile) and the tile-edge lip.
    pub fn floor_z(&self, x: f64, y: f64) -> (f64, [f64; 3]) {
        let up = (0.0, [0.0, 0.0, 1.0]);
        let Some(tx) = self.maze.tile_at(Vec2 { x, y }) else {
            return up;
        };
        let Some(tile) = self.maze.tile(tx) else {
            return up;
        };
        let t = self.tile_m;
        let u = x - tx.col as f64 * t;
        let v = y - tx.row as f64 * t;
        let mut z = tile.height_ref;
        let (mut gx, mut gy) = (0.0, 0.0);
        for f in &tile.features {
            match *f {
                TileFeature::Ramp { up_dir, rise_m, .. } => {
                    let (s, dsx, dsy) = progress(up_dir, u, v, t);
                    z += rise_m * s;
                    gx += rise_m * dsx;
                    gy += rise_m * dsy;
                }
                TileFeature::Stairs {
                    up_dir,
                    step_m,
                    steps,
                    ..
                } => {
                    if steps > 0 {
                        let (s, _, _) = progress(up_dir, u, v, t);
                        let k = ((s * steps as f64).floor() + 1.0).min(steps as f64);
                        z += step_m * k.max(0.0);
                    }
                }
                TileFeature::SpeedBump {
                    across,
                    height_m,
                    width_m,
                } => {
                    let along_x = matches!(across, Dir::East | Dir::West);
                    let d = if along_x { u } else { v } - t / 2.0;
                    let half = width_m / 2.0;
                    if half > 0.0 && d.abs() < half {
                        let ph = PI * d / half;
                        z += height_m * 0.5 * (1.0 + ph.cos());
                        let dz = -height_m * 0.5 * ph.sin() * PI / half;
                        if along_x {
                            gx += dz;
                        } else {
                            gy += dz;
                        }
                    }
                }
            }
        }
        if self.lip_m > 0.0 {
            let e = u.min(t - u).min(v).min(t - v);
            if e < LIP_WIDTH_M {
                let ph = PI * e / LIP_WIDTH_M;
                z += self.lip_m * 0.5 * (1.0 + ph.cos());
            }
        }
        let n = (gx * gx + gy * gy + 1.0).sqrt();
        (z, [-gx / n, -gy / n, 1.0 / n])
    }

    /// First hit along `dir` (normalised here) within `max_range`: wall quads,
    /// obstacle cylinders and the floor (modelled as the plane z = 0).
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max_range: f64) -> Option<Hit> {
        let len = (dir.x * dir.x + dir.y * dir.y + dir.z * dir.z).sqrt();
        if len < 1e-12 {
            return None;
        }
        let d = Vec3 {
            x: dir.x / len,
            y: dir.y / len,
            z: dir.z / len,
        };
        let p = Vec2 {
            x: origin.x,
            y: origin.y,
        };
        let r = Vec2 { x: d.x, y: d.y };
        let mut best = max_range;
        let mut kind = None;
        for s in &self.segs {
            let e = sub(s.b, s.a);
            let denom = cross(r, e);
            if denom.abs() < 1e-12 {
                continue;
            }
            let ap = sub(s.a, p);
            let t = cross(ap, e) / denom;
            let u = cross(ap, r) / denom;
            if t > 1e-9 && t < best && (0.0..=1.0).contains(&u) {
                let z = origin.z + t * d.z;
                if z >= s.z0 && z <= s.top {
                    best = t;
                    kind = Some(HitKind::Wall);
                }
            }
        }
        let a = dot(r, r);
        if a > 1e-12 {
            for o in &self.maze.obstacles {
                let pc = sub(p, o.center);
                let b = 2.0 * dot(r, pc);
                let c = dot(pc, pc) - o.radius_m * o.radius_m;
                let disc = b * b - 4.0 * a * c;
                if disc < 0.0 {
                    continue;
                }
                let t = (-b - disc.sqrt()) / (2.0 * a);
                if t > 1e-9 && t < best {
                    let z = origin.z + t * d.z;
                    if (0.0..=o.height_m).contains(&z) {
                        best = t;
                        kind = Some(HitKind::Obstacle);
                    }
                }
            }
        }
        if d.z < -1e-9 {
            let t = -origin.z / d.z;
            if t > 1e-9 && t < best {
                best = t;
                kind = Some(HitKind::Floor);
            }
        }
        kind.map(|kind| Hit { dist: best, kind })
    }

    /// Deepest penetration of a circle (centre `c`, radius `r`) into walls and obstacles.
    pub fn penetration(&self, c: Vec2, r: f64) -> Option<Contact> {
        let mut best: Option<Contact> = None;
        let mut consider = |cand: Contact| {
            if best.is_none_or(|b| cand.depth > b.depth) {
                best = Some(cand);
            }
        };
        for s in &self.segs {
            if c.x + r < s.min.x || c.x - r > s.max.x || c.y + r < s.min.y || c.y - r > s.max.y {
                continue;
            }
            let ab = sub(s.b, s.a);
            let l2 = dot(ab, ab);
            let u = if l2 < 1e-18 {
                0.0
            } else {
                (dot(sub(c, s.a), ab) / l2).clamp(0.0, 1.0)
            };
            let q = Vec2 {
                x: s.a.x + ab.x * u,
                y: s.a.y + ab.y * u,
            };
            let dv = sub(c, q);
            let dist = dot(dv, dv).sqrt();
            if dist < r {
                let normal = if dist > 1e-9 {
                    Vec2 {
                        x: dv.x / dist,
                        y: dv.y / dist,
                    }
                } else {
                    let l = l2.sqrt().max(1e-9);
                    Vec2 {
                        x: -ab.y / l,
                        y: ab.x / l,
                    }
                };
                consider(Contact {
                    normal,
                    depth: r - dist,
                    point: q,
                });
            }
        }
        for o in &self.maze.obstacles {
            let dv = sub(c, o.center);
            let dist = dot(dv, dv).sqrt();
            let reach = r + o.radius_m;
            if dist < reach {
                let normal = if dist > 1e-9 {
                    Vec2 {
                        x: dv.x / dist,
                        y: dv.y / dist,
                    }
                } else {
                    Vec2 { x: 1.0, y: 0.0 }
                };
                consider(Contact {
                    normal,
                    depth: reach - dist,
                    point: Vec2 {
                        x: o.center.x + normal.x * o.radius_m,
                        y: o.center.y + normal.y * o.radius_m,
                    },
                });
            }
        }
        best
    }
}

/// Progress 0..1 along `dir` across a tile, plus its gradient.
fn progress(dir: Dir, u: f64, v: f64, t: f64) -> (f64, f64, f64) {
    match dir {
        Dir::East => (u / t, 1.0 / t, 0.0),
        Dir::West => (1.0 - u / t, -1.0 / t, 0.0),
        Dir::North => (v / t, 0.0, 1.0 / t),
        Dir::South => (1.0 - v / t, 0.0, -1.0 / t),
    }
}
