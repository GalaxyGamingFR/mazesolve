//! Maze validation against a rules profile.

use std::collections::BTreeSet;

use mm_core::{
    ConfigIssue, Dir, MazeGraph, MazeValidation, RingColor, RulesProfile, Severity, TileFeature,
    TileKind, TileXY, VictimKind, VictimStatus, VictimTier,
};

use crate::grid::{canonical, edge_height, edge_points, idx, reach, Linearity, WallSet};

const EPS: f64 = 1e-6;

/// Sum of the ring values under `rules`.
pub(crate) fn ring_sum(rules: &RulesProfile, rings: &[RingColor; 5]) -> i32 {
    rings.iter().map(|&c| rules.ring_value(c)).sum()
}

/// Status encoded by a ring sum; `None` when the target is not a valid victim.
pub(crate) fn status_for_sum(sum: i32) -> Option<VictimStatus> {
    match sum {
        2 => Some(VictimStatus::Harmed),
        1 => Some(VictimStatus::Stable),
        0 => Some(VictimStatus::Unharmed),
        _ => None,
    }
}

struct Issues(Vec<ConfigIssue>);

impl Issues {
    fn add(&mut self, sev: Severity, path: &str, msg: String) {
        self.0.push(ConfigIssue {
            path: path.into(),
            severity: sev,
            message: msg,
        });
    }
    fn err(&mut self, path: &str, msg: String) {
        self.add(Severity::Error, path, msg);
    }
    fn warn(&mut self, path: &str, msg: String) {
        self.add(Severity::Warning, path, msg);
    }
    fn finish(self, reachable: u32, enterable: u32) -> MazeValidation {
        MazeValidation {
            ok: !self.0.iter().any(|i| i.severity == Severity::Error),
            issues: self.0,
            reachable_tiles: reachable,
            enterable_tiles: enterable,
        }
    }
}

