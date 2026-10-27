//! Seeded maze generation.
//!
//! Order of work (each step re-checks legality and records a warning when it skips):
//! start tile, Dangerous Zone layout, spanning tree + loops, black, silver, blue,
//! ramps, stairs, speed bumps, obstacles, then (after classifying walls) victims.
//!
//! Design choices:
//! * Ramps and stairs lead to a raised dead-end plateau: `L - R - P` in a line, `R` is
//!   the ramp tile (both side edges walled), `P` is a dead end whose `height_ref` is
//!   the ramp rise. Edges between `P` and everything else are walled, so the
//!   heightfield stays consistent. Stairs use one step (tread = tile length).
//! * The Dangerous Zone is a small rectangle `D` of tiles (`in_dz`), sealed by walls
//!   except one door to the Red entrance tile, which lies outside `D`.
//! * Obstacles are 4 cm radius, 18 cm high, and touch a wall (the "touching a wall"
//!   alternative of the rule; a 28 cm tile cannot give 20 cm clearance).
//! * Walls are stored once per physical edge, canonicalised to the south/west tile.

use std::f64::consts::TAU;

use mm_core::{
    ConfigIssue, DangerousZone, Dir, GenerateRequest, MazeGraph, MazeHeader, Obstacle, RingColor,
    RulesProfile, Severity, Tile, TileClass, TileFeature, TileKind, TileXY, Vec2, Vec3, Victim,
    VictimKind, VictimStatus, VictimTier, Zones,
};
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use crate::grid::{build_walls, canonical, idx, reach, EdgeKey, Linearity, Uf, WallSet};
use crate::validate::{ring_sum, status_for_sum, validate};
use crate::{classify, MazeError};

const RING_COLORS: [RingColor; 5] = [
    RingColor::Black,
    RingColor::Red,
    RingColor::Yellow,
    RingColor::Green,
    RingColor::Blue,
];
const OBSTACLE_RADIUS_M: f64 = 0.04;
const RAMP_ANGLE_DEG: f64 = 15.0;

/// Generate a maze, discarding the skip warnings. See [`generate_with_report`].
pub fn generate(
    req: &GenerateRequest,
    seed: u64,
    rules: &RulesProfile,
) -> Result<MazeGraph, MazeError> {
    generate_with_report(req, seed, rules).map(|r| r.0)
}

/// Generate a maze and the warnings for features that could not be placed.
/// Same `seed` and request give an identical result.
pub fn generate_with_report(
    req: &GenerateRequest,
    seed: u64,
    rules: &RulesProfile,
) -> Result<(MazeGraph, Vec<ConfigIssue>), MazeError> {
    let (w, h) = (req.grid.w, req.grid.h);
    if !(2..=20).contains(&w) || !(2..=20).contains(&h) {
        return Err(MazeError::BadRequest(format!(
            "grid must be 2..=20 per side, got {w}x{h}"
        )));
    }
    for (name, v) in [
        ("loopiness", req.features.loopiness),
        ("floating_victim_share", req.features.floating_victim_share),
    ] {
        if !v.is_finite() || !(0.0..=1.0).contains(&v) {
            return Err(MazeError::BadRequest(format!("{name} must be in 0..=1")));
        }
    }
    let mut g = Gen::new(w, h, seed, rules);
    let f = &req.features;
    g.pick_start();
    if f.dangerous_zone {
        g.place_dz();
    }
    g.carve(f.loopiness);
    g.place_black(f.black);
    g.place_marks(TileKind::Silver, f.silver);
    g.place_marks(TileKind::Blue, f.blue);
    g.place_sites(false, f.ramps);
    g.place_sites(true, f.stairs);
    g.place_bumps(f.speed_bumps);
    g.place_obstacles(f.obstacles);
    let mut m = g.finish(req, seed);
    classify(&mut m);
    g.place_victims(&mut m, req);
    let v = validate(&m, rules);
    if !v.ok {
        let errs: Vec<String> = v
            .issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| format!("{}: {}", i.path, i.message))
            .collect();
        return Err(MazeError::Internal(format!(
            "{}\n{}",
            errs.join("; "),
            crate::ascii(&m)
        )));
    }
    Ok((m, g.warnings))
}

struct Gen<'a> {
    w: u32,
    h: u32,
    rules: &'a RulesProfile,
    rng: ChaCha8Rng,
    tiles: Vec<Tile>,
    walls: WallSet,
    occupied: Vec<bool>,
    start: TileXY,
    dz: Option<DangerousZone>,
    door: Option<EdgeKey>,
    obstacles: Vec<Obstacle>,
    warnings: Vec<ConfigIssue>,
}

