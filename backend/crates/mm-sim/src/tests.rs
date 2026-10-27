//! Integration-style tests for the whole sim (hand-built mazes plus the small fixture).

use std::collections::BTreeMap;

use mm_core::*;

use crate::sensors::imu::wrap_pi;
use crate::*;

const FIXTURE: &str = include_str!("../../../fixtures/maze_small.json");
const T: f64 = 0.28;

fn fixture_maze() -> MazeGraph {
    serde_json::from_str(FIXTURE).expect("fixture parses")
}

fn wall(a: (f64, f64), b: (f64, f64), tile: TileXY, side: Dir) -> Wall {
    Wall {
        a: Vec2 { x: a.0, y: a.1 },
        b: Vec2 { x: b.0, y: b.1 },
        z0: 0.0,
        height_m: 0.15,
        edge: Some(WallEdge { tile, side }),
    }
}

/// `w` x `h` maze with only perimeter walls; start at (0,0) facing east.
fn open_maze(w: u32, h: u32) -> MazeGraph {
    let mut tiles = Vec::new();
    for r in 0..h {
        for c in 0..w {
            tiles.push(Tile {
                at: TileXY::new(c, r),
                kind: if c == 0 && r == 0 {
                    TileKind::Start
                } else {
                    TileKind::Normal
                },
                class: TileClass::Linear,
                in_dz: false,
                height_ref: 0.0,
                features: vec![],
            });
        }
    }
    let mut walls = Vec::new();
    for c in 0..w {
        let (x0, x1) = (c as f64 * T, (c + 1) as f64 * T);
        walls.push(wall((x0, 0.0), (x1, 0.0), TileXY::new(c, 0), Dir::South));
        walls.push(wall(
            (x0, h as f64 * T),
            (x1, h as f64 * T),
            TileXY::new(c, h - 1),
            Dir::North,
        ));
    }
    for r in 0..h {
        let (y0, y1) = (r as f64 * T, (r + 1) as f64 * T);
        walls.push(wall((0.0, y0), (0.0, y1), TileXY::new(0, r), Dir::West));
        walls.push(wall(
            (w as f64 * T, y0),
            (w as f64 * T, y1),
            TileXY::new(w - 1, r),
            Dir::East,
        ));
    }
    MazeGraph {
        header: MazeHeader {
            id: None,
            name: None,
            rules_profile: "rcj_maze_2027_draft".into(),
            seed: 1,
            tile_m: T,
            tolerance_frac: 0.0,
            generator_version: "test".into(),
        },
        grid: GridSize { w, h },
        tiles,
        walls,
        victims: vec![],
        obstacles: vec![],
        zones: Zones {
            start: TileXY::new(0, 0),
            start_yaw: 0.0,
            checkpoints: vec![],
            dangerous_zone: None,
        },
    }
}

fn set_tile(m: &mut MazeGraph, c: u32, r: u32, f: impl FnOnce(&mut Tile)) {
    let i = m.idx(TileXY::new(c, r)).unwrap();
    f(&mut m.tiles[i]);
}

fn input(
    maze: MazeGraph,
    robot: RobotConfig,
    errors: ErrorProfile,
    seed: u64,
    dur: f64,
) -> SimInput {
    SimInput {
        maze,
        robot,
        rules: rules_2027_draft(),
        errors,
        settings: RunSettings {
            seed,
            rng_mode: RngMode::Fixed,
            timing: SimTiming::default(),
            max_duration_s: Some(dur),
            lop_enabled: true,
        },
        params: BTreeMap::new(),
        run_id: "test".into(),
        hashes: ConfigHashes::default(),
    }
}

struct Script<F: FnMut(&Observations) -> Commands + Send>(F);

impl<F: FnMut(&Observations) -> Commands + Send> Controller for Script<F> {
    fn init(&mut self, _info: &RobotInfo) -> Result<(), String> {
        Ok(())
    }
    fn step(&mut self, obs: &Observations) -> Result<Commands, String> {
        Ok((self.0)(obs))
    }
}

fn drive(v: [f64; 4]) -> Commands {
    Commands {
        wheel_v: v,
        ..Default::default()
    }
}

fn one_tof(max_range: f64, res: u8) -> RobotConfig {
    let mut r = RobotConfig::reference();
    r.tof = vec![TofMount {
        id: "front".into(),
        model: TofModel::Vl53l8cx,
        pose: MountPose {
            x: 0.05,
            y: 0.0,
            z: 0.06,
            yaw: 0.0,
            pitch: 0.0,
        },
        resolution: res,
        fov_deg: 45.0,
        rate_hz: 15.0,
        max_range_m: max_range,
    }];
    r
}

fn wobble(obs: &Observations) -> Commands {
    let s = (obs.t * 1.3).sin() * 0.05;
    drive([0.15 - s, 0.15 + s, 0.15 - s, 0.15 + s])
}

fn run_fixture(errors: ErrorProfile, seed: u64, dur: f64) -> Replay {
    let sim = Sim::new(input(
        fixture_maze(),
        RobotConfig::reference(),
        errors,
        seed,
        dur,
    ))
    .unwrap();
    sim.run(&mut Script(wobble))
}

#[test]
fn determinism_same_seed_same_bytes() {
    let a = encode_msgpack(&run_fixture(ErrorProfile::realistic(), 1234, 20.0)).unwrap();
    let b = encode_msgpack(&run_fixture(ErrorProfile::realistic(), 1234, 20.0)).unwrap();
    assert_eq!(a, b);
    assert!(a.len() > 1000);
}

#[test]
fn different_seed_differs_under_realistic_and_header_under_clean() {
    let a = run_fixture(ErrorProfile::realistic(), 1, 10.0);
    let b = run_fixture(ErrorProfile::realistic(), 2, 10.0);
    assert_ne!(a.header.seed, b.header.seed);
    assert_ne!(a.frames, b.frames);
    let c = run_fixture(ErrorProfile::clean(), 1, 5.0);
    let d = run_fixture(ErrorProfile::clean(), 2, 5.0);
    assert_ne!(c.header.seed, d.header.seed);
}

#[test]
fn msgpack_and_json_round_trip() {
    let r = run_fixture(ErrorProfile::realistic(), 5, 3.0);
    let bytes = encode_msgpack(&r).unwrap();
    assert_eq!(decode_msgpack(&bytes).unwrap(), r);
    let j = to_json(&r).unwrap();
    assert_eq!(serde_json::from_str::<Replay>(&j).unwrap(), r);
}

