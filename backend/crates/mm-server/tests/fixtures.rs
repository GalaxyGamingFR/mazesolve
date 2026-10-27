use std::path::PathBuf;

use mm_core::*;
use serde::de::DeserializeOwned;
use serde_json::Value;

fn read(name: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn parse<T: DeserializeOwned>(name: &str) {
    if let Err(e) = serde_json::from_str::<T>(&read(name)) {
        panic!("{name}: {e}");
    }
}

#[test]
fn fixtures_deserialize_into_core_types() {
    parse::<RobotConfig>("robot_reference.json");
    parse::<Vec<ErrorProfile>>("error_profiles.json");
    parse::<ErrorProfile>("error_example.json");
    parse::<RulesProfile>("rules_rcj_maze_2027_draft.json");
    parse::<MazeGraph>("maze_small.json");
    parse::<MazeGraph>("maze_generated.json");
    parse::<Replay>("replay_small.json");
    parse::<Value>("health.json");
    parse::<Value>("spec.json");
}

#[test]
fn generated_fixtures_are_current() {
    for (name, v) in mm_server::export::generated_fixtures().unwrap() {
        // Compare text, not parsed values: serde_json's default float parse is not exact.
        let want = format!("{}\n", serde_json::to_string_pretty(&v).unwrap());
        assert!(
            read(name) == want,
            "{name} is stale: run `cargo run -p mm-server --bin mm-schemas`"
        );
    }
}

#[test]
fn small_maze_is_consistent() {
    let m: MazeGraph = serde_json::from_str(&read("maze_small.json")).unwrap();
    assert_eq!(m.tiles.len(), (m.grid.w * m.grid.h) as usize);
    for (i, t) in m.tiles.iter().enumerate() {
        assert_eq!(m.idx(t.at), Some(i));
    }
}