impl<'a> Gen<'a> {
    fn new(w: u32, h: u32, seed: u64, rules: &'a RulesProfile) -> Self {
        let mut tiles = Vec::new();
        let mut walls = WallSet::default();
        for row in 0..h {
            for col in 0..w {
                let at = TileXY::new(col, row);
                tiles.push(Tile {
                    at,
                    kind: TileKind::Normal,
                    class: TileClass::Floating,
                    in_dz: false,
                    height_ref: 0.0,
                    features: vec![],
                });
                for d in Dir::ALL {
                    walls.insert(at, d);
                }
            }
        }
        Gen {
            w,
            h,
            rules,
            rng: ChaCha8Rng::seed_from_u64(seed),
            occupied: vec![false; tiles.len()],
            tiles,
            walls,
            start: TileXY::new(0, 0),
            dz: None,
            door: None,
            obstacles: vec![],
            warnings: vec![],
        }
    }

    fn warn(&mut self, path: &str, msg: String) {
        self.warnings.push(ConfigIssue {
            path: path.into(),
            severity: Severity::Warning,
            message: msg,
        });
    }

    fn at(&self, t: TileXY) -> &Tile {
        &self.tiles[idx(self.w, t)]
    }

    fn tile_m(&self) -> f64 {
        self.rules.field.tile_m
    }

    fn pick_start(&mut self) {
        let (w, h) = (self.w, self.h);
        let per: Vec<TileXY> = self
            .tiles
            .iter()
            .map(|t| t.at)
            .filter(|t| t.col == 0 || t.row == 0 || t.col == w - 1 || t.row == h - 1)
            .collect();
        self.start = per[self.rng.gen_range(0..per.len())];
        let i = idx(w, self.start);
        self.tiles[i].kind = TileKind::Start;
        self.occupied[i] = true;
    }

    fn connected(&self) -> bool {
        let seen = reach(self.w, self.h, &self.tiles, &self.walls, self.start, false);
        self.tiles
            .iter()
            .zip(seen)
            .all(|(t, s)| s || t.kind == TileKind::Black)
    }

    /// Whether the grid minus `dz` is connected from the start (ignoring walls).
    fn rest_connected(&self, dz: &[TileXY]) -> bool {
        let mut seen = vec![false; self.tiles.len()];
        let mut q = vec![self.start];
        seen[idx(self.w, self.start)] = true;
        let mut n = 1usize;
        while let Some(t) = q.pop() {
            for d in Dir::ALL {
                let Some(x) = t.step(d, self.w, self.h) else {
                    continue;
                };
                if seen[idx(self.w, x)] || dz.contains(&x) {
                    continue;
                }
                seen[idx(self.w, x)] = true;
                n += 1;
                q.push(x);
            }
        }
        n + dz.len() == self.tiles.len()
    }

    fn place_dz(&mut self) {
        let total = (self.w * self.h) as usize;
        let mut shapes = Vec::new();
        if total >= 16 {
            shapes.push((2u32, 2u32));
        }
        shapes.extend([(2, 1), (1, 2), (1, 1)]);
        for (a, b) in shapes {
            if a > self.w || b > self.h || (a * b) as usize + 2 > total {
                continue;
            }
            for _ in 0..80 {
                let c0 = self.rng.gen_range(0..=self.w - a);
                let r0 = self.rng.gen_range(0..=self.h - b);
                let mut dz = Vec::new();
                for r in r0..r0 + b {
                    for c in c0..c0 + a {
                        dz.push(TileXY::new(c, r));
                    }
                }
                if dz.contains(&self.start) || !self.rest_connected(&dz) {
                    continue;
                }
                let mut doors = Vec::new();
                for &t in &dz {
                    for d in Dir::ALL {
                        if let Some(o) = t.step(d, self.w, self.h) {
                            if !dz.contains(&o) && o != self.start {
                                doors.push((t, d, o));
                            }
                        }
                    }
                }
                if doors.is_empty() {
                    continue;
                }
                let (inner, d, outer) = doors[self.rng.gen_range(0..doors.len())];
                for &t in &dz {
                    let i = idx(self.w, t);
                    self.tiles[i].in_dz = true;
                    self.occupied[i] = true;
                }
                let oi = idx(self.w, outer);
                self.tiles[oi].kind = TileKind::Red;
                self.occupied[oi] = true;
                self.door = Some(canonical(inner, d));
                self.dz = Some(DangerousZone {
                    entrance: outer,
                    tiles: dz,
                });
                return;
            }
        }
        self.warn(
            "features.dangerous_zone",
            "no legal Dangerous Zone position found; skipped".into(),
        );
    }