#[test]
fn straight_line_clean_covers_v_t() {
    let mut sim = Sim::new(input(
        open_maze(6, 3),
        RobotConfig::reference(),
        ErrorProfile::clean(),
        1,
        10.0,
    ))
    .unwrap();
    let (x0, y0) = (sim.snapshot().pose.x, sim.snapshot().pose.y);
    let mut ctrl = Script(|_: &Observations| drive([0.2; 4]));
    for _ in 0..500 {
        sim.step(&mut ctrl);
    }
    let f = sim.snapshot();
    let d = f.pose.x - x0;
    assert!((d - 1.0).abs() < 0.01, "travelled {d}");
    assert!((f.pose.y - y0).abs() < 1e-9);
    assert!((sim.metrics().distance_m - 1.0).abs() < 0.01);
    // Encoders: ticks = distance / circumference * ticks per rev.
    let obs_ticks = f.wheels[0].actual; // wheels report speed
    assert!((obs_ticks - 0.2).abs() < 1e-9);
}

#[test]
fn encoders_count_wheel_rotation() {
    let mut sim = Sim::new(input(
        open_maze(6, 3),
        RobotConfig::reference(),
        ErrorProfile::clean(),
        1,
        10.0,
    ))
    .unwrap();
    let mut last = [0i64; 4];
    let mut ctrl = Script(|o: &Observations| {
        last = o.encoders;
        drive([0.2; 4])
    });
    for _ in 0..301 {
        sim.step(&mut ctrl);
    }
    let expect = 0.2 * 3.0 / (std::f64::consts::TAU * 0.0325) * 4096.0;
    assert!(
        (last[0] as f64 - expect).abs() / expect < 0.01,
        "{last:?} vs {expect}"
    );
}

#[test]
fn turn_in_place_gets_expected_yaw() {
    let mut sim = Sim::new(input(
        open_maze(3, 3),
        RobotConfig::reference(),
        ErrorProfile::clean(),
        1,
        10.0,
    ))
    .unwrap();
    let mut ctrl = Script(|_: &Observations| drive([-0.1, 0.1, -0.1, 0.1]));
    for _ in 0..200 {
        sim.step(&mut ctrl);
    }
    // omega = (0.1 - -0.1) / 0.16 = 1.25 rad/s for 2 s.
    let yaw = sim.snapshot().pose.yaw;
    assert!((yaw - wrap_pi(2.5)).abs() < 0.01, "yaw {yaw}");
}

#[test]
fn wall_stops_robot_and_emits_collision() {
    let sim = Sim::new(input(
        open_maze(3, 1),
        RobotConfig::reference(),
        ErrorProfile::clean(),
        1,
        6.0,
    ))
    .unwrap();
    let r = sim.run(&mut Script(|_: &Observations| drive([0.3; 4])));
    let limit = 3.0 * T - 0.095;
    let max_x = r.frames.iter().map(|f| f.pose.x).fold(f64::MIN, f64::max);
    assert!(max_x <= limit + 0.001, "max x {max_x} limit {limit}");
    assert!(max_x > limit - 0.005, "should reach the wall: {max_x}");
    assert!(r
        .events
        .iter()
        .any(|e| matches!(e.kind, EventKind::Collision { .. })));
    assert!(matches!(
        r.events.last().unwrap().kind,
        EventKind::RunEnd {
            reason: EndReason::TimeUp,
            ..
        }
    ));
}

fn first_tof_obs(robot: RobotConfig) -> TofReading {
    let sim = Sim::new(input(
        open_maze(3, 1),
        robot,
        ErrorProfile::clean(),
        1,
        0.05,
    ))
    .unwrap();
    let mut got = None;
    let mut ctrl = Script(|o: &Observations| {
        if got.is_none() {
            got = Some(o.tof[0].clone());
        }
        Commands::default()
    });
    sim.run(&mut ctrl);
    got.unwrap()
}

#[test]
fn tof_reads_wall_distance_in_centre_zones() {
    let r = first_tof_obs(one_tof(4.0, 8));
    assert!(r.fresh);
    assert_eq!(r.zones.len(), 64);
    assert_eq!(r.status.len(), 64);
    let dist = 3.0 * T - (T / 2.0 + 0.05);
    let a = (0.5 / 8.0 * 45.0f64).to_radians();
    let expect = dist / (a.cos() * a.cos()) * 1000.0;
    for row in [3usize, 4] {
        for col in [3usize, 4] {
            let z = r.zones[row * 8 + col] as f64;
            assert!(
                (z - expect).abs() <= 1.0,
                "zone {row},{col} = {z}, want {expect}"
            );
            assert_eq!(r.status[row * 8 + col], tof_status::OK);
        }
    }
}

#[test]
fn tof_out_of_range_gives_sentinel_and_4x4_has_16_zones() {
    let r = first_tof_obs(one_tof(0.4, 8));
    for row in [3usize, 4] {
        for col in [3usize, 4] {
            assert_eq!(r.zones[row * 8 + col], TOF_OUT_OF_RANGE_MM);
            assert_eq!(r.status[row * 8 + col], tof_status::NO_TARGET);
        }
    }
    assert_eq!(first_tof_obs(one_tof(4.0, 4)).zones.len(), 16);
}

#[test]
fn tof_fresh_follows_sensor_rate() {
    let sim = Sim::new(input(
        open_maze(3, 1),
        one_tof(4.0, 4),
        ErrorProfile::clean(),
        1,
        2.0,
    ))
    .unwrap();
    let mut fresh = 0;
    let mut steps = 0;
    let mut ctrl = Script(|o: &Observations| {
        steps += 1;
        fresh += o.tof[0].fresh as u32;
        Commands::default()
    });
    sim.run(&mut ctrl);
    assert_eq!(steps, 200);
    assert!(
        (29..=31).contains(&fresh),
        "fresh samples in 2 s at 15 Hz: {fresh}"
    );
}

#[test]
fn imu_clean_yaw_equals_true_yaw() {
    let mut sim = Sim::new(input(
        open_maze(3, 3),
        RobotConfig::reference(),
        ErrorProfile::clean(),
        1,
        10.0,
    ))
    .unwrap();
    let mut ctrl = Script(|_: &Observations| drive([-0.1, 0.1, -0.1, 0.1]));
    for i in 0..500 {
        sim.step(&mut ctrl);
        if i % 50 == 0 {
            let f = sim.snapshot();
            assert!(
                (f.imu.yaw - f.pose.yaw).abs() < 1e-9,
                "{} vs {}",
                f.imu.yaw,
                f.pose.yaw
            );
        }
    }
}