/// Check a maze against the rules. `ok` is true when there is no `Error` issue.
pub fn validate(m: &MazeGraph, rules: &RulesProfile) -> MazeValidation {
    let mut is = Issues(vec![]);
    let (w, h) = (m.grid.w, m.grid.h);
    let f = &rules.field;
    if w == 0 || h == 0 || m.tiles.len() != (w * h) as usize {
        is.err(
            "tiles",
            format!("{} tiles for a {w}x{h} grid", m.tiles.len()),
        );
        return is.finish(0, 0);
    }
    for (i, t) in m.tiles.iter().enumerate() {
        if idx(w, t.at) != i || t.at.col >= w {
            is.err(
                &format!("tiles[{i}]"),
                format!("tile address {:?} is off", t.at),
            );
            return is.finish(0, 0);
        }
    }
    if (m.header.tile_m - f.tile_m).abs() > EPS {
        is.err(
            "header.tile_m",
            format!("{} differs from rules {}", m.header.tile_m, f.tile_m),
        );
    }

    // Walls.
    let mut ws = WallSet::default();
    let mut seen_edges = BTreeSet::new();
    for (i, wl) in m.walls.iter().enumerate() {
        let path = format!("walls[{i}]");
        if wl.height_m < f.wall_height_min_m - EPS {
            is.err(
                &path,
                format!(
                    "height {} below minimum {}",
                    wl.height_m, f.wall_height_min_m
                ),
            );
        }
        let Some(e) = wl.edge else {
            is.warn(&path, "wall has no grid edge; ignored for topology".into());
            continue;
        };
        if e.tile.col >= w || e.tile.row >= h {
            is.err(&path, format!("edge tile {:?} outside the grid", e.tile));
            continue;
        }
        let key = canonical(e.tile, e.side);
        if !seen_edges.insert(key) {
            is.err(&path, format!("duplicate wall on edge {key:?}"));
        }
        let (a, b) = edge_points(e.tile, e.side, f.tile_m);
        let close =
            |p: mm_core::Vec2, q: mm_core::Vec2| (p.x - q.x).abs() < EPS && (p.y - q.y).abs() < EPS;
        if !(close(wl.a, a) && close(wl.b, b) || close(wl.a, b) && close(wl.b, a)) {
            is.err(&path, "segment does not match its tile edge".into());
        }
        ws.insert(e.tile, e.side);
    }
    let mut missing = 0;
    for t in &m.tiles {
        for d in Dir::ALL {
            if t.at.step(d, w, h).is_none() && !ws.has(t.at, d) {
                missing += 1;
            }
        }
    }
    if missing > 0 {
        is.err("walls", format!("{missing} perimeter edges have no wall"));
    }

    // Start and connectivity.
    let start = m.zones.start;
    let start_ok = start.col < w && start.row < h;
    if !start_ok {
        is.err("zones.start", "start outside the grid".into());
        return is.finish(0, 0);
    }
    let st = &m.tiles[idx(w, start)];
    if st.kind != TileKind::Start {
        is.err("zones.start", format!("start tile is {:?}", st.kind));
    }
    if st.in_dz {
        is.err(
            "zones.start",
            "start tile is inside the Dangerous Zone".into(),
        );
    }
    if !(start.col == 0 || start.row == 0 || start.col == w - 1 || start.row == h - 1) {
        is.err("zones.start", "start is not on the perimeter".into());
    }
    if m.tiles.iter().filter(|t| t.kind == TileKind::Start).count() != 1 {
        is.err("tiles", "expected exactly one Start tile".into());
    }
    let seen = reach(w, h, &m.tiles, &ws, start, false);
    let reachable = seen.iter().filter(|&&s| s).count() as u32;
    let enterable = m.tiles.iter().filter(|t| t.kind != TileKind::Black).count() as u32;
    let lost: Vec<TileXY> = m
        .tiles
        .iter()
        .zip(&seen)
        .filter(|(t, &s)| !s && t.kind != TileKind::Black)
        .map(|(t, _)| t.at)
        .collect();
    if !lost.is_empty() {
        is.err(
            "tiles",
            format!(
                "{} tiles unreachable from start, first {:?}",
                lost.len(),
                lost[0]
            ),
        );
    }

    // Checkpoints.
    let silver: BTreeSet<TileXY> = m
        .tiles
        .iter()
        .filter(|t| t.kind == TileKind::Silver)
        .map(|t| t.at)
        .collect();
    let listed: BTreeSet<TileXY> = m.zones.checkpoints.iter().copied().collect();
    if silver != listed || listed.len() != m.zones.checkpoints.len() {
        is.err(
            "zones.checkpoints",
            "must list exactly the silver tiles".into(),
        );
    }

    check_dz(m, &ws, &mut is);

    // Tile features and heightfield.
    for t in &m.tiles {
        let path = format!("tiles[{},{}]", t.at.col, t.at.row);
        if t.features.len() > 1 {
            is.err(&path, "more than one feature on a tile".into());
        }
        if !t.features.is_empty() && t.kind == TileKind::Black {
            is.err(&path, "feature on a black tile".into());
        }
        let lateral_walled = |up: Dir| {
            Dir::ALL
                .iter()
                .filter(|&&d| d != up && d != up.opposite())
                .all(|&d| ws.has(t.at, d))
        };
        for feat in &t.features {
            match *feat {
                TileFeature::Ramp {
                    up_dir,
                    angle_deg,
                    rise_m,
                } => {
                    let real = rise_m.atan2(f.tile_m).to_degrees();
                    if angle_deg > f.ramp_max_deg || real > f.ramp_max_deg + EPS {
                        is.err(
                            &path,
                            format!("ramp angle {angle_deg} exceeds {}", f.ramp_max_deg),
                        );
                    }
                    if (real - angle_deg).abs() > 0.5 || rise_m < 0.0 {
                        is.err(&path, "ramp angle and rise disagree".into());
                    }
                    if !lateral_walled(up_dir) {
                        is.err(&path, "ramp side edges must be walled".into());
                    }
                }
                TileFeature::Stairs {
                    up_dir,
                    step_m,
                    tread_m,
                    steps,
                } => {
                    let max_inc = if t.in_dz {
                        f.stairs_incline_max_deg_dz
                    } else {
                        f.stairs_incline_max_deg
                    };
                    if steps == 0 || step_m > f.stairs_step_max_m + EPS {
                        is.err(&path, format!("stair step {step_m} too high"));
                    }
                    if tread_m < f.stairs_tread_min_m - EPS {
                        is.err(&path, format!("stair tread {tread_m} too short"));
                    }
                    if step_m.atan2(tread_m).to_degrees() >= max_inc {
                        is.err(&path, "stair incline too steep".into());
                    }
                    if !lateral_walled(up_dir) {
                        is.err(&path, "stairs must sit between walls".into());
                    }
                }
                TileFeature::SpeedBump { height_m, .. } => {
                    let max = if t.in_dz {
                        f.speed_bump_max_m_dz
                    } else {
                        f.speed_bump_max_m
                    };
                    if height_m > max + EPS || height_m <= 0.0 {
                        is.err(
                            &path,
                            format!("speed bump height {height_m} outside 0..={max}"),
                        );
                    }
                    if !t.in_dz
                        && t.features.iter().any(|x| {
                            matches!(x, TileFeature::Ramp { .. } | TileFeature::Stairs { .. })
                        })
                    {
                        is.err(&path, "speed bump on a ramp or stairs tile".into());
                    }
                }
            }
        }
    }
    for t in &m.tiles {
        for side in [Dir::East, Dir::North] {
            let Some(n) = t.at.step(side, w, h) else {
                continue;
            };
            let nt = &m.tiles[idx(w, n)];
            if ws.has(t.at, side) || t.kind == TileKind::Black || nt.kind == TileKind::Black {
                continue;
            }
            match (edge_height(t, side), edge_height(nt, side.opposite())) {
                (Some(a), Some(b)) if (a - b).abs() <= EPS => {}
                (Some(a), Some(b)) => is.err(
                    "heightfield",
                    format!("open edge {:?}/{side:?} joins floors at {a} and {b}", t.at),
                ),
                _ => is.err(
                    "heightfield",
                    format!("open side edge of a ramp or stairs at {:?}/{side:?}", t.at),
                ),
            }
        }
    }

    check_obstacles(m, rules, &ws, &mut is);
    check_victims(m, rules, &ws, &mut is);

    // Cached classes must match.
    let lin = Linearity::new(w, h, &m.tiles, &ws);
    let stale = m
        .tiles
        .iter()
        .enumerate()
        .filter(|(i, t)| t.class != lin.tile_class(*i))
        .count();
    if stale > 0 {
        is.err(
            "tiles",
            format!("{stale} tiles carry a stale linear/floating class"),
        );
    }
    is.finish(reachable, enterable)
}