    /// Randomised Kruskal spanning tree, then open `loopiness` of the remaining edges.
    fn carve(&mut self, loopiness: f64) {
        let mut edges: Vec<(TileXY, Dir)> = Vec::new();
        for t in &self.tiles {
            for d in [Dir::East, Dir::North] {
                let Some(n) = t.at.step(d, self.w, self.h) else {
                    continue;
                };
                if t.in_dz != self.at(n).in_dz && self.door != Some(canonical(t.at, d)) {
                    continue; // DZ boundary stays sealed
                }
                edges.push((t.at, d));
            }
        }
        edges.shuffle(&mut self.rng);
        let mut uf = Uf::new(self.tiles.len());
        let mut left = Vec::new();
        for (t, d) in edges {
            let n = t.step(d, self.w, self.h).expect("edge has neighbour");
            if uf.union(idx(self.w, t), idx(self.w, n)) {
                self.walls.remove(t, d);
            } else {
                left.push((t, d));
            }
        }
        let k = (loopiness * left.len() as f64).round() as usize;
        for &(t, d) in left.iter().take(k) {
            self.walls.remove(t, d);
        }
    }

    fn free_normal(&self, t: &Tile) -> bool {
        t.kind == TileKind::Normal
            && !t.in_dz
            && !self.occupied[idx(self.w, t.at)]
            && t.features.is_empty()
            && t.height_ref == 0.0
    }

    fn shuffled_free(&mut self) -> Vec<TileXY> {
        let mut c: Vec<TileXY> = self
            .tiles
            .iter()
            .filter(|t| self.free_normal(t))
            .map(|t| t.at)
            .collect();
        c.shuffle(&mut self.rng);
        c
    }

    fn place_black(&mut self, n: u32) {
        let mut placed = 0;
        for t in self.shuffled_free() {
            if placed >= n {
                break;
            }
            let i = idx(self.w, t);
            self.tiles[i].kind = TileKind::Black;
            if self.connected() {
                self.occupied[i] = true;
                placed += 1;
            } else {
                self.tiles[i].kind = TileKind::Normal;
            }
        }
        if placed < n {
            self.warn(
                "features.black",
                format!("placed {placed} of {n} black tiles without cutting the maze"),
            );
        }
    }

    fn place_marks(&mut self, kind: TileKind, n: u32) {
        let c = self.shuffled_free();
        for &t in c.iter().take(n as usize) {
            let i = idx(self.w, t);
            self.tiles[i].kind = kind;
            self.occupied[i] = true;
        }
        if (c.len() as u32) < n {
            self.warn(
                &format!("features.{kind:?}").to_lowercase(),
                format!("placed {} of {n}", c.len()),
            );
        }
    }

    fn place_sites(&mut self, stairs: bool, n: u32) {
        let mut order: Vec<(TileXY, Dir)> = Vec::new();
        for t in &self.tiles {
            for d in Dir::ALL {
                order.push((t.at, d));
            }
        }
        order.shuffle(&mut self.rng);
        let mut placed = 0;
        for (r, d) in order {
            if placed >= n {
                break;
            }
            if self.try_site(r, d, stairs) {
                placed += 1;
            }
        }
        if placed < n {
            self.warn(
                if stairs {
                    "features.stairs"
                } else {
                    "features.ramps"
                },
                format!("placed {placed} of {n}"),
            );
        }
    }