#[test]
fn lop_fires_after_30s_stationary_and_teleports_to_start() {
    let sim = Sim::new(input(
        open_maze(4, 1),
        RobotConfig::reference(),
        ErrorProfile::clean(),
        1,
        40.0,
    ))
    .unwrap();
    let mut flags = 0;
    let mut ctrl = Script(|o: &Observations| {
        flags += o.lop_restarted as u32;
        if o.t < 1.0 {
            drive([0.2; 4])
        } else {
            drive([0.0; 4])
        }
    });
    let r = sim.run(&mut ctrl);
    let lop: Vec<_> = r
        .events
        .iter()
        .filter_map(|e| match e.kind {
            EventKind::LopRestart { to, restarts } => Some((e.t, to, restarts)),
            _ => None,
        })
        .collect();
    assert_eq!(lop.len(), 1, "{lop:?}");
    assert!((30.0..32.0).contains(&lop[0].0), "t = {}", lop[0].0);
    assert_eq!(lop[0].1, TileXY::new(0, 0));
    assert_eq!(lop[0].2, 1);
    assert_eq!(flags, 1);
    let last = r.frames.last().unwrap();
    assert!((last.pose.x - T / 2.0).abs() < 1e-6 && (last.pose.y - T / 2.0).abs() < 1e-6);
}

#[test]
fn entering_black_triggers_restart_and_silver_scores_once() {
    let mut m = open_maze(4, 1);
    set_tile(&mut m, 1, 0, |t| t.kind = TileKind::Silver);
    set_tile(&mut m, 3, 0, |t| t.kind = TileKind::Black);
    let sim = Sim::new(input(
        m,
        RobotConfig::reference(),
        ErrorProfile::clean(),
        1,
        8.0,
    ))
    .unwrap();
    let r = sim.run(&mut Script(|_: &Observations| drive([0.3; 4])));
    let kinds: Vec<&EventKind> = r.events.iter().map(|e| &e.kind).collect();
    let black = kinds
        .iter()
        .position(|k| matches!(k, EventKind::EnteredBlack { .. }))
        .expect("entered black");
    assert!(
        matches!(kinds[black + 1], EventKind::TileExit { .. })
            || matches!(kinds[black + 1], EventKind::LopRestart { .. })
    );
    let to = kinds[black..]
        .iter()
        .find_map(|k| match k {
            EventKind::LopRestart { to, .. } => Some(*to),
            _ => None,
        })
        .expect("restart after black");
    // The silver tile was visited first, so the restart goes there.
    assert_eq!(to, TileXY::new(1, 0));
    let cps = kinds
        .iter()
        .filter(|k| matches!(k, EventKind::Checkpoint { .. }))
        .count();
    assert_eq!(cps, 1);
    match &r.events.last().unwrap().kind {
        EventKind::RunEnd { score, .. } => assert_eq!(score.checkpoints, 10),
        k => panic!("{k:?}"),
    }
}

#[test]
fn ramp_raises_z_and_pitch() {
    let mut m = open_maze(3, 1);
    set_tile(&mut m, 1, 0, |t| {
        t.features = vec![TileFeature::Ramp {
            up_dir: Dir::East,
            angle_deg: 10.0,
            rise_m: 0.05,
        }]
    });
    set_tile(&mut m, 2, 0, |t| t.height_ref = 0.05);
    let sim = Sim::new(input(
        m,
        RobotConfig::reference(),
        ErrorProfile::clean(),
        1,
        6.0,
    ))
    .unwrap();
    let r = sim.run(&mut Script(|_: &Observations| drive([0.2; 4])));
    assert!(r.frames.first().unwrap().pose.z.abs() < 1e-9);
    let last = r.frames.last().unwrap();
    assert!(
        (last.pose.z - 0.05).abs() < 0.004,
        "final z {}",
        last.pose.z
    );
    let max_pitch = r.frames.iter().map(|f| f.pose.pitch).fold(0.0, f64::max);
    assert!(max_pitch > 0.1, "pitch {max_pitch}");
    let zs: Vec<f64> = r.frames.iter().map(|f| f.pose.z).collect();
    assert!(zs.windows(2).all(|w| w[1] >= w[0] - 1e-9), "z must not dip");
}

#[test]
fn world_heightfield_and_raycast() {
    let mut m = open_maze(3, 1);
    set_tile(&mut m, 0, 0, |t| {
        t.features = vec![TileFeature::Stairs {
            up_dir: Dir::East,
            step_m: 0.01,
            tread_m: 0.07,
            steps: 4,
        }]
    });
    set_tile(&mut m, 1, 0, |t| {
        t.features = vec![TileFeature::SpeedBump {
            across: Dir::East,
            height_m: 0.01,
            width_m: 0.04,
        }]
    });
    m.obstacles.push(Obstacle {
        id: 1,
        center: Vec2 { x: 0.7, y: 0.14 },
        radius_m: 0.03,
        height_m: 0.1,
        tile: TileXY::new(2, 0),
    });
    let w = World::new(
        &m,
        &rules_2027_draft(),
        &ErrorProfile::clean(),
        &SimRng::new(1),
    );
    assert!((w.floor_z(0.01, 0.1).0 - 0.01).abs() < 1e-12);
    assert!((w.floor_z(0.27, 0.1).0 - 0.04).abs() < 1e-12);
    assert!((w.floor_z(T + 0.14, 0.1).0 - 0.01).abs() < 1e-12);
    assert!(w.floor_z(T + 0.05, 0.1).0 < 1e-12);
    let o = Vec3 {
        x: 0.4,
        y: 0.14,
        z: 0.05,
    };
    let east = Vec3 {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    };
    let h = w.raycast(o, east, 4.0).unwrap();
    assert_eq!(h.kind, HitKind::Obstacle);
    assert!((h.dist - 0.27).abs() < 1e-9);
    // Above the 0.15 m walls nothing is hit.
    assert!(w
        .raycast(
            Vec3 {
                x: 0.4,
                y: 0.14,
                z: 0.2
            },
            east,
            4.0
        )
        .is_none());
    let wall = w
        .raycast(
            Vec3 {
                x: 0.1,
                y: 0.05,
                z: 0.1,
            },
            east,
            4.0,
        )
        .unwrap();
    assert_eq!(wall.kind, HitKind::Wall);
    assert!((wall.dist - 0.74).abs() < 1e-9, "{}", wall.dist);
    let down = w
        .raycast(
            o,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: -1.0,
            },
            1.0,
        )
        .unwrap();
    assert_eq!(down.kind, HitKind::Floor);
    assert!(w.raycast(o, east, 0.1).is_none());
}

#[test]
fn walls_jitter_with_tolerance_and_stay_connected() {
    let m = open_maze(3, 3);
    let rules = rules_2027_draft();
    let w = World::new(&m, &rules, &ErrorProfile::realistic(), &SimRng::new(9));
    assert!((w.applied_tolerance - 0.1).abs() < 1e-12);
    assert_ne!(w.maze.walls, m.walls);
    // Wall chains meet: the north end of the west wall in row 0 is the west end of the
    // south wall of... check shared vertex (0, T) between consecutive west walls.
    let west: Vec<&Wall> = w
        .maze
        .walls
        .iter()
        .filter(|x| x.a.x < 0.2 && x.b.x < 0.2)
        .collect();
    assert!(west
        .windows(2)
        .any(|p| p[0].b == p[1].a || p[0].a == p[1].b || p[0].b == p[1].b));
    let clean = World::new(&m, &rules, &ErrorProfile::clean(), &SimRng::new(9));
    assert_eq!(clean.maze.walls, m.walls);
    assert_eq!(clean.applied_tolerance, 0.0);
}

