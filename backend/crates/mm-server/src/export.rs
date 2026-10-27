//! Schema and fixture generation, shared by the `mm-schemas` bin and the tests.

use std::path::Path;

use mm_controller::ValidationOutcome;
use mm_core::*;
use serde_json::Value;

use crate::routes;
use crate::runner::prepare;
use crate::state::AppState;
use crate::wire::*;

const CREATED_AT: &str = "2026-01-01T00:00:00Z";

fn fx<E: ToString>(e: E) -> serde_json::Error {
    serde::ser::Error::custom(e.to_string())
}

/// Fixtures that come from a real fixed-seed run on `maze_small.json` (seed 42, 10 s).
fn run_fixtures() -> Result<Vec<(&'static str, Value)>, serde_json::Error> {
    let maze: MazeGraph = serde_json::from_str(include_str!("../../../fixtures/maze_small.json"))?;
    let state = AppState::for_tests();
    state.store.insert_maze(&maze).map_err(fx)?;
    let maze_id = maze.header.id.clone().unwrap_or_default();
    let req = CreateRun {
        robot_id: crate::store::REFERENCE_ROBOT_ID.into(),
        maze_id,
        controller_id: "ctl_wall_follower".into(),
        rng: RngSpec {
            mode: RngMode::Fixed,
            seed: Some(42),
        },
        error_profile_id: "realistic".into(),
        speed: 0.0,
        max_duration_s: Some(10.0),
        lop_enabled: true,
        params: Default::default(),
    };
    let run_id = "run_000000000001";
    let p = prepare(&state, &req, run_id).map_err(fx)?;
    let code = p.code.clone();
    let mut sim = mm_sim::Sim::new(p.input).map_err(fx)?;
    let mut ctrl =
        mm_controller::RhaiController::new(&code, mm_controller::Limits::default()).map_err(fx)?;
    while let mm_sim::StepOutcome::Running = sim.step(&mut ctrl) {}
    let metrics = sim.metrics();
    let reason = sim.is_ended().unwrap_or(EndReason::TimeUp);
    let mut replay = sim.into_replay();
    replay.header.created_at = Some(CREATED_AT.into());

    let running = RunDetail {
        id: run_id.into(),
        state: RunState::Running,
        config: req.clone(),
        seed: 42,
        maze_id: req.maze_id.clone(),
        controller_id: req.controller_id.clone(),
        controller_version: 1,
        end_reason: None,
        metrics: None,
        created_at: CREATED_AT.into(),
        finished_at: None,
    };
    let detail = RunDetail {
        state: RunState::Finished,
        end_reason: Some(reason),
        metrics: Some(metrics.clone()),
        finished_at: Some(CREATED_AT.into()),
        ..running.clone()
    };
    let mut ws = vec![WsServerMessage::Hello { run: running, maze }];
    for i in [0usize, 25] {
        if let Some(f) = replay.frames.get(i) {
            ws.push(WsServerMessage::Frame { frame: f.clone() });
        }
    }
    for e in replay.events.iter().take(3) {
        ws.push(WsServerMessage::Event {
            t: e.t,
            event: e.kind.clone(),
        });
    }
    ws.push(WsServerMessage::State {
        state: RunState::Paused,
    });
    ws.push(WsServerMessage::State {
        state: RunState::Running,
    });
    ws.push(WsServerMessage::Dropped { count: 12 });
    ws.push(WsServerMessage::End { reason, metrics });

    let mut validation = mm_controller::validate(&code, &RobotConfig::reference());
    validation.dry_run_trace.truncate(2);
    let saved = ControllerSaved {
        id: "ctl_000000000001".into(),
        version: 2,
        tunables: validation.tunables.clone(),
        validation,
    };
    Ok(vec![
        (
            "run_created.json",
            serde_json::to_value(RunCreated {
                id: run_id.into(),
                seed: 42,
                state: RunState::Queued,
            })?,
        ),
        ("run_detail.json", serde_json::to_value(detail)?),
        ("ws_messages.json", serde_json::to_value(ws)?),
        ("controller_saved.json", serde_json::to_value(saved)?),
    ])
}

macro_rules! schemas {
    ($($t:ty),* $(,)?) => {
        Ok(vec![$((stringify!($t), serde_json::to_value(schemars::schema_for!($t))?)),*])
    };
}

/// JSON Schema for every top-level wire type.
pub fn schemas() -> Result<Vec<(&'static str, Value)>, serde_json::Error> {
    schemas!(
        RobotConfig,
        ErrorProfile,
        RulesProfile,
        MazeGraph,
        GenerateRequest,
        MazeValidation,
        CreateRun,
        Replay,
        Frame,
        Event,
        Metrics,
        Observations,
        Commands,
        TunableParam,
        MuseReport,
        ValidationOutcome,
        ValidateController,
        SaveController,
        ControllerSaved,
        ControllerSummary,
        ControllerVersion,
        ControllerDetail,
        ConversionPrompt,
        RunCreated,
        RunSummary,
        RunDetail,
        StateBody,
        BatchRequest,
        BatchResponse,
        WsServerMessage,
        WsClientMessage,
    )
}

/// Fixtures that come straight from code (the maze and replay ones are hand-written).
pub fn generated_fixtures() -> Result<Vec<(&'static str, Value)>, serde_json::Error> {
    let rules = rules_2027_draft();
    let presets = ErrorProfile::presets();
    let example = presets
        .iter()
        .find(|p| p.id == "realistic")
        .or_else(|| presets.first())
        .cloned();
    let req = GenerateRequest {
        seed: Some(42),
        grid: GridSize { w: 8, h: 8 },
        rules: rules.id.clone(),
        features: FeatureCounts::default(),
        name: Some("Example 8x8".into()),
    };
    let (mut maze, warnings) = routes::mazes::generate_maze(&req, 42, &rules)
        .map_err(|e| serde::ser::Error::custom(e.to_string()))?;
    maze.header.id = Some("maz_000000000042".into());
    let mut out = run_fixtures()?;
    out.extend(vec![
        ("maze_generated.json", serde_json::to_value(&maze)?),
        (
            "maze_generate_response.json",
            routes::mazes::generate_response(&maze, &warnings),
        ),
        ("health.json", routes::health::health_body(false)),
        ("spec.json", routes::spec::spec_body(&rules)),
        (
            "robot_reference.json",
            serde_json::to_value(RobotConfig::reference())?,
        ),
        ("error_profiles.json", serde_json::to_value(&presets)?),
        (
            "rules_rcj_maze_2027_draft.json",
            serde_json::to_value(&rules)?,
        ),
        ("error_example.json", serde_json::to_value(example)?),
    ]);
    Ok(out)
}

fn write_json(path: &Path, v: &Value) -> anyhow::Result<()> {
    let mut s = serde_json::to_string_pretty(v)?;
    s.push('\n');
    std::fs::write(path, s)?;
    Ok(())
}

/// Writes `<dir>/<TypeName>.json` for every schema; returns the file names.
pub fn write_schemas(dir: &Path) -> anyhow::Result<Vec<String>> {
    std::fs::create_dir_all(dir)?;
    let mut names = Vec::new();
    for (name, schema) in schemas()? {
        write_json(&dir.join(format!("{name}.json")), &schema)?;
        names.push(format!("{name}.json"));
    }
    Ok(names)
}

pub fn write_fixtures(dir: &Path) -> anyhow::Result<Vec<String>> {
    std::fs::create_dir_all(dir)?;
    let mut names = Vec::new();
    for (name, v) in generated_fixtures()? {
        write_json(&dir.join(name), &v)?;
        names.push(name.to_string());
    }
    Ok(names)
}