    /// Ramp or stairs on `r`, rising toward `d`, ending on a dead-end plateau.
    fn try_site(&mut self, r: TileXY, d: Dir, stairs: bool) -> bool {
        let (w, h) = (self.w, self.h);
        let (Some(p), Some(l)) = (r.step(d, w, h), r.step(d.opposite(), w, h)) else {
            return false;
        };
        if !self.free_normal(self.at(r)) || !self.free_normal(self.at(p)) {
            return false;
        }
        let lt = self.at(l);
        let l_ok = matches!(
            lt.kind,
            TileKind::Normal | TileKind::Start | TileKind::Silver | TileKind::Blue
        ) && !lt.in_dz
            && lt.features.is_empty()
            && lt.height_ref == 0.0;
        if !l_ok {
            return false;
        }
        for x in [r, p] {
            for dd in Dir::ALL {
                if let Some(n) = x.step(dd, w, h) {
                    if n == r || n == p || n == l {
                        continue;
                    }
                    let nt = self.at(n);
                    if !nt.features.is_empty() || nt.kind == TileKind::Black {
                        return false;
                    }
                }
            }
        }
        let snapshot = self.walls.clone();
        self.walls.remove(r, d);
        self.walls.remove(r, d.opposite());
        for dd in Dir::ALL {
            if dd != d && dd != d.opposite() {
                self.walls.insert(r, dd);
            }
            if dd != d.opposite() {
                self.walls.insert(p, dd);
            }
        }
        if !self.connected() {
            self.walls = snapshot;
            return false;
        }
        let tile_m = self.tile_m();
        let f = &self.rules.field;
        let (feature, rise) = if stairs {
            let steps = ((tile_m / f.stairs_tread_min_m).floor() as u32).clamp(1, 1);
            let step_m = f.stairs_step_max_m * 0.9;
            (
                TileFeature::Stairs {
                    up_dir: d,
                    step_m,
                    tread_m: tile_m / steps as f64,
                    steps,
                },
                step_m * steps as f64,
            )
        } else {
            let angle = RAMP_ANGLE_DEG.min(f.ramp_max_deg * 0.6);
            let rise = tile_m * angle.to_radians().tan();
            (
                TileFeature::Ramp {
                    up_dir: d,
                    angle_deg: rise.atan2(tile_m).to_degrees(),
                    rise_m: rise,
                },
                rise,
            )
        };
        let (ri, pi) = (idx(w, r), idx(w, p));
        self.tiles[ri].features.push(feature);
        self.tiles[pi].height_ref = rise;
        self.occupied[ri] = true;
        self.occupied[pi] = true;
        true
    }

    fn place_bumps(&mut self, n: u32) {
        let c = self.shuffled_free();
        let height = self.rules.field.speed_bump_max_m * 0.8;
        let tile_m = self.tile_m();
        for &t in c.iter().take(n as usize) {
            let across = Dir::ALL[self.rng.gen_range(0..4)];
            let i = idx(self.w, t);
            self.tiles[i].features.push(TileFeature::SpeedBump {
                across,
                height_m: height,
                width_m: tile_m * 0.1,
            });
            self.occupied[i] = true;
        }
        if (c.len() as u32) < n {
            self.warn("features.speed_bumps", format!("placed {} of {n}", c.len()));
        }
    }

    fn place_obstacles(&mut self, n: u32) {
        let tile_m = self.tile_m();
        let height = self.rules.field.obstacle_min_height_m * 1.2;
        let mut placed = 0u32;
        for t in self.shuffled_free() {
            if placed >= n {
                break;
            }
            let sides: Vec<Dir> = Dir::ALL
                .into_iter()
                .filter(|&d| self.walls.has(t, d))
                .collect();
            if sides.is_empty() {
                continue;
            }
            let s = sides[self.rng.gen_range(0..sides.len())];
            let c: Vec2 = t.center(tile_m);
            let off = tile_m / 2.0 - OBSTACLE_RADIUS_M;
            let (dx, dy) = s.delta();
            self.obstacles.push(Obstacle {
                id: placed,
                center: Vec2 {
                    x: c.x + dx as f64 * off,
                    y: c.y + dy as f64 * off,
                },
                radius_m: OBSTACLE_RADIUS_M,
                height_m: height,
                tile: t,
            });
            self.occupied[idx(self.w, t)] = true;
            placed += 1;
        }
        if placed < n {
            self.warn("features.obstacles", format!("placed {placed} of {n}"));
        }
    }

    fn finish(&mut self, req: &GenerateRequest, seed: u64) -> MazeGraph {
        let f = &self.rules.field;
        let start_yaw = Dir::ALL
            .into_iter()
            .find(|&d| {
                !self.walls.has(self.start, d)
                    && self
                        .start
                        .step(d, self.w, self.h)
                        .is_some_and(|n| self.at(n).kind != TileKind::Black)
            })
            .map_or(0.0, Dir::yaw);
        let mut checkpoints: Vec<TileXY> = self
            .tiles
            .iter()
            .filter(|t| t.kind == TileKind::Silver)
            .map(|t| t.at)
            .collect();
        checkpoints.sort();
        MazeGraph {
            header: MazeHeader {
                id: None,
                name: req.name.clone(),
                rules_profile: self.rules.id.clone(),
                seed,
                tile_m: f.tile_m,
                tolerance_frac: f.tolerance_frac,
                generator_version: format!("mm-maze/{}", env!("CARGO_PKG_VERSION")),
            },
            grid: req.grid,
            walls: build_walls(
                self.w,
                self.h,
                f.tile_m,
                f.wall_height_min_m,
                &self.tiles,
                &self.walls,
            ),
            tiles: self.tiles.clone(),
            victims: vec![],
            obstacles: self.obstacles.clone(),
            zones: Zones {
                start: self.start,
                start_yaw,
                checkpoints,
                dangerous_zone: self.dz.clone(),
            },
        }
    }

