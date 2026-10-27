//! The simulator: fixed-timestep loop tying world, physics, sensors, controller and game.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use mm_core::{
    Commands, Controller, Dir, EndReason, ErrorProfile, Event, EventKind, Frame, MazeGraph,
    Metrics, Observations, Pose3, Replay, ReplayHeader, RobotConfig, RobotInfo, RulesProfile,
    RunSettings, ScoreBreakdown, TileKind, TileXY, Vec2, VictimKind, REPLAY_FORMAT_VERSION,
};

use crate::game::{GameCtx, GameState};
use crate::kinematics::{KinematicBackend, PhysicsBackend};
use crate::rng::SimRng;
use crate::sensors::camera::CameraArray;
use crate::sensors::color::ColorArray;
use crate::sensors::imu::{wrap_pi, Imu};
use crate::sensors::tof::TofArray;
use crate::world::World;

#[derive(Debug, thiserror::Error)]
pub enum SimError {
    #[error("invalid maze: {0}")]
    Maze(String),
    #[error("invalid robot: {0}")]
    Robot(String),
    #[error("invalid timing: {0}")]
    Timing(String),
}

/// Hashes of the inputs, computed by the caller (see `replay::config_hash`).
#[derive(Debug, Clone, Default)]
pub struct ConfigHashes {
    pub robot: String,
    pub maze: String,
    pub controller: String,
    pub error_profile: String,
    pub rules: String,
}

#[derive(Debug, Clone)]
pub struct SimInput {
    pub maze: MazeGraph,
    pub robot: RobotConfig,
    pub rules: RulesProfile,
    pub errors: ErrorProfile,
    pub settings: RunSettings,
    pub params: BTreeMap<String, f64>,
    pub run_id: String,
    pub hashes: ConfigHashes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    Running,
    Ended(EndReason),
}

pub struct Sim {
    rules: RulesProfile,
    robot: RobotConfig,
    params: BTreeMap<String, f64>,
    lop_enabled: bool,
    header: ReplayHeader,
    world: World,
    backend: Box<dyn PhysicsBackend>,
    tof: TofArray,
    imu: Imu,
    color: ColorArray,
    camera: CameraArray,
    game: GameState,
    start: TileXY,
    controller_dt: f64,
    physics_dt: f64,
    substeps: u32,
    frame_stride: u64,
    total_ticks: u64,
    tick: u64,
    t: f64,
    id_light: bool,
    events: Vec<Event>,
    cursor: usize,
    frames: Vec<Frame>,
    last_frame_tick: Option<u64>,
    cur_tile: Option<TileXY>,
    visited: BTreeSet<TileXY>,
    reachable: u32,
    restarts: u32,
    collisions: u32,
    distance: f64,
    lop_ref: Vec2,
    lop_ref_t: f64,
    lop_flag: bool,
    initialised: bool,
    ended: Option<EndReason>,
    final_score: Option<ScoreBreakdown>,
}