fn check_dz(m: &MazeGraph, ws: &WallSet, is: &mut Issues) {
    let (w, h) = (m.grid.w, m.grid.h);
    let in_dz: BTreeSet<TileXY> = m.tiles.iter().filter(|t| t.in_dz).map(|t| t.at).collect();
    let reds: Vec<TileXY> = m
        .tiles
        .iter()
        .filter(|t| t.kind == TileKind::Red)
        .map(|t| t.at)
        .collect();
    let Some(dz) = &m.zones.dangerous_zone else {
        if !in_dz.is_empty() || !reds.is_empty() {
            is.err(
                "zones.dangerous_zone",
                "DZ tiles present but no zone declared".into(),
            );
        }
        return;
    };
    let declared: BTreeSet<TileXY> = dz.tiles.iter().copied().collect();
    if declared != in_dz || declared.is_empty() {
        is.err(
            "zones.dangerous_zone",
            "tiles do not match the in_dz flags".into(),
        );
    }
    if dz.entrance.col >= w || dz.entrance.row >= h {
        is.err("zones.dangerous_zone", "entrance outside the grid".into());
        return;
    }
    if reds != vec![dz.entrance] {
        is.err(
            "zones.dangerous_zone",
            "exactly one Red tile, the entrance, expected".into(),
        );
    }
    if in_dz.contains(&dz.entrance) {
        is.err(
            "zones.dangerous_zone",
            "entrance must lie outside the zone tiles".into(),
        );
    }
    let mut doors = 0;
    for &t in &in_dz {
        if m.tiles[idx(w, t)].kind == TileKind::Black {
            is.err("zones.dangerous_zone", "black tile inside the zone".into());
        }
        for d in Dir::ALL {
            if ws.has(t, d) {
                continue;
            }
            match t.step(d, w, h) {
                Some(n) if in_dz.contains(&n) => {}
                Some(n) if n == dz.entrance => doors += 1,
                _ => is.err(
                    "zones.dangerous_zone",
                    format!("zone tile {t:?} is open toward {d:?} besides the entrance"),
                ),
            }
        }
    }
    if doors != 1 {
        is.err(
            "zones.dangerous_zone",
            format!("{doors} doors into the zone, expected 1"),
        );
    }
    let seen = reach(w, h, &m.tiles, ws, m.zones.start, true);
    let blocked = m
        .tiles
        .iter()
        .zip(&seen)
        .filter(|(t, &s)| !s && !t.in_dz && t.kind != TileKind::Black)
        .count();
    if blocked > 0 {
        is.err(
            "zones.dangerous_zone",
            format!("{blocked} tiles are only reachable through the zone"),
        );
    }
}