    fn place_victims(&mut self, m: &mut MazeGraph, req: &GenerateRequest) {
        let f = &req.features;
        let lin = Linearity::new(self.w, self.h, &m.tiles, &self.walls);
        let obstacle_tiles: Vec<TileXY> = m.obstacles.iter().map(|o| o.tile).collect();
        let mut linear: Vec<(TileXY, Dir)> = Vec::new();
        let mut floating: Vec<(TileXY, Dir)> = Vec::new();
        for t in &m.tiles {
            if t.kind != TileKind::Normal
                || !t.features.is_empty()
                || obstacle_tiles.contains(&t.at)
            {
                continue;
            }
            for d in Dir::ALL {
                if self.walls.has(t.at, d) {
                    if lin.edge_linear(t.at, d) {
                        linear.push((t.at, d));
                    } else {
                        floating.push((t.at, d));
                    }
                }
            }
        }
        // 0 smiley, 1 cognitive, 2 decoy
        let mut plan: Vec<u8> = vec![0; f.smiley_victims as usize];
        plan.extend(std::iter::repeat(1u8).take(f.cognitive_victims as usize));
        plan.extend(std::iter::repeat(2u8).take(f.decoys as usize));
        let wanted = plan.len();
        let mut used: Vec<EdgeKey> = Vec::new();
        for (id, kind_id) in plan.into_iter().enumerate() {
            let want_floating = self.rng.gen_bool(f.floating_victim_share);
            let mut picked = None;
            'pick: for from_floating in [want_floating, !want_floating] {
                let list = if from_floating {
                    &mut floating
                } else {
                    &mut linear
                };
                while !list.is_empty() {
                    let i = self.rng.gen_range(0..list.len());
                    let (t, d) = list.swap_remove(i);
                    if !used.contains(&canonical(t, d)) {
                        used.push(canonical(t, d));
                        picked = Some((t, d));
                        break 'pick;
                    }
                }
            }
            let Some((tile, side)) = picked else {
                break;
            };
            let (kind, status, tiered) = self.victim_kind(kind_id);
            let tile_m = self.tile_m();
            let c = tile.center(tile_m);
            let (dx, dy) = side.delta();
            let along = self.rng.gen_range(-0.05..0.05);
            let (ax, ay) = (dy.abs() as f64 * along, dx.abs() as f64 * along);
            let z = m.tiles[idx(self.w, tile)].height_ref + self.rules.field.victim_height_m;
            m.victims.push(Victim {
                id: id as u32,
                kind,
                status,
                pos: Vec3 {
                    x: c.x + dx as f64 * tile_m / 2.0 + ax,
                    y: c.y + dy as f64 * tile_m / 2.0 + ay,
                    z,
                },
                facing_yaw: side.opposite().yaw(),
                rotation: self.rng.gen_range(0.0..TAU),
                tile,
                wall_side: side,
                tier: tiered.then(|| {
                    if lin.edge_linear(tile, side) {
                        VictimTier::Linear
                    } else {
                        VictimTier::Floating
                    }
                }),
            });
        }
        if m.victims.len() < wanted {
            self.warn(
                "features.victims",
                format!(
                    "placed {} of {wanted}: out of legal wall faces",
                    m.victims.len()
                ),
            );
        }
    }

    /// Kind, ground-truth status and whether the victim counts (has a tier).
    fn victim_kind(&mut self, which: u8) -> (VictimKind, Option<VictimStatus>, bool) {
        match which {
            0 => {
                let s = [
                    VictimStatus::Harmed,
                    VictimStatus::Stable,
                    VictimStatus::Unharmed,
                ][self.rng.gen_range(0..3)];
                (VictimKind::Smiley { status: s }, Some(s), true)
            }
            1 => {
                let (rings, sum) = self.rings(|s| (0..=2).contains(&s));
                (VictimKind::Cognitive { rings }, status_for_sum(sum), true)
            }
            _ => {
                if self.rng.gen_bool(0.5) {
                    (VictimKind::Decoy, None, false)
                } else {
                    let (rings, _) = self.rings(|s| !(0..=2).contains(&s));
                    (VictimKind::Cognitive { rings }, None, false)
                }
            }
        }
    }

    fn rings(&mut self, ok: impl Fn(i32) -> bool) -> ([RingColor; 5], i32) {
        loop {
            let mut rings = [RingColor::Black; 5];
            for r in &mut rings {
                *r = RING_COLORS[self.rng.gen_range(0..5)];
            }
            let sum = ring_sum(self.rules, &rings);
            if ok(sum) {
                return (rings, sum);
            }
        }
    }
}