#[test]
fn controller_fault_and_stop_end_the_run() {
    struct Bad;
    impl Controller for Bad {
        fn init(&mut self, _: &RobotInfo) -> Result<(), String> {
            Ok(())
        }
        fn step(&mut self, o: &Observations) -> Result<Commands, String> {
            if o.t > 0.1 {
                Err("boom".into())
            } else {
                Ok(Commands::default())
            }
        }
    }
    let sim = Sim::new(input(
        open_maze(2, 2),
        RobotConfig::reference(),
        ErrorProfile::clean(),
        1,
        5.0,
    ))
    .unwrap();
    let r = sim.run(&mut Bad);
    assert!(r
        .events
        .iter()
        .any(|e| matches!(&e.kind, EventKind::ControllerError { message } if message == "boom")));
    assert!(matches!(
        r.events.last().unwrap().kind,
        EventKind::RunEnd {
            reason: EndReason::ControllerFault,
            ..
        }
    ));

    let mut sim = Sim::new(input(
        open_maze(2, 2),
        RobotConfig::reference(),
        ErrorProfile::clean(),
        1,
        5.0,
    ))
    .unwrap();
    let mut c = Script(|_: &Observations| Commands {
        note: Some("hi".into()),
        ..Default::default()
    });
    sim.step(&mut c);
    let ev = sim.events_since();
    assert!(ev.iter().any(|e| matches!(e.kind, EventKind::RunStart)));
    assert!(ev
        .iter()
        .any(|e| matches!(&e.kind, EventKind::Note { text } if text == "hi")));
    sim.stop();
    assert_eq!(sim.is_ended(), Some(EndReason::Stopped));
    assert!(sim
        .events_since()
        .iter()
        .any(|e| matches!(e.kind, EventKind::RunEnd { .. })));
}

#[test]
fn metrics_count_reachable_tiles() {
    let mut m = open_maze(3, 1);
    set_tile(&mut m, 2, 0, |t| t.kind = TileKind::Black);
    let sim = Sim::new(input(
        m,
        RobotConfig::reference(),
        ErrorProfile::clean(),
        1,
        1.0,
    ))
    .unwrap();
    let mx = sim.metrics();
    assert_eq!(mx.tiles_reachable, 2);
    assert_eq!(mx.tiles_visited, 1);
    assert!((mx.coverage - 0.5).abs() < 1e-12);
}

// ---- M4: scoring, colour and camera ----

fn final_score(r: &Replay) -> ScoreBreakdown {
    match &r.events.last().unwrap().kind {
        EventKind::RunEnd { score, .. } => score.clone(),
        k => panic!("{k:?}"),
    }
}

fn count(r: &Replay, f: impl Fn(&EventKind) -> bool) -> usize {
    r.events.iter().filter(|e| f(&e.kind)).count()
}

fn end_reason(r: &Replay) -> EndReason {
    match &r.events.last().unwrap().kind {
        EventKind::RunEnd { reason, .. } => *reason,
        k => panic!("{k:?}"),
    }
}

/// Victim on the north wall of tile (0, 0), facing south into the start tile.
fn north_victim(
    id: u32,
    kind: VictimKind,
    status: Option<VictimStatus>,
    tier: VictimTier,
) -> Victim {
    Victim {
        id,
        kind,
        status,
        pos: Vec3 {
            x: T / 2.0,
            y: T,
            z: 0.07,
        },
        facing_yaw: -std::f64::consts::FRAC_PI_2,
        rotation: 0.0,
        tile: TileXY::new(0, 0),
        wall_side: Dir::North,
        tier: Some(tier),
    }
}

fn smiley(id: u32, s: VictimStatus, tier: VictimTier) -> Victim {
    north_victim(id, VictimKind::Smiley { status: s }, Some(s), tier)
}

/// Blink for 5 s (0.5 s on/off), report at 4.99 s, deploy kits at the given times.
fn blinker(
    status: Option<VictimStatus>,
    speed: f64,
    deploys: Vec<(f64, u32)>,
) -> impl FnMut(&Observations) -> Commands + Send {
    let mut reported = false;
    let mut next = 0usize;
    move |o: &Observations| {
        let mut c = drive([speed; 4]);
        if let Some(s) = status {
            if o.t < 5.0 {
                c.id_light = ((o.t / 0.5) as u64).is_multiple_of(2);
            }
            if o.t >= 4.99 && !reported {
                reported = true;
                c.report = Some(VictimReport { status: s });
            }
        }
        if let Some(&(t, n)) = deploys.get(next) {
            if o.t >= t {
                next += 1;
                c.deploy_kits = n;
            }
        }
        c
    }
}

fn run_ident(
    victims: Vec<Victim>,
    status: Option<VictimStatus>,
    speed: f64,
    deploys: Vec<(f64, u32)>,
    dur: f64,
) -> (Replay, Metrics) {
    let mut m = open_maze(3, 1);
    m.victims = victims;
    let mut sim = Sim::new(input(
        m,
        RobotConfig::reference(),
        ErrorProfile::clean(),
        1,
        dur,
    ))
    .unwrap();
    let mut ctrl = Script(blinker(status, speed, deploys));
    while let StepOutcome::Running = sim.step(&mut ctrl) {}
    let metrics = sim.metrics();
    (sim.into_replay(), metrics)
}

fn identified(r: &Replay) -> Vec<(u32, bool, i32)> {
    r.events
        .iter()
        .filter_map(|e| match e.kind {
            EventKind::VictimIdentified {
                victim_id,
                correct,
                points,
                ..
            } => Some((victim_id, correct, points)),
            _ => None,
        })
        .collect()
}