fn check_obstacles(m: &MazeGraph, rules: &RulesProfile, ws: &WallSet, is: &mut Issues) {
    let f = &rules.field;
    let w = m.grid.w;
    for (i, o) in m.obstacles.iter().enumerate() {
        let path = format!("obstacles[{i}]");
        if m.tile_at(o.center) != Some(o.tile) {
            is.err(&path, "center is not inside its tile".into());
            continue;
        }
        let t = &m.tiles[idx(w, o.tile)];
        if t.kind == TileKind::Black || !t.features.is_empty() {
            is.err(
                &path,
                "obstacle on a black, ramp, stairs or bump tile".into(),
            );
        }
        if o.height_m < f.obstacle_min_height_m - EPS {
            is.err(
                &path,
                format!("height {} below {}", o.height_m, f.obstacle_min_height_m),
            );
        }
        if o.radius_m <= 0.0 {
            is.err(&path, "radius must be positive".into());
        }
        let (x0, y0) = (o.tile.col as f64 * f.tile_m, o.tile.row as f64 * f.tile_m);
        let mut touching = false;
        let mut min_gap = f64::INFINITY;
        for d in Dir::ALL {
            if !ws.has(o.tile, d) {
                continue;
            }
            let dist = match d {
                Dir::North => y0 + f.tile_m - o.center.y,
                Dir::South => o.center.y - y0,
                Dir::East => x0 + f.tile_m - o.center.x,
                Dir::West => o.center.x - x0,
            };
            let gap = dist - o.radius_m;
            if gap < -EPS {
                is.err(&path, "obstacle overlaps a wall".into());
            }
            if gap.abs() <= EPS {
                touching = true;
            }
            min_gap = min_gap.min(gap);
        }
        if !touching && min_gap < f.obstacle_wall_clearance_m - EPS {
            is.err(
                &path,
                format!(
                    "obstacle is {min_gap:.3} m from a wall: neither touching nor {} m clear",
                    f.obstacle_wall_clearance_m
                ),
            );
        }
    }
}

fn check_victims(m: &MazeGraph, rules: &RulesProfile, ws: &WallSet, is: &mut Issues) {
    let (w, h) = (m.grid.w, m.grid.h);
    let f = &rules.field;
    let lin = Linearity::new(w, h, &m.tiles, ws);
    let mut ids = BTreeSet::new();
    let mut faces = BTreeSet::new();
    for (i, v) in m.victims.iter().enumerate() {
        let path = format!("victims[{i}]");
        if !ids.insert(v.id) {
            is.err(&path, format!("duplicate victim id {}", v.id));
        }
        if v.tile.col >= w || v.tile.row >= h {
            is.err(&path, "tile outside the grid".into());
            continue;
        }
        if !faces.insert(canonical(v.tile, v.wall_side)) {
            is.err(&path, "two victims on one physical wall".into());
        }
        let t = &m.tiles[idx(w, v.tile)];
        if matches!(
            t.kind,
            TileKind::Black | TileKind::Silver | TileKind::Blue | TileKind::Red
        ) || !t.features.is_empty()
            || m.obstacles.iter().any(|o| o.tile == v.tile)
        {
            is.err(&path, format!("victim on an illegal tile ({:?})", t.kind));
        }
        if !ws.has(v.tile, v.wall_side) {
            is.err(&path, "no wall on the victim's side".into());
            continue;
        }
        let (a, b) = edge_points(v.tile, v.wall_side, f.tile_m);
        let on_plane = match v.wall_side {
            Dir::North | Dir::South => {
                (v.pos.y - a.y).abs() < EPS && v.pos.x >= a.x - EPS && v.pos.x <= b.x + EPS
            }
            Dir::East | Dir::West => {
                (v.pos.x - a.x).abs() < EPS && v.pos.y >= a.y - EPS && v.pos.y <= b.y + EPS
            }
        };
        if !on_plane {
            is.err(&path, "position is not on the wall face".into());
        }
        if (v.pos.z - (t.height_ref + f.victim_height_m)).abs() > EPS {
            is.err(
                &path,
                format!(
                    "height {} is not {} above the floor",
                    v.pos.z, f.victim_height_m
                ),
            );
        }
        if (v.facing_yaw - v.wall_side.opposite().yaw()).abs() > EPS {
            is.err(&path, "victim does not face into the tile".into());
        }
        let (status, counts) = match &v.kind {
            VictimKind::Smiley { status } => (Some(*status), true),
            VictimKind::Cognitive { rings } => {
                let s = status_for_sum(ring_sum(rules, rings));
                (s, s.is_some())
            }
            VictimKind::Decoy => (None, false),
        };
        if v.status != status {
            is.err(
                &path,
                format!(
                    "status {:?} does not match the target ({status:?})",
                    v.status
                ),
            );
        }
        let tier = counts.then(|| {
            if lin.edge_linear(v.tile, v.wall_side) {
                VictimTier::Linear
            } else {
                VictimTier::Floating
            }
        });
        if v.tier != tier {
            is.err(&path, format!("tier {:?} should be {tier:?}", v.tier));
        }
    }
}
