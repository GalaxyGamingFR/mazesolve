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