#[test]
fn correct_identification_scores_by_tier_and_kind() {
    use VictimStatus::*;
    let rings = |a: RingColor| {
        [
            a,
            RingColor::Yellow,
            RingColor::Yellow,
            RingColor::Yellow,
            RingColor::Yellow,
        ]
    };
    let cases: Vec<(Victim, VictimStatus, i32)> = vec![
        (smiley(1, Harmed, VictimTier::Linear), Harmed, 5),
        (smiley(1, Stable, VictimTier::Floating), Stable, 15),
        (
            north_victim(
                1,
                VictimKind::Cognitive {
                    rings: rings(RingColor::Green),
                },
                Some(Stable),
                VictimTier::Linear,
            ),
            Stable,
            10,
        ),
        (
            north_victim(
                1,
                VictimKind::Cognitive {
                    rings: rings(RingColor::Blue),
                },
                Some(Harmed),
                VictimTier::Floating,
            ),
            Harmed,
            30,
        ),
    ];
    let rules = rules_2027_draft();
    let want = [
        rules.scoring.victim_points.linear_smiley,
        rules.scoring.victim_points.floating_smiley,
        rules.scoring.victim_points.linear_cognitive,
        rules.scoring.victim_points.floating_cognitive,
    ];
    for ((v, status, pts), w) in cases.into_iter().zip(want) {
        assert_eq!(pts, w);
        let (r, m) = run_ident(vec![v], Some(status), 0.0, vec![], 7.0);
        assert_eq!(identified(&r), vec![(1, true, pts)]);
        let s = final_score(&r);
        assert_eq!(s.victims, pts);
        // Reliability: SVI * 10, nothing else.
        assert_eq!(s.reliability, rules.scoring.reliability_per_svi);
        assert_eq!(s.total, pts + s.reliability);
        assert_eq!(m.victims_identified, 1);
        assert_eq!(
            count(&r, |k| matches!(k, EventKind::VictimMissed { .. })),
            0
        );
    }
}

#[test]
fn wrong_status_is_a_misidentification_and_zero_points() {
    let v = smiley(1, VictimStatus::Harmed, VictimTier::Linear);
    let (r, m) = run_ident(vec![v], Some(VictimStatus::Unharmed), 0.0, vec![], 7.0);
    assert_eq!(identified(&r), vec![(1, false, 0)]);
    let s = final_score(&r);
    assert_eq!(s.victims, 0);
    assert_eq!(s.penalties, -5);
    assert_eq!(s.total, 0);
    assert_eq!(m.misidentifications, 1);
    assert_eq!(m.victims_identified, 0);
    assert_eq!(
        count(&r, |k| matches!(
            k,
            EventKind::VictimMissed { victim_id: 1 }
        )),
        1
    );
}

#[test]
fn blink_while_moving_is_rejected_with_a_note() {
    let v = smiley(1, VictimStatus::Harmed, VictimTier::Linear);
    let (r, m) = run_ident(vec![v], Some(VictimStatus::Harmed), 0.1, vec![], 6.0);
    assert!(identified(&r).is_empty());
    assert_eq!(
        count(&r, |k| matches!(k, EventKind::Misidentification { .. })),
        0
    );
    assert!(r
        .events
        .iter()
        .any(|e| matches!(&e.kind, EventKind::Note { text } if text.contains("rejected"))));
    assert_eq!(final_score(&r).penalties, 0);
    assert_eq!(m.misidentifications, 0);
}

#[test]
fn constant_light_is_not_a_blink() {
    let v = smiley(1, VictimStatus::Harmed, VictimTier::Linear);
    let mut m = open_maze(3, 1);
    m.victims = vec![v];
    let sim = Sim::new(input(
        m,
        RobotConfig::reference(),
        ErrorProfile::clean(),
        1,
        6.0,
    ))
    .unwrap();
    let r = sim.run(&mut Script(|o: &Observations| Commands {
        id_light: true,
        report: (o.t >= 4.99 && o.t < 5.0).then_some(VictimReport {
            status: VictimStatus::Harmed,
        }),
        ..Default::default()
    }));
    assert!(identified(&r).is_empty());
    assert_eq!(final_score(&r).penalties, 0);
}

#[test]
fn no_victim_in_range_is_a_misidentification() {
    let (r, m) = run_ident(vec![], Some(VictimStatus::Stable), 0.0, vec![], 6.0);
    assert_eq!(
        count(&r, |k| matches!(
            k,
            EventKind::Misidentification {
                reported: VictimStatus::Stable
            }
        )),
        1
    );
    let s = final_score(&r);
    assert_eq!(s.penalties, -5);
    assert_eq!(s.total, 0, "total is clamped at 0");
    assert_eq!(m.misidentifications, 1);
}

#[test]
fn decoy_in_range_is_a_false_positive() {
    let d = north_victim(7, VictimKind::Decoy, None, VictimTier::Linear);
    let (r, m) = run_ident(vec![d], Some(VictimStatus::Harmed), 0.0, vec![], 6.0);
    assert_eq!(
        count(&r, |k| matches!(
            k,
            EventKind::DecoyFalsePositive { victim_id: 7 }
        )),
        1
    );
    assert!(identified(&r).is_empty());
    assert_eq!(final_score(&r).penalties, -5);
    assert_eq!(m.decoy_false_positives, 1);
    // Decoys are never "missed".
    assert_eq!(
        count(&r, |k| matches!(k, EventKind::VictimMissed { .. })),
        0
    );
}

fn kit_events(r: &Replay) -> Vec<(Option<u32>, u32, i32)> {
    r.events
        .iter()
        .filter_map(|e| match e.kind {
            EventKind::KitDeployed {
                victim_id,
                count,
                points,
            } => Some((victim_id, count, points)),
            _ => None,
        })
        .collect()
}

#[test]
fn kits_are_capped_by_status() {
    // Harmed: cap 2 -> 1 kit = 10, second kit brings the total to 30, third adds nothing.
    let v = smiley(1, VictimStatus::Harmed, VictimTier::Linear);
    let (r, m) = run_ident(
        vec![v],
        Some(VictimStatus::Harmed),
        0.0,
        vec![(5.5, 1), (6.0, 2)],
        7.0,
    );
    assert_eq!(kit_events(&r), vec![(Some(1), 1, 10), (Some(1), 2, 20)]);
    assert_eq!(final_score(&r).kits, 30);
    assert_eq!(m.kits_used, 3);
    // Reliability counts only the 2 kits that counted: SVI 1 + SRD 2.
    assert_eq!(final_score(&r).reliability, 10 + 20);

    // Stable: cap 1.
    let v = smiley(1, VictimStatus::Stable, VictimTier::Linear);
    let (r, _) = run_ident(
        vec![v],
        Some(VictimStatus::Stable),
        0.0,
        vec![(5.5, 2)],
        7.0,
    );
    assert_eq!(kit_events(&r), vec![(Some(1), 2, 10)]);

    // Unharmed: cap 0.
    let v = smiley(1, VictimStatus::Unharmed, VictimTier::Linear);
    let (r, _) = run_ident(
        vec![v],
        Some(VictimStatus::Unharmed),
        0.0,
        vec![(5.5, 1)],
        7.0,
    );
    assert_eq!(kit_events(&r), vec![(Some(1), 1, 0)]);

    // No identified victim: deploys with no target and no points.
    let (r, m) = run_ident(vec![], None, 0.0, vec![(1.0, 1)], 2.0);
    assert_eq!(kit_events(&r), vec![(None, 1, 0)]);
    assert_eq!(m.kits_used, 1);
}

