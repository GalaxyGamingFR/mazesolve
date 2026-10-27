//! Rules-level game state: scoring and run bookkeeping (RCJ Rescue Maze 2027 draft §5.6).
//!
//! Every hook receives a `GameCtx` with the world and ground truth, and pushes
//! `EventKind`s that the sim time-stamps. All numbers come from the `RulesProfile`.
//! See `docs/SCORING.md` for the rule -> code mapping.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use mm_core::{
    Commands, Dir, EndReason, EventKind, MazeGraph, Pose3, RingColor, RobotConfig, RulesProfile,
    ScoreBreakdown, TileClass, TileFeature, TileKind, TileXY, Vec2, Victim, VictimKind,
    VictimStatus, VictimTier,
};

/// Fraction of a feature tile's length the robot centre must cover between entering and
/// leaving for the feature to count as traversed (the centre crosses edge to edge, ~1.0).
const TRAVERSE_MIN_FRAC: f64 = 0.75;
/// Minimum share of the identification window that must match the blink pattern.
const BLINK_MATCH_MIN: f64 = 0.8;

/// Read-only context for the hooks.
pub struct GameCtx<'a> {
    pub t: f64,
    /// Controller tick length, seconds.
    pub dt: f64,
    pub rules: &'a RulesProfile,
    pub maze: &'a MazeGraph,
    /// Ground-truth pose.
    pub pose: Pose3,
    /// Ground-truth speed, m/s.
    pub speed: f64,
}

impl GameCtx<'_> {
    fn still(&self) -> bool {
        self.speed.abs() < self.rules.play.stationary_mps
    }

    fn tile(&self) -> Option<TileXY> {
        self.maze.tile_at(Vec2 {
            x: self.pose.x,
            y: self.pose.y,
        })
    }
}

/// Counters mirrored into `Metrics`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GameStats {
    pub victims_identified: u32,
    pub misidentifications: u32,
    pub decoy_false_positives: u32,
    pub kits_used: u32,
}

#[derive(Debug, Clone, Copy)]
struct Sample {
    light: bool,
    still: bool,
}

