//! Shared grid helpers: canonical wall edges, reachability, linear/floating classes,
//! floor heights and wall segments.

use std::collections::{BTreeSet, VecDeque};

use mm_core::{
    Dir, MazeGraph, Tile, TileClass, TileFeature, TileKind, TileXY, Vec2, Wall, WallEdge,
};

pub type EdgeKey = (TileXY, Dir);

/// South/west canonical form of a tile edge.
pub fn canonical(t: TileXY, side: Dir) -> EdgeKey {
    match side {
        Dir::South if t.row > 0 => (TileXY::new(t.col, t.row - 1), Dir::North),
        Dir::West if t.col > 0 => (TileXY::new(t.col - 1, t.row), Dir::East),
        _ => (t, side),
    }
}

pub fn is_canonical(t: TileXY, side: Dir) -> bool {
    canonical(t, side) == (t, side)
}

#[derive(Clone, Debug, Default)]
pub struct WallSet {
    set: BTreeSet<EdgeKey>,
}

impl WallSet {
    pub fn has(&self, t: TileXY, side: Dir) -> bool {
        self.set.contains(&canonical(t, side))
    }
    pub fn insert(&mut self, t: TileXY, side: Dir) -> bool {
        self.set.insert(canonical(t, side))
    }
    pub fn remove(&mut self, t: TileXY, side: Dir) {
        self.set.remove(&canonical(t, side));
    }
    pub fn len(&self) -> usize {
        self.set.len()
    }
    /// Walls of a graph that sit on grid edges (segments without `edge` are ignored).
    pub fn from_graph(m: &MazeGraph) -> WallSet {
        let mut s = WallSet::default();
        for w in &m.walls {
            if let Some(e) = w.edge {
                s.insert(e.tile, e.side);
            }
        }
        s
    }
}

pub fn idx(w: u32, t: TileXY) -> usize {
    (t.row * w + t.col) as usize
}

pub struct Uf {
    p: Vec<usize>,
}

impl Uf {
    pub fn new(n: usize) -> Self {
        Uf {
            p: (0..n).collect(),
        }
    }
    pub fn find(&mut self, mut x: usize) -> usize {
        while self.p[x] != x {
            self.p[x] = self.p[self.p[x]];
            x = self.p[x];
        }
        x
    }
    /// True when the two sets were distinct.
    pub fn union(&mut self, a: usize, b: usize) -> bool {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return false;
        }
        self.p[ra] = rb;
        true
    }
}

/// Tiles reachable from `start` over open edges, never entering black tiles
/// (and, with `block_dz`, never entering Dangerous Zone tiles).
pub fn reach(
    w: u32,
    h: u32,
    tiles: &[Tile],
    walls: &WallSet,
    start: TileXY,
    block_dz: bool,
) -> Vec<bool> {
    let mut seen = vec![false; tiles.len()];
    if start.col >= w || start.row >= h || tiles[idx(w, start)].kind == TileKind::Black {
        return seen;
    }
    let mut q = VecDeque::new();
    seen[idx(w, start)] = true;
    q.push_back(start);
    while let Some(t) = q.pop_front() {
        for d in Dir::ALL {
            if walls.has(t, d) {
                continue;
            }
            let Some(n) = t.step(d, w, h) else { continue };
            let nt = &tiles[idx(w, n)];
            if seen[idx(w, n)] || nt.kind == TileKind::Black || (block_dz && nt.in_dz) {
                continue;
            }
            seen[idx(w, n)] = true;
            q.push_back(n);
        }
    }
    seen
}

/// Grid-vertex endpoints of a tile edge.
pub fn edge_vertices(t: TileXY, side: Dir) -> ((u32, u32), (u32, u32)) {
    let (c, r) = (t.col, t.row);
    match side {
        Dir::South => ((c, r), (c + 1, r)),
        Dir::North => ((c, r + 1), (c + 1, r + 1)),
        Dir::West => ((c, r), (c, r + 1)),
        Dir::East => ((c + 1, r), (c + 1, r + 1)),
    }
}

pub fn edge_points(t: TileXY, side: Dir, tile_m: f64) -> (Vec2, Vec2) {
    let (a, b) = edge_vertices(t, side);
    (
        Vec2 {
            x: a.0 as f64 * tile_m,
            y: a.1 as f64 * tile_m,
        },
        Vec2 {
            x: b.0 as f64 * tile_m,
            y: b.1 as f64 * tile_m,
        },
    )
}

fn solid_edge(w: u32, h: u32, tiles: &[Tile], walls: &WallSet, t: TileXY, d: Dir) -> bool {
    walls.has(t, d)
        || tiles[idx(w, t)].kind == TileKind::Black
        || t.step(d, w, h)
            .is_some_and(|n| tiles[idx(w, n)].kind == TileKind::Black)
}