/// Speed by time: the first phase whose end time is after `t` wins.
fn phases(p: Vec<(f64, f64)>) -> impl FnMut(&Observations) -> Commands + Send {
    move |o: &Observations| {
        let v = p
            .iter()
            .find(|(end, _)| o.t < *end)
            .map_or(0.0, |(_, v)| *v);
        drive([v; 4])
    }
}

fn blue_maze() -> MazeGraph {
    let mut m = open_maze(3, 1);
    set_tile(&mut m, 1, 0, |t| t.kind = TileKind::Blue);
    m
}

fn run_phases(m: MazeGraph, p: Vec<(f64, f64)>, dur: f64) -> Replay {
    Sim::new(input(
        m,
        RobotConfig::reference(),
        ErrorProfile::clean(),
        1,
        dur,
    ))
    .unwrap()
    .run(&mut Script(phases(p)))
}

fn blue_stops(r: &Replay) -> Vec<bool> {
    r.events
        .iter()
        .filter_map(|e| match e.kind {
            EventKind::BlueStop { ok, .. } => Some(ok),
            _ => None,
        })
        .collect()
}

#[test]
fn blue_stop_ok_scores_once() {
    let r = run_phases(blue_maze(), vec![(1.0, 0.2), (6.5, 0.0), (9.0, 0.2)], 9.0);
    assert_eq!(blue_stops(&r), vec![true]);
    let s = final_score(&r);
    assert_eq!(s.blue_tiles, 30);
    assert_eq!(count(&r, |k| matches!(k, EventKind::LopRestart { .. })), 0);
    assert_eq!(s.reliability, 10, "SBV * 10");
}

#[test]
fn blue_early_exit_fails_and_restarts_with_lop() {
    let r = run_phases(blue_maze(), vec![(3.0, 0.3)], 3.0);
    // The robot keeps driving after the teleport, so it may fail the tile again.
    let stops = blue_stops(&r);
    assert!(!stops.is_empty() && stops.iter().all(|ok| !ok), "{stops:?}");
    let stop = r
        .events
        .iter()
        .position(|e| matches!(e.kind, EventKind::BlueStop { .. }))
        .unwrap();
    let lop = r.events[stop..]
        .iter()
        .find_map(|e| match e.kind {
            EventKind::LopRestart { to, .. } => Some(to),
            _ => None,
        })
        .expect("LoP restart after the early exit");
    assert_eq!(lop, TileXY::new(0, 0));
    let s = final_score(&r);
    assert_eq!(s.blue_tiles, 0);
    assert_eq!(s.reliability, 0, "-15 floored at 0");
}

#[test]
fn blue_revisit_subtracts_penalty() {
    let r = run_phases(
        blue_maze(),
        vec![
            (1.0, 0.2),
            (6.5, 0.0),
            (8.0, 0.2),
            (9.0, -0.2),
            (14.5, 0.0),
            (16.0, -0.2),
        ],
        16.0,
    );
    assert_eq!(blue_stops(&r), vec![true, true]);
    assert_eq!(final_score(&r).blue_tiles, 30 - 10);
}

fn ramp_maze() -> MazeGraph {
    let mut m = open_maze(3, 1);
    set_tile(&mut m, 1, 0, |t| {
        t.features = vec![TileFeature::Ramp {
            up_dir: Dir::East,
            angle_deg: 10.0,
            rise_m: 0.05,
        }]
    });
    set_tile(&mut m, 2, 0, |t| t.height_ref = 0.05);
    m
}

#[test]
fn ramp_points_awarded_once_for_a_full_upward_traversal() {
    // Up, back down, up again.
    let r = run_phases(
        ramp_maze(),
        vec![(4.0, 0.2), (8.0, -0.2), (13.0, 0.2)],
        13.0,
    );
    let ramps: Vec<_> = r
        .events
        .iter()
        .filter_map(|e| match e.kind {
            EventKind::RampTraversed { up, points, .. } => Some((up, points)),
            _ => None,
        })
        .collect();
    assert_eq!(ramps, vec![(true, 10)]);
    assert_eq!(final_score(&r).ramps, 10);
}

#[test]
fn stairs_and_speed_bump_score_once_each() {
    let mut m = open_maze(4, 1);
    set_tile(&mut m, 1, 0, |t| {
        t.features = vec![TileFeature::SpeedBump {
            across: Dir::East,
            height_m: 0.01,
            width_m: 0.04,
        }]
    });
    set_tile(&mut m, 2, 0, |t| {
        t.features = vec![TileFeature::Stairs {
            up_dir: Dir::East,
            step_m: 0.01,
            tread_m: 0.07,
            steps: 3,
        }]
    });
    set_tile(&mut m, 3, 0, |t| t.height_ref = 0.03);
    let r = run_phases(m, vec![(6.0, 0.2)], 6.0);
    let s = final_score(&r);
    assert_eq!(s.speed_bumps, 5);
    assert_eq!(s.stairs, 10);
    assert_eq!(
        count(&r, |k| matches!(k, EventKind::StairsTraversed { .. })),
        1
    );
}

#[test]
fn exit_bonus_matches_the_formula() {
    // Identify the victim at the start, climb the ramp, come back and hold the light.
    let mut m = ramp_maze();
    m.victims = vec![smiley(1, VictimStatus::Harmed, VictimTier::Linear)];
    let sim = Sim::new(input(
        m,
        RobotConfig::reference(),
        ErrorProfile::clean(),
        1,
        30.0,
    ))
    .unwrap();
    let mut blink = blinker(Some(VictimStatus::Harmed), 0.0, vec![]);
    let mut path = phases(vec![(5.0, 0.0), (8.2, 0.2), (13.0, -0.2)]);
    let mut ctrl = Script(|o: &Observations| {
        let mut c = blink(o);
        c.wheel_v = path(o).wheel_v;
        if o.t >= 13.0 {
            c.id_light = true;
        }
        c
    });
    let r = sim.run(&mut ctrl);
    assert_eq!(end_reason(&r), EndReason::Exited);
    let end = r.events.last().unwrap().t;
    assert!((18.0..18.1).contains(&end), "exit at {end}");
    let rules = rules_2027_draft();
    let sc = &rules.scoring;
    let (svi, sbv, ssn, srn) = (1, 0, 0, 1);
    let want = sc.exit_per_svi * svi
        + sc.exit_per_sbv * sbv
        + sc.exit_per_ssn * ssn
        + sc.exit_per_srn * srn;
    let s = final_score(&r);
    assert_eq!(want, 15);
    assert_eq!(s.exit_bonus, want);
    assert_eq!(s.victims, 5);
    assert_eq!(s.ramps, 10);
    assert_eq!(s.reliability, 10);
    assert_eq!(s.total, 5 + 10 + 10 + 15);
}