#[derive(Debug, Clone, Copy)]
struct BlueVisit {
    tile: TileXY,
    still_s: f64,
    ok: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum FeatKind {
    Ramp,
    Stairs,
    Bump,
}

/// Entry state of the feature tile the robot is currently crossing.
#[derive(Debug, Clone)]
struct Traverse {
    tile: TileXY,
    /// (kind, travel axis, progress at entry)
    feats: Vec<(FeatKind, Dir, f64)>,
}

#[derive(Debug, Clone)]
pub struct GameState {
    pub score: ScoreBreakdown,
    pub kits_left: u32,
    /// Last silver tile visited (LoP teleports here).
    pub last_checkpoint: Option<TileXY>,
    pub lop_count: u32,
    pub stats: GameStats,
    checkpoints_seen: BTreeSet<TileXY>,
    hist: VecDeque<Sample>,
    /// Victims that have been reported once (right or wrong): each scores at most once.
    attempted: BTreeSet<u32>,
    identified: BTreeSet<u32>,
    last_identified: Option<u32>,
    kits_on: BTreeMap<u32, u32>,
    srd: u32,
    blue: Option<BlueVisit>,
    /// Running total per blue tile (first ok visit, minus penalty per later ok visit).
    blue_total: BTreeMap<TileXY, i32>,
    trav: Option<Traverse>,
    rewarded: BTreeSet<(TileXY, FeatKind)>,
    ramps_n: u32,
    stairs_n: u32,
    exit_ticks: u32,
    exit_ready: bool,
}

fn progress(dir: Dir, u: f64, v: f64, t: f64) -> f64 {
    match dir {
        Dir::East => u / t,
        Dir::West => 1.0 - u / t,
        Dir::North => v / t,
        Dir::South => 1.0 - v / t,
    }
}

fn dist_xy(p: &Pose3, v: &Victim) -> f64 {
    ((p.x - v.pos.x).powi(2) + (p.y - v.pos.y).powi(2)).sqrt()
}

/// Status encoded by cognitive ring values (same table as `mm-maze`).
fn status_for_sum(sum: i32) -> Option<VictimStatus> {
    match sum {
        2 => Some(VictimStatus::Harmed),
        1 => Some(VictimStatus::Stable),
        0 => Some(VictimStatus::Unharmed),
        _ => None,
    }
}

/// Ground-truth status of a target; `None` for decoys and invalid ring combinations.
fn truth_status(rules: &RulesProfile, v: &Victim) -> Option<VictimStatus> {
    match &v.kind {
        VictimKind::Decoy => None,
        VictimKind::Smiley { status } => v.status.or(Some(*status)),
        VictimKind::Cognitive { rings } => {
            let sum: i32 = rings.iter().map(|&c: &RingColor| rules.ring_value(c)).sum();
            status_for_sum(sum).or(v.status)
        }
    }
}

fn victim_points(rules: &RulesProfile, maze: &MazeGraph, v: &Victim) -> i32 {
    let tier = v.tier.unwrap_or_else(|| {
        match maze
            .tile(v.tile)
            .map(|t| t.class)
            .unwrap_or(TileClass::Linear)
        {
            TileClass::Linear => VictimTier::Linear,
            TileClass::Floating => VictimTier::Floating,
        }
    });
    let p = &rules.scoring.victim_points;
    let cognitive = matches!(v.kind, VictimKind::Cognitive { .. });
    match (tier, cognitive) {
        (VictimTier::Linear, false) => p.linear_smiley,
        (VictimTier::Linear, true) => p.linear_cognitive,
        (VictimTier::Floating, false) => p.floating_smiley,
        (VictimTier::Floating, true) => p.floating_cognitive,
    }
}

impl GameState {
    pub fn new(_rules: &RulesProfile, robot: &RobotConfig) -> Self {
        GameState {
            score: ScoreBreakdown::default(),
            kits_left: robot.kits,
            last_checkpoint: None,
            lop_count: 0,
            stats: GameStats::default(),
            checkpoints_seen: BTreeSet::new(),
            hist: VecDeque::new(),
            attempted: BTreeSet::new(),
            identified: BTreeSet::new(),
            last_identified: None,
            kits_on: BTreeMap::new(),
            srd: 0,
            blue: None,
            blue_total: BTreeMap::new(),
            trav: None,
            rewarded: BTreeSet::new(),
            ramps_n: 0,
            stairs_n: 0,
            exit_ticks: 0,
            exit_ready: false,
        }
    }

    /// True once the exit conditions (§5.6.12) have been met; the sim then ends the run.
    pub fn exit_ready(&self) -> bool {
        self.exit_ready
    }

    /// Called once whenever the robot centre enters a tile (not for LoP teleports).
    pub fn on_tile_enter(
        &mut self,
        ctx: &GameCtx,
        tile: TileXY,
        kind: TileKind,
        out: &mut Vec<EventKind>,
    ) {
        if kind == TileKind::Silver && self.checkpoints_seen.insert(tile) {
            self.score.checkpoints += ctx.rules.scoring.checkpoint;
            self.last_checkpoint = Some(tile);
            out.push(EventKind::Checkpoint { tile });
        } else if kind == TileKind::Silver {
            self.last_checkpoint = Some(tile);
        }
        if kind == TileKind::Blue {
            self.blue = Some(BlueVisit {
                tile,
                still_s: 0.0,
                ok: false,
            });
        }
        self.trav = None;
        if let Some(t) = ctx.maze.tile(tile) {
            let tm = ctx.maze.header.tile_m;
            let u = ctx.pose.x - tile.col as f64 * tm;
            let v = ctx.pose.y - tile.row as f64 * tm;
            let feats: Vec<_> = t
                .features
                .iter()
                .map(|f| match *f {
                    TileFeature::Ramp { up_dir, .. } => {
                        (FeatKind::Ramp, up_dir, progress(up_dir, u, v, tm))
                    }
                    TileFeature::Stairs { up_dir, .. } => {
                        (FeatKind::Stairs, up_dir, progress(up_dir, u, v, tm))
                    }
                    TileFeature::SpeedBump { across, .. } => {
                        (FeatKind::Bump, across, progress(across, u, v, tm))
                    }
                })
                .collect();
            if !feats.is_empty() {
                self.trav = Some(Traverse { tile, feats });
            }
        }
    }

