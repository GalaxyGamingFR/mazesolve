//! Shared MazeMuse types. Every wire type lives here so the server, sim, tuner and
//! frontend fixtures agree on one shape. This crate does no I/O.
//!
//! Conventions: meters, radians, seconds internally. World frame is x east, y north,
//! z up; yaw 0 points along +x and grows counter-clockwise. Tile `(col, row)` covers
//! `[col*t, (col+1)*t] x [row*t, (row+1)*t]` where `t` is the rules' tile size.

pub mod error_profile;
pub mod geom;
pub mod maze;
pub mod metrics;
pub mod obs;
pub mod replay;
pub mod robot;
pub mod rules;
pub mod run;
pub mod tuner;

pub use error_profile::*;
pub use geom::*;
pub use maze::*;
pub use metrics::*;
pub use obs::*;
pub use replay::*;
pub use robot::*;
pub use rules::*;
pub use run::*;
pub use tuner::*;

/// Wire format version of every type in this crate. Bumped with `docs/API.md`.
pub const API_VERSION: &str = "0.2.0";

/// Seeds are kept below 2^53 so JavaScript clients can hold them as numbers.
pub const SEED_MASK: u64 = (1u64 << 53) - 1;

/// The bundled 2027 draft profile, so tests and the CLI work without a file path.
pub const RULES_2027_DRAFT_JSON: &str =
    include_str!("../../../rules_profiles/rcj_maze_2027_draft.json");

pub fn rules_2027_draft() -> RulesProfile {
    // The bundled file is checked by the test below, so this cannot fail at runtime.
    RulesProfile::from_json(RULES_2027_DRAFT_JSON).unwrap_or_else(|e| panic!("bundled rules: {e}"))
}

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("invalid rules profile: {0}")]
    Rules(String),
    #[error("invalid config: {0}")]
    Config(String),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_rules_load() {
        let r = rules_2027_draft();
        assert_eq!(r.field.tile_m, 0.28);
        assert_eq!(r.ring_value(RingColor::Blue), 2);
        assert_eq!(r.scoring.victim_points.floating_cognitive, 30);
    }

    #[test]
    fn reference_robot_is_valid() {
        let r = rules_2027_draft();
        let issues = RobotConfig::reference().validate(&r);
        assert!(
            issues.iter().all(|i| i.severity != Severity::Error),
            "{issues:?}"
        );
        assert_eq!(RobotConfig::reference().tof.len(), 8);
    }

    #[test]
    fn presets_round_trip() {
        for p in ErrorProfile::presets() {
            let s = serde_json::to_string(&p).unwrap();
            assert_eq!(serde_json::from_str::<ErrorProfile>(&s).unwrap(), p);
        }
    }

    #[test]
    fn score_floor_is_zero() {
        let mut s = ScoreBreakdown {
            penalties: -20,
            victims: 5,
            ..Default::default()
        };
        s.sum();
        assert_eq!(s.total, 0);
    }
}
