//! Schema and fixture generation, shared by the `mm-schemas` bin and the tests.

use std::path::Path;

use mm_core::*;
use serde_json::Value;

use crate::routes;

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
    Ok(vec![
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
    ])
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