impl Sim {
    pub fn new(cfg: SimInput) -> Result<Sim, SimError> {
        let SimInput {
            maze,
            robot,
            rules,
            errors,
            settings,
            params,
            run_id,
            hashes,
        } = cfg;
        if maze.tiles.len() != (maze.grid.w * maze.grid.h) as usize {
            return Err(SimError::Maze("tiles.len() != w*h".into()));
        }
        if maze.idx(maze.zones.start).is_none() {
            return Err(SimError::Maze("start tile outside the grid".into()));
        }
        if maze.header.tile_m <= 0.0 {
            return Err(SimError::Maze("tile_m must be positive".into()));
        }
        robot
            .check(&rules)
            .map_err(|e| SimError::Robot(e.to_string()))?;
        let tm = &settings.timing;
        if tm.physics_dt_s <= 0.0 || tm.controller_dt_s < tm.physics_dt_s || tm.frame_dt_s <= 0.0 {
            return Err(SimError::Timing(
                "need 0 < physics <= controller, frame > 0".into(),
            ));
        }
        let ratio = tm.controller_dt_s / tm.physics_dt_s;
        let substeps = ratio.round() as u32;
        if (ratio - substeps as f64).abs() > 1e-6 {
            return Err(SimError::Timing(
                "controller_dt must be a whole multiple of physics_dt".into(),
            ));
        }
        let frame_stride = ((tm.frame_dt_s / tm.controller_dt_s).round() as u64).max(1);
        let duration = settings.max_duration_s.unwrap_or(rules.play.run_duration_s);
        let total_ticks = (duration / tm.controller_dt_s + 1e-9).floor() as u64;

        let rng = SimRng::new(settings.seed);
        let world = World::new(&maze, &rules, &errors, &rng);
        let start = maze.zones.start;
        let start_c = start.center(world.tile_m);
        let backend = KinematicBackend::new(
            &robot,
            &errors.wheel,
            &rng,
            &world,
            start_c,
            maze.zones.start_yaw,
        );
        let mut imu = Imu::new(robot.imu, &errors.imu, &rng);
        imu.update(0.0, backend.body(), false);
        let mut tof = TofArray::new(&robot, &errors.tof, &rng);
        tof.sample_due(0.0, backend.body(), &world);

        let header = ReplayHeader {
            format_version: REPLAY_FORMAT_VERSION,
            run_id,
            seed: settings.seed,
            rng_mode: settings.rng_mode,
            robot_hash: hashes.robot,
            maze_hash: hashes.maze,
            controller_hash: hashes.controller,
            error_profile_hash: hashes.error_profile,
            rules_hash: hashes.rules,
            applied_tolerance_frac: world.applied_tolerance,
            physics_dt_s: tm.physics_dt_s,
            controller_dt_s: tm.controller_dt_s,
            frame_dt_s: tm.frame_dt_s,
            created_at: None,
            backend_version: env!("CARGO_PKG_VERSION").to_string(),
        };
        let reachable = count_reachable(&world.maze);
        let game = GameState::new(&rules, &robot);
        let mut sim = Sim {
            rules,
            params,
            lop_enabled: settings.lop_enabled,
            header,
            backend: Box::new(backend),
            tof,
            imu,
            color: ColorArray::new(&robot),
            camera: CameraArray::new(&robot),
            game,
            start,
            controller_dt: tm.controller_dt_s,
            physics_dt: tm.physics_dt_s,
            substeps,
            frame_stride,
            total_ticks,
            tick: 0,
            t: 0.0,
            id_light: false,
            events: Vec::new(),
            cursor: 0,
            frames: Vec::new(),
            last_frame_tick: None,
            cur_tile: None,
            visited: BTreeSet::new(),
            reachable,
            restarts: 0,
            collisions: 0,
            distance: 0.0,
            lop_ref: start_c,
            lop_ref_t: 0.0,
            lop_flag: false,
            initialised: false,
            ended: None,
            final_score: None,
            robot,
            world,
        };
        sim.push(0.0, EventKind::RunStart);
        sim.update_tile(0.0);
        Ok(sim)
    }

    pub fn header(&self) -> &ReplayHeader {
        &self.header
    }

    pub fn time(&self) -> f64 {
        self.t
    }

    pub fn is_ended(&self) -> Option<EndReason> {
        self.ended
    }

    /// The perturbed maze instance this run uses.
    pub fn world(&self) -> &World {
        &self.world
    }

    /// Advance one controller tick.
    pub fn step(&mut self, ctrl: &mut dyn Controller) -> StepOutcome {
        if let Some(r) = self.ended {
            return StepOutcome::Ended(r);
        }
        if !self.initialised {
            self.initialised = true;
            let info = RobotInfo {
                robot: self.robot.clone(),
                tile_m: self.world.tile_m,
                controller_dt_s: self.controller_dt,
                params: self.params.clone(),
            };
            if let Err(m) = ctrl.init(&info) {
                return self.fault(m);
            }
        }
        if self.tick.is_multiple_of(self.frame_stride) {
            self.record_frame();
        }
        let obs = self.build_obs();
        let cmds = match ctrl.step(&obs) {
            Ok(c) => c,
            Err(m) => return self.fault(m),
        };
        self.apply_commands(&cmds);
        self.advance();
        if cmds.end_run {
            self.finish(EndReason::Exited)
        } else if self.tick >= self.total_ticks {
            self.finish(EndReason::TimeUp)
        } else {
            StepOutcome::Running
        }
    }

    /// Run to the end and return the replay.
    pub fn run(mut self, ctrl: &mut dyn Controller) -> Replay {
        while let StepOutcome::Running = self.step(ctrl) {}
        self.into_replay()
    }

    /// External stop (user pressed stop).
    pub fn stop(&mut self) {
        if self.ended.is_none() {
            self.finish(EndReason::Stopped);
        }
    }

    /// Current state as a frame (ToF holds samples not yet in a recorded frame).
    pub fn snapshot(&self) -> Frame {
        self.make_frame(self.tof.peek_frame())
    }