#[test]
fn end_run_off_the_start_tile_is_ignored() {
    let sim = Sim::new(input(
        open_maze(3, 1),
        RobotConfig::reference(),
        ErrorProfile::clean(),
        1,
        5.0,
    ))
    .unwrap();
    let r = sim.run(&mut Script(|o: &Observations| Commands {
        wheel_v: [0.2; 4],
        end_run: o.t >= 3.0,
        id_light: true,
        ..Default::default()
    }));
    assert_eq!(end_reason(&r), EndReason::TimeUp);
    assert!(r
        .events
        .iter()
        .any(|e| matches!(&e.kind, EventKind::Note { text } if text.contains("end_run ignored"))));
}

#[test]
fn end_run_on_start_without_the_light_is_ignored() {
    let sim = Sim::new(input(
        open_maze(2, 1),
        RobotConfig::reference(),
        ErrorProfile::clean(),
        1,
        8.0,
    ))
    .unwrap();
    let r = sim.run(&mut Script(|o: &Observations| Commands {
        end_run: o.t >= 1.0,
        ..Default::default()
    }));
    assert_eq!(end_reason(&r), EndReason::TimeUp);
}

#[test]
fn reliability_is_floored_at_zero() {
    // Idle for 65 s: two LoP restarts, -30 raw, floored.
    let r = run_phases(open_maze(3, 1), vec![], 65.0);
    assert!(count(&r, |k| matches!(k, EventKind::LopRestart { .. })) >= 2);
    let s = final_score(&r);
    assert_eq!(s.reliability, 0);
    assert_eq!(s.total, 0);
}

// ---- camera ----

fn cam_robot(yaw: f64, range: f64, vfov: f64) -> RobotConfig {
    let mut r = RobotConfig::reference();
    r.cameras = vec![CameraMount {
        id: "cam".into(),
        model: CameraModel::OpenmvN6,
        pose: MountPose {
            x: 0.05,
            y: 0.0,
            z: 0.07,
            yaw,
            pitch: 0.0,
        },
        hfov_deg: 70.0,
        vfov_deg: vfov,
        max_range_m: range,
        rate_hz: 20.0,
    }];
    r
}

/// Victim on the east wall of a 3x1 maze, 0.65 m ahead of the camera.
fn east_victim(kind: VictimKind, facing: f64) -> Victim {
    let status = match &kind {
        VictimKind::Smiley { status } => Some(*status),
        _ => None,
    };
    Victim {
        id: 3,
        kind,
        status,
        pos: Vec3 {
            x: 3.0 * T,
            y: T / 2.0,
            z: 0.07,
        },
        facing_yaw: facing,
        rotation: 0.0,
        tile: TileXY::new(2, 0),
        wall_side: Dir::East,
        tier: Some(VictimTier::Linear),
    }
}

fn cam_dets(maze: MazeGraph, robot: RobotConfig, errors: ErrorProfile) -> Vec<(f64, CamDetection)> {
    let sim = Sim::new(input(maze, robot, errors, 5, 1.0)).unwrap();
    let mut got = Vec::new();
    let mut ctrl = Script(|o: &Observations| {
        for d in &o.cams {
            got.push((o.t, d.clone()));
        }
        Commands::default()
    });
    sim.run(&mut ctrl);
    got
}

fn happy() -> VictimKind {
    VictimKind::Smiley {
        status: VictimStatus::Unharmed,
    }
}

fn maze_with(v: Victim) -> MazeGraph {
    let mut m = open_maze(3, 1);
    m.victims = vec![v];
    m
}

const WEST: f64 = std::f64::consts::PI;

#[test]
fn camera_detects_in_range_in_fov_facing_victim() {
    let d = cam_dets(
        maze_with(east_victim(happy(), WEST)),
        cam_robot(0.0, 1.0, 55.0),
        ErrorProfile::clean(),
    );
    assert!(d.len() >= 15, "~20 frames in 1 s, got {}", d.len());
    let (_, det) = &d[0];
    assert_eq!(det.kind, DetectionKind::Smiley);
    assert_eq!(det.status, Some(VictimStatus::Unharmed));
    assert!((det.distance_m - (3.0 * T - (T / 2.0 + 0.05))).abs() < 1e-6);
    assert!(det.bearing.abs() < 1e-6);
    assert!(det.confidence > 0.3 && det.confidence <= 1.0);
}

#[test]
fn camera_respects_range_fov_and_facing() {
    let m = || maze_with(east_victim(happy(), WEST));
    // Out of range.
    assert!(cam_dets(m(), cam_robot(0.0, 0.6, 55.0), ErrorProfile::clean()).is_empty());
    // Looking north: the victim is 90 degrees off-axis, outside the 70 degree hfov.
    assert!(cam_dets(
        m(),
        cam_robot(std::f64::consts::FRAC_PI_2, 1.0, 55.0),
        ErrorProfile::clean()
    )
    .is_empty());
    // Vertical FOV too narrow for the 0 m height offset? A target at camera height is
    // always inside; tilt the target up out of a 10 degree vfov instead.
    let mut high = east_victim(happy(), WEST);
    high.pos.z = 0.07 + 0.2;
    assert!(cam_dets(
        maze_with(high),
        cam_robot(0.0, 1.0, 10.0),
        ErrorProfile::clean()
    )
    .is_empty());
    // Facing away from the camera.
    assert!(cam_dets(
        maze_with(east_victim(happy(), 0.0)),
        cam_robot(0.0, 1.0, 55.0),
        ErrorProfile::clean()
    )
    .is_empty());
}

#[test]
fn camera_is_occluded_by_a_wall() {
    let mut m = maze_with(east_victim(happy(), WEST));
    m.walls.push(Wall {
        a: Vec2 { x: 0.5, y: 0.0 },
        b: Vec2 { x: 0.5, y: T },
        z0: 0.0,
        height_m: 0.15,
        edge: None,
    });
    assert!(cam_dets(m, cam_robot(0.0, 1.0, 55.0), ErrorProfile::clean()).is_empty());
}

#[test]
fn camera_reports_rings_and_handles_decoys() {
    let rings = [
        RingColor::Blue,
        RingColor::Yellow,
        RingColor::Yellow,
        RingColor::Yellow,
        RingColor::Yellow,
    ];
    let d = cam_dets(
        maze_with(east_victim(VictimKind::Cognitive { rings }, WEST)),
        cam_robot(0.0, 1.0, 55.0),
        ErrorProfile::clean(),
    );
    assert_eq!(d[0].1.kind, DetectionKind::Cognitive);
    assert_eq!(d[0].1.rings, Some(rings));
    assert_eq!(d[0].1.status, None);
    let decoy = || maze_with(east_victim(VictimKind::Decoy, WEST));
    assert!(cam_dets(decoy(), cam_robot(0.0, 1.0, 55.0), ErrorProfile::clean()).is_empty());
    let mut e = ErrorProfile::clean();
    e.camera.decoy_confusion = 1.0;
    let d = cam_dets(decoy(), cam_robot(0.0, 1.0, 55.0), e);
    assert!(!d.is_empty());
    assert_eq!(d[0].1.kind, DetectionKind::Smiley);
}