    /// Called when the robot centre leaves `tile` (not for LoP teleports). Returns true
    /// when the exit is an early blue-tile exit that must trigger a LoP restart (D2).
    pub fn on_tile_exit(&mut self, ctx: &GameCtx, tile: TileXY, out: &mut Vec<EventKind>) -> bool {
        if let Some(tr) = self.trav.take() {
            if tr.tile == tile {
                let tm = ctx.maze.header.tile_m;
                let u = ctx.pose.x - tile.col as f64 * tm;
                let v = ctx.pose.y - tile.row as f64 * tm;
                for (kind, axis, s_in) in tr.feats {
                    let moved = progress(axis, u, v, tm) - s_in;
                    if moved.abs() < TRAVERSE_MIN_FRAC || self.rewarded.contains(&(tile, kind)) {
                        continue;
                    }
                    let sc = &ctx.rules.scoring;
                    match kind {
                        FeatKind::Ramp if moved > 0.0 => {
                            self.rewarded.insert((tile, kind));
                            self.ramps_n += 1;
                            self.score.ramps += sc.ramp;
                            out.push(EventKind::RampTraversed {
                                tile,
                                up: true,
                                points: sc.ramp,
                            });
                        }
                        FeatKind::Ramp => {}
                        FeatKind::Stairs => {
                            self.rewarded.insert((tile, kind));
                            self.stairs_n += 1;
                            self.score.stairs += sc.stairs;
                            out.push(EventKind::StairsTraversed {
                                tile,
                                points: sc.stairs,
                            });
                        }
                        FeatKind::Bump => {
                            self.rewarded.insert((tile, kind));
                            self.score.speed_bumps += sc.speed_bump_tile;
                            out.push(EventKind::SpeedBumpTraversed {
                                tile,
                                points: sc.speed_bump_tile,
                            });
                        }
                    }
                }
            }
        }
        if let Some(b) = self.blue.take() {
            if b.tile == tile && !b.ok {
                out.push(EventKind::BlueStop { tile, ok: false });
                return true;
            }
        }
        false
    }

    /// Called every controller step with the commands just issued.
    pub fn on_commands(&mut self, ctx: &GameCtx, cmds: &Commands, out: &mut Vec<EventKind>) {
        let play = &ctx.rules.play;
        let still = ctx.still();
        self.hist.push_back(Sample {
            light: cmds.id_light,
            still,
        });
        let keep = ((play.identification_light_s / ctx.dt).round() as usize).max(1) + 2;
        while self.hist.len() > keep {
            self.hist.pop_front();
        }

        self.update_blue(ctx, still, out);

        // Exit (§5.6.12): on the start tile, light held on, stationary for exit_light_s.
        let on_start = ctx.tile() == Some(ctx.maze.zones.start);
        if on_start && cmds.id_light && still {
            self.exit_ticks += 1;
        } else {
            self.exit_ticks = 0;
        }
        let need = ((play.exit_light_s / ctx.dt - 1e-9).ceil() as u32).max(1);
        let satisfied = self.exit_ticks >= need;
        if satisfied {
            self.exit_ready = true;
        } else if cmds.end_run {
            out.push(EventKind::Note {
                text: "end_run ignored: exit needs the start tile, id_light on and stationary"
                    .into(),
            });
        }

        if let Some(r) = &cmds.report {
            self.handle_report(ctx, r.status, out);
        }
        if cmds.deploy_kits > 0 {
            self.handle_kits(ctx, cmds.deploy_kits, out);
        }
    }