    /// Events pushed since the previous call, for live streaming.
    pub fn events_since(&mut self) -> Vec<Event> {
        let v = self.events[self.cursor..].to_vec();
        self.cursor = self.events.len();
        v
    }

    /// Frames recorded so far.
    pub fn frames(&self) -> &[Frame] {
        &self.frames
    }

    pub fn replay(&self) -> Replay {
        Replay {
            header: self.header.clone(),
            frames: self.frames.clone(),
            events: self.events.clone(),
        }
    }

    pub fn into_replay(self) -> Replay {
        Replay {
            header: self.header,
            frames: self.frames,
            events: self.events,
        }
    }

    pub fn metrics(&self) -> Metrics {
        let reachable = self.reachable;
        Metrics {
            score: self
                .final_score
                .clone()
                .unwrap_or_else(|| self.game.score_now()),
            duration_s: self.t,
            exited: self.ended == Some(EndReason::Exited),
            restarts: self.restarts,
            collisions: self.collisions,
            tiles_visited: self.visited.len() as u32,
            tiles_reachable: reachable,
            coverage: if reachable > 0 {
                self.visited.len() as f64 / reachable as f64
            } else {
                0.0
            },
            victims_total: self
                .world
                .maze
                .victims
                .iter()
                .filter(|v| !matches!(v.kind, VictimKind::Decoy))
                .count() as u32,
            distance_m: self.distance,
            ..Default::default()
        }
    }

    // ---- internals ----

    fn push(&mut self, t: f64, kind: EventKind) {
        self.events.push(Event { t, kind });
    }

    fn pose(&self) -> Pose3 {
        let b = self.backend.body();
        Pose3 {
            x: b.x,
            y: b.y,
            z: b.z,
            roll: b.roll,
            pitch: b.pitch,
            yaw: wrap_pi(b.yaw),
        }
    }

    fn make_frame(&self, tof: Vec<mm_core::TofReading>) -> Frame {
        Frame {
            t: self.t,
            pose: self.pose(),
            wheels: self.backend.wheel_states(),
            imu: self.imu.reading(),
            tof,
            color: Vec::new(),
            cams: Vec::new(),
            id_light: self.id_light,
        }
    }

    fn record_frame(&mut self) {
        if self.last_frame_tick == Some(self.tick) {
            return;
        }
        let tof = self.tof.drain_frame();
        let mut f = self.make_frame(tof);
        f.color = self.color.drain_frame();
        f.cams = self.camera.drain_frame();
        self.frames.push(f);
        self.last_frame_tick = Some(self.tick);
    }

    fn build_obs(&mut self) -> Observations {
        Observations {
            t: self.t,
            tof: self.tof.observe(),
            color: self.color.observe(),
            cams: self.camera.observe(),
            imu: self.imu.reading(),
            encoders: self.backend.encoders(),
            kits_left: self.game.kits_left,
            lop_restarted: std::mem::take(&mut self.lop_flag),
        }
    }

    fn apply_commands(&mut self, cmds: &Commands) {
        let t = self.t;
        if let Some(text) = &cmds.note {
            self.push(t, EventKind::Note { text: text.clone() });
        }
        let ctx = GameCtx {
            t,
            rules: &self.rules,
            maze: &self.world.maze,
            pose: self.pose(),
            speed: self.backend.body().v,
        };
        let mut out = Vec::new();
        self.game.on_commands(&ctx, cmds, &mut out);
        for k in out {
            self.push(t, k);
        }
        self.backend.set_targets(cmds.wheel_v);
        self.id_light = cmds.id_light;
    }

    fn advance(&mut self) {
        let mut jolt = false;
        for k in 1..=self.substeps {
            let ts = self.t + k as f64 * self.physics_dt;
            let info = self.backend.step(&self.world, ts, self.physics_dt);
            self.distance += info.moved;
            jolt |= info.jolt;
            if let Some(p) = info.collision {
                self.collisions += 1;
                self.push(ts, EventKind::Collision { x: p.x, y: p.y });
            }
            self.update_tile(ts);
            self.tof.sample_due(ts, self.backend.body(), &self.world);
        }
        self.tick += 1;
        self.t = self.tick as f64 * self.controller_dt;
        self.imu
            .update(self.controller_dt, self.backend.body(), jolt);
        self.check_lop();
    }