#[test]
fn camera_latency_delays_delivery() {
    let mut e = ErrorProfile::clean();
    e.camera.latency_ms = 100.0;
    let d = cam_dets(
        maze_with(east_victim(happy(), WEST)),
        cam_robot(0.0, 1.0, 55.0),
        e,
    );
    assert!(!d.is_empty());
    for (t, det) in &d {
        assert!(
            t - det.captured_t >= 0.1 - 1e-9,
            "{} vs {}",
            t,
            det.captured_t
        );
        assert!(t - det.captured_t < 0.12);
    }
    let fast = cam_dets(
        maze_with(east_victim(happy(), WEST)),
        cam_robot(0.0, 1.0, 55.0),
        ErrorProfile::clean(),
    );
    assert!(fast.iter().all(|(t, det)| t - det.captured_t < 0.02));
}

#[test]
fn camera_errors_misclassify_and_miss() {
    let mut e = ErrorProfile::clean();
    e.camera.class_confusion = 1.0;
    let d = cam_dets(
        maze_with(east_victim(happy(), WEST)),
        cam_robot(0.0, 1.0, 55.0),
        e,
    );
    assert!(!d.is_empty());
    assert!(d
        .iter()
        .all(|(_, x)| x.status != Some(VictimStatus::Unharmed)));
    let mut e = ErrorProfile::clean();
    e.camera.recall = 0.0;
    assert!(cam_dets(
        maze_with(east_victim(happy(), WEST)),
        cam_robot(0.0, 1.0, 55.0),
        e
    )
    .is_empty());
}

// ---- colour ----

fn classify(ch: &[f32; 14], mount: &ColorMount) -> TileKind {
    let g = |i: usize| ch[i] as f64;
    let lum = g(12) / (mount.gain * mount.integration_ms * crate::sensors::color::COUNT_SCALE);
    if lum < 0.15 {
        return TileKind::Black;
    }
    let blue = (g(1) + g(2) + g(3)) / 3.0;
    let red = (g(6) + g(7)) / 2.0;
    if blue > 1.5 * red {
        TileKind::Blue
    } else if red > 1.5 * blue {
        TileKind::Red
    } else if lum > 0.75 {
        TileKind::Normal
    } else {
        TileKind::Silver
    }
}

#[test]
fn colour_sensor_separates_the_five_surfaces() {
    let mut m = open_maze(5, 1);
    set_tile(&mut m, 1, 0, |t| t.kind = TileKind::Silver);
    set_tile(&mut m, 2, 0, |t| t.kind = TileKind::Blue);
    set_tile(&mut m, 3, 0, |t| t.kind = TileKind::Red);
    set_tile(&mut m, 4, 0, |t| t.kind = TileKind::Black);
    let robot = RobotConfig::reference();
    let mount = robot.color[0].clone();
    let rules = rules_2027_draft();
    for errors in [ErrorProfile::clean(), ErrorProfile::realistic()] {
        for seed in 1..=6u64 {
            let rng = SimRng::new(seed);
            let world = World::new(&m, &rules, &errors, &rng);
            let mut arr = crate::sensors::color::ColorArray::new(&robot, &errors.color, &rng);
            for c in 0..5u32 {
                let body = BodyState {
                    x: (c as f64 + 0.5) * T - mount.pose.x,
                    y: T / 2.0,
                    z: 0.0,
                    roll: 0.0,
                    pitch: 0.0,
                    yaw: 0.0,
                    v: 0.0,
                    omega: 0.0,
                };
                // Distinct sample times so every call is due (20 Hz).
                arr.sample_due(c as f64, &body, &world);
                let r = arr.observe();
                assert!(r[0].fresh);
                let kind = m.tile(TileXY::new(c, 0)).unwrap().kind;
                let want = if kind == TileKind::Start {
                    TileKind::Normal
                } else {
                    kind
                };
                assert_eq!(
                    classify(&r[0].ch, &mount),
                    want,
                    "{} seed {seed} tile {c}: {:?}",
                    errors.id,
                    r[0].ch
                );
            }
            // Not fresh again until the next period.
            assert!(!arr.observe()[0].fresh);
        }
    }
}

#[test]
fn colour_counts_scale_with_gain_and_integration() {
    let m = open_maze(1, 1);
    let rules = rules_2027_draft();
    let errors = ErrorProfile::clean();
    let rng = SimRng::new(1);
    let world = World::new(&m, &rules, &errors, &rng);
    let body = BodyState {
        x: 0.1,
        y: 0.14,
        z: 0.0,
        roll: 0.0,
        pitch: 0.0,
        yaw: 0.0,
        v: 0.0,
        omega: 0.0,
    };
    let clear = |gain: f64, ms: f64| {
        let mut robot = RobotConfig::reference();
        robot.color[0].gain = gain;
        robot.color[0].integration_ms = ms;
        let mut a = crate::sensors::color::ColorArray::new(&robot, &errors.color, &rng);
        a.sample_due(0.0, &body, &world);
        a.observe()[0].ch[12] as f64
    };
    let base = clear(4.0, 10.0);
    assert!((clear(8.0, 10.0) / base - 2.0).abs() < 1e-3);
    assert!((clear(4.0, 20.0) / base - 2.0).abs() < 1e-3);
}

#[test]
fn new_sensors_are_present_and_byte_identical_under_realistic() {
    let a = run_fixture(ErrorProfile::realistic(), 77, 15.0);
    let b = run_fixture(ErrorProfile::realistic(), 77, 15.0);
    assert_eq!(encode_msgpack(&a).unwrap(), encode_msgpack(&b).unwrap());
    assert!(a.frames.iter().any(|f| !f.color.is_empty()));
}

/// `cargo test --release -p mm-sim -- --ignored --nocapture`
#[test]
#[ignore]
fn perf_60s_reference_robot_at_least_10x_realtime() {
    let sim = Sim::new(input(
        fixture_maze(),
        RobotConfig::reference(),
        ErrorProfile::realistic(),
        99,
        60.0,
    ))
    .unwrap();
    let start = std::time::Instant::now();
    let r = sim.run(&mut Script(wobble));
    let wall = start.elapsed().as_secs_f64();
    let factor = 60.0 / wall;
    eprintln!(
        "60 s of sim in {wall:.3} s = {factor:.1}x realtime ({} frames)",
        r.frames.len()
    );
    assert!(factor >= 10.0, "only {factor:.1}x realtime");
}