    fn update_blue(&mut self, ctx: &GameCtx, still: bool, out: &mut Vec<EventKind>) {
        let Some(b) = self.blue.as_mut() else { return };
        if b.ok {
            return;
        }
        if still {
            b.still_s += ctx.dt;
        } else {
            b.still_s = 0.0;
        }
        if b.still_s + 1e-9 < ctx.rules.field.blue_stop_s {
            return;
        }
        b.ok = true;
        let tile = b.tile;
        let sc = &ctx.rules.scoring;
        let old = self.blue_total.get(&tile).copied();
        let new = match old {
            None => sc.blue_tile,
            Some(o) => (o - sc.blue_revisit_penalty).max(0),
        };
        self.blue_total.insert(tile, new);
        self.score.blue_tiles += new - old.unwrap_or(0);
        out.push(EventKind::BlueStop { tile, ok: true });
    }

    /// Did `id_light` follow the blink pattern, with the robot still, over the window?
    fn identification_valid(&self, ctx: &GameCtx) -> bool {
        let p = &ctx.rules.play;
        let n = (p.identification_light_s / ctx.dt).round() as usize;
        let on = (p.identification_blink_on_s / ctx.dt).round() as isize;
        let off = (p.identification_blink_off_s / ctx.dt).round() as isize;
        if n == 0 || on == 0 || off == 0 || self.hist.len() < n {
            return false;
        }
        let win: Vec<Sample> = self
            .hist
            .iter()
            .skip(self.hist.len() - n)
            .copied()
            .collect();
        if win.iter().any(|s| !s.still) {
            return false;
        }
        let period = on + off;
        for phase in 0..period {
            let expect = |j: isize| (j + phase).rem_euclid(period) < on;
            let matched = win
                .iter()
                .enumerate()
                .filter(|(i, s)| {
                    let i = *i as isize;
                    // +-1 tick of jitter per edge.
                    (i - 1..=i + 1).any(|j| expect(j) == s.light)
                })
                .count();
            if matched as f64 >= BLINK_MATCH_MIN * n as f64 {
                return true;
            }
        }
        false
    }

    fn penalise(&mut self, ctx: &GameCtx) {
        self.score.penalties += ctx.rules.scoring.misidentification;
        self.stats.misidentifications += 1;
    }

    fn handle_report(&mut self, ctx: &GameCtx, reported: VictimStatus, out: &mut Vec<EventKind>) {
        let valid = self.identification_valid(ctx);
        // One blink window identifies one target.
        self.hist.clear();
        if !valid {
            out.push(EventKind::Note {
                text: "report rejected: no valid stationary blink pattern".into(),
            });
            return;
        }
        let max_d = ctx.rules.scoring.identify_within_m;
        let robot_tile = ctx.tile();
        let near = |v: &Victim| {
            robot_tile.is_some_and(|rt| {
                rt.col.abs_diff(v.tile.col) <= 1 && rt.row.abs_diff(v.tile.row) <= 1
            })
        };
        let mut best: Option<(&Victim, f64)> = None;
        for v in &ctx.maze.victims {
            let d = dist_xy(&ctx.pose, v);
            if d <= max_d && near(v) && best.is_none_or(|(_, bd)| d < bd) {
                best = Some((v, d));
            }
        }
        let Some((v, _)) = best else {
            self.penalise(ctx);
            out.push(EventKind::Misidentification { reported });
            return;
        };
        if !self.attempted.insert(v.id) {
            out.push(EventKind::Note {
                text: format!("report ignored: victim {} already reported", v.id),
            });
            return;
        }
        let Some(truth) = truth_status(ctx.rules, v) else {
            self.stats.decoy_false_positives += 1;
            self.penalise(ctx);
            out.push(EventKind::DecoyFalsePositive { victim_id: v.id });
            return;
        };
        let correct = truth == reported;
        let mut points = 0;
        if correct {
            points = victim_points(ctx.rules, ctx.maze, v);
            self.score.victims += points;
            self.stats.victims_identified += 1;
            self.identified.insert(v.id);
            self.last_identified = Some(v.id);
        } else {
            self.penalise(ctx);
        }
        out.push(EventKind::VictimIdentified {
            victim_id: v.id,
            reported,
            correct,
            points,
        });
    }

