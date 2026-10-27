use std::collections::BTreeMap;

use criterion::{criterion_group, criterion_main, Criterion};
use mm_core::*;
use mm_sim::{ConfigHashes, Sim, SimInput};

struct Wobble;

impl Controller for Wobble {
    fn init(&mut self, _: &RobotInfo) -> Result<(), String> {
        Ok(())
    }
    fn step(&mut self, obs: &Observations) -> Result<Commands, String> {
        let s = (obs.t * 1.3).sin() * 0.05;
        Ok(Commands {
            wheel_v: [0.15 - s, 0.15 + s, 0.15 - s, 0.15 + s],
            ..Default::default()
        })
    }
}

fn sim_60s(c: &mut Criterion) {
    let maze: MazeGraph =
        serde_json::from_str(include_str!("../../../fixtures/maze_small.json")).unwrap();
    c.bench_function("sim_60s_reference_robot", |b| {
        b.iter(|| {
            let sim = Sim::new(SimInput {
                maze: maze.clone(),
                robot: RobotConfig::reference(),
                rules: rules_2027_draft(),
                errors: ErrorProfile::realistic(),
                settings: RunSettings {
                    seed: 99,
                    rng_mode: RngMode::Fixed,
                    timing: SimTiming::default(),
                    max_duration_s: Some(60.0),
                    lop_enabled: true,
                },
                params: BTreeMap::new(),
                run_id: "bench".into(),
                hashes: ConfigHashes::default(),
            })
            .unwrap();
            sim.run(&mut Wobble)
        })
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = sim_60s
}
criterion_main!(benches);