    fn update_tile(&mut self, ts: f64) {
        let b = self.backend.body();
        let now = self.world.maze.tile_at(Vec2 { x: b.x, y: b.y });
        if now == self.cur_tile {
            return;
        }
        if let Some(old) = self.cur_tile {
            self.push(ts, EventKind::TileExit { tile: old });
        }
        self.cur_tile = now;
        let Some(tile) = now else { return };
        self.push(ts, EventKind::TileEnter { tile });
        let kind = self
            .world
            .maze
            .tile(tile)
            .map_or(TileKind::Normal, |t| t.kind);
        if kind != TileKind::Black {
            self.visited.insert(tile);
        }
        let ctx = GameCtx {
            t: ts,
            rules: &self.rules,
            maze: &self.world.maze,
            pose: self.pose(),
            speed: self.backend.body().v,
        };
        let mut out = Vec::new();
        self.game.on_tile_enter(&ctx, tile, kind, &mut out);
        for k in out {
            self.push(ts, k);
        }
        if kind == TileKind::Black {
            self.push(ts, EventKind::EnteredBlack { tile });
            self.lop_restart(ts);
        }
    }

    fn check_lop(&mut self) {
        if !self.lop_enabled {
            return;
        }
        let b = self.backend.body();
        let d = ((b.x - self.lop_ref.x).powi(2) + (b.y - self.lop_ref.y).powi(2)).sqrt();
        if d > self.rules.play.lop_min_displacement_m {
            self.lop_ref = Vec2 { x: b.x, y: b.y };
            self.lop_ref_t = self.t;
        } else if self.t - self.lop_ref_t >= self.rules.play.lop_no_progress_s {
            self.lop_restart(self.t);
        }
    }

    /// Teleport to the last visited checkpoint (or start) keeping yaw.
    fn lop_restart(&mut self, ts: f64) {
        let dest = self.game.last_checkpoint.unwrap_or(self.start);
        let c = dest.center(self.world.tile_m);
        self.backend.teleport(&self.world, c);
        self.restarts += 1;
        self.lop_flag = true;
        self.lop_ref = c;
        self.lop_ref_t = self.t.max(ts);
        self.game.on_lop_restart();
        if let Some(old) = self.cur_tile {
            if old != dest {
                self.push(ts, EventKind::TileExit { tile: old });
            }
        }
        self.push(
            ts,
            EventKind::LopRestart {
                to: dest,
                restarts: self.restarts,
            },
        );
        if self.cur_tile != Some(dest) {
            self.push(ts, EventKind::TileEnter { tile: dest });
        }
        self.cur_tile = Some(dest);
    }

    fn fault(&mut self, message: String) -> StepOutcome {
        self.push(self.t, EventKind::ControllerError { message });
        self.finish(EndReason::ControllerFault)
    }

    fn finish(&mut self, reason: EndReason) -> StepOutcome {
        self.record_frame();
        let ctx = GameCtx {
            t: self.t,
            rules: &self.rules,
            maze: &self.world.maze,
            pose: self.pose(),
            speed: self.backend.body().v,
        };
        let mut out = Vec::new();
        let score = self.game.finish(&ctx, reason, &mut out);
        let t = self.t;
        for k in out {
            self.push(t, k);
        }
        self.push(
            t,
            EventKind::RunEnd {
                reason,
                score: score.clone(),
            },
        );
        self.final_score = Some(score);
        self.ended = Some(reason);
        StepOutcome::Ended(reason)
    }
}

/// Tiles reachable from the start over open edges, black tiles excluded.
fn count_reachable(maze: &MazeGraph) -> u32 {
    let mut blocked: BTreeSet<(TileXY, Dir)> = BTreeSet::new();
    for w in &maze.walls {
        if let Some(e) = w.edge {
            blocked.insert((e.tile, e.side));
            if let Some(o) = e.tile.step(e.side, maze.grid.w, maze.grid.h) {
                blocked.insert((o, e.side.opposite()));
            }
        }
    }
    let is_black = |t: TileXY| maze.tile(t).is_none_or(|x| x.kind == TileKind::Black);
    let start = maze.zones.start;
    if is_black(start) {
        return 0;
    }
    let mut seen = BTreeSet::from([start]);
    let mut q = VecDeque::from([start]);
    while let Some(t) = q.pop_front() {
        for d in Dir::ALL {
            if blocked.contains(&(t, d)) {
                continue;
            }
            if let Some(n) = t.step(d, maze.grid.w, maze.grid.h) {
                if !is_black(n) && seen.insert(n) {
                    q.push_back(n);
                }
            }
        }
    }
    seen.len() as u32
}