    fn handle_kits(&mut self, ctx: &GameCtx, wanted: u32, out: &mut Vec<EventKind>) {
        let n = wanted.min(self.kits_left);
        if n == 0 {
            out.push(EventKind::Note {
                text: "deploy_kits ignored: no kits left".into(),
            });
            return;
        }
        self.kits_left -= n;
        self.stats.kits_used += n;
        let sc = &ctx.rules.scoring;
        let target = self
            .last_identified
            .and_then(|id| ctx.maze.victims.iter().find(|v| v.id == id))
            .filter(|v| dist_xy(&ctx.pose, v) <= ctx.rules.play.kit_max_distance_m);
        let Some(v) = target else {
            out.push(EventKind::KitDeployed {
                victim_id: None,
                count: n,
                points: 0,
            });
            return;
        };
        let cap = match truth_status(ctx.rules, v) {
            Some(VictimStatus::Harmed) => sc.kits_max.harmed,
            Some(VictimStatus::Stable) => sc.kits_max.stable,
            _ => sc.kits_max.unharmed,
        };
        let prev = self.kits_on.get(&v.id).copied().unwrap_or(0);
        let total = prev + n;
        self.kits_on.insert(v.id, total);
        let (before, after) = (prev.min(cap), total.min(cap));
        let pts_for = |k: u32| -> i32 {
            if k == 0 {
                0
            } else {
                sc.kit_points_by_count
                    .get(k as usize - 1)
                    .or(sc.kit_points_by_count.last())
                    .copied()
                    .unwrap_or(0)
            }
        };
        let points = pts_for(after) - pts_for(before);
        self.score.kits += points;
        self.srd += after - before;
        out.push(EventKind::KitDeployed {
            victim_id: Some(v.id),
            count: n,
            points,
        });
    }

    /// Called after a Lack-of-Progress restart (reliability penalty applies at finish).
    pub fn on_lop_restart(&mut self) {
        self.lop_count += 1;
        self.blue = None;
        self.trav = None;
        self.hist.clear();
        self.exit_ticks = 0;
    }

    /// Called once at run end: missed victims, exit bonus, reliability.
    pub fn finish(
        &mut self,
        ctx: &GameCtx,
        reason: EndReason,
        out: &mut Vec<EventKind>,
    ) -> ScoreBreakdown {
        for v in &ctx.maze.victims {
            if truth_status(ctx.rules, v).is_some() && !self.identified.contains(&v.id) {
                out.push(EventKind::VictimMissed { victim_id: v.id });
            }
        }
        let sc = &ctx.rules.scoring;
        let svi = self.stats.victims_identified as i32;
        let sbv = self.blue_total.len() as i32;
        if reason == EndReason::Exited {
            self.score.exit_bonus = sc.exit_per_svi * svi
                + sc.exit_per_sbv * sbv
                + sc.exit_per_ssn * self.stairs_n as i32
                + sc.exit_per_srn * self.ramps_n as i32;
        }
        self.score.reliability = (sc.reliability_per_svi * svi
            + sc.reliability_per_srd * self.srd as i32
            + sc.reliability_per_sbv * sbv
            - sc.reliability_per_lop * self.lop_count as i32)
            .max(0);
        self.score_now()
    }

    /// Current score with the total summed.
    pub fn score_now(&self) -> ScoreBreakdown {
        let mut s = self.score.clone();
        s.sum();
        s
    }
}