/// Wall components over grid vertices; the one holding vertex (0,0) is the perimeter's.
pub struct Linearity {
    perimeter: Vec<bool>,
    tile_linear: Vec<bool>,
    vw: usize,
}

impl Linearity {
    pub fn new(w: u32, h: u32, tiles: &[Tile], walls: &WallSet) -> Self {
        let vw = (w + 1) as usize;
        let vid = |p: (u32, u32)| p.1 as usize * vw + p.0 as usize;
        let mut uf = Uf::new(vw * (h + 1) as usize);
        for t in tiles {
            for d in Dir::ALL {
                if solid_edge(w, h, tiles, walls, t.at, d) {
                    let (a, b) = edge_vertices(t.at, d);
                    uf.union(vid(a), vid(b));
                }
            }
        }
        let root = uf.find(0);
        let perimeter: Vec<bool> = (0..vw * (h + 1) as usize)
            .map(|v| uf.find(v) == root)
            .collect();
        let tile_linear = tiles
            .iter()
            .map(|t| {
                Dir::ALL.iter().any(|&d| {
                    solid_edge(w, h, tiles, walls, t.at, d)
                        && perimeter[vid(edge_vertices(t.at, d).0)]
                })
            })
            .collect();
        Linearity {
            perimeter,
            tile_linear,
            vw,
        }
    }

    pub fn tile_class(&self, i: usize) -> TileClass {
        if self.tile_linear[i] {
            TileClass::Linear
        } else {
            TileClass::Floating
        }
    }

    /// Whether the (existing) wall on this edge belongs to the perimeter component.
    pub fn edge_linear(&self, t: TileXY, side: Dir) -> bool {
        let a = edge_vertices(t, side).0;
        self.perimeter[a.1 as usize * self.vw + a.0 as usize]
    }
}

/// Set `class` on every tile: linear when one of its walls (or black-neighbour edges)
/// is connected to the perimeter wall chain, floating otherwise.
pub fn classify(m: &mut MazeGraph) {
    let ws = WallSet::from_graph(m);
    let lin = Linearity::new(m.grid.w, m.grid.h, &m.tiles, &ws);
    for (i, t) in m.tiles.iter_mut().enumerate() {
        t.class = lin.tile_class(i);
    }
}

/// `(direction, rise)` of a ramp or stairs feature.
pub fn rise_of(t: &Tile) -> Option<(Dir, f64)> {
    t.features.iter().find_map(|f| match f {
        TileFeature::Ramp { up_dir, rise_m, .. } => Some((*up_dir, *rise_m)),
        TileFeature::Stairs {
            up_dir,
            step_m,
            steps,
            ..
        } => Some((*up_dir, step_m * *steps as f64)),
        TileFeature::SpeedBump { .. } => None,
    })
}

/// Floor height at the middle of a tile edge; `None` for the side edges of a ramp or
/// stairs, which have no single height.
pub fn edge_height(t: &Tile, side: Dir) -> Option<f64> {
    match rise_of(t) {
        None => Some(t.height_ref),
        Some((up, rise)) if side == up => Some(t.height_ref + rise),
        Some((up, _)) if side == up.opposite() => Some(t.height_ref),
        Some(_) => None,
    }
}

/// Lowest and highest floor height along an edge.
pub fn edge_range(t: &Tile, side: Dir) -> (f64, f64) {
    match edge_height(t, side) {
        Some(v) => (v, v),
        None => (t.height_ref, t.height_ref + rise_of(t).map_or(0.0, |r| r.1)),
    }
}

/// One `Wall` per canonical edge in `walls`, row-major, sides South, West, North, East.
/// `z0` is the lowest floor under the wall; `height_m` reaches `wall_min` above the
/// highest floor next to it (so exactly `wall_min` on flat ground).
pub fn build_walls(
    w: u32,
    h: u32,
    tile_m: f64,
    wall_min: f64,
    tiles: &[Tile],
    walls: &WallSet,
) -> Vec<Wall> {
    let mut out = Vec::with_capacity(walls.len());
    for t in tiles {
        for side in [Dir::South, Dir::West, Dir::North, Dir::East] {
            if !is_canonical(t.at, side) || !walls.has(t.at, side) {
                continue;
            }
            let (mut lo, mut hi) = edge_range(t, side);
            if let Some(n) = t.at.step(side, w, h) {
                let (l2, h2) = edge_range(&tiles[idx(w, n)], side.opposite());
                lo = lo.min(l2);
                hi = hi.max(h2);
            }
            let (a, b) = edge_points(t.at, side, tile_m);
            out.push(Wall {
                a,
                b,
                z0: lo,
                height_m: hi - lo + wall_min,
                edge: Some(WallEdge { tile: t.at, side }),
            });
        }
    }
    out
}
