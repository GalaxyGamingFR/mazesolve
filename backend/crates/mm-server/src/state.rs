//! Shared server state. Storage (robots, mazes, controllers, runs, jobs) joins here in M2+.

use std::sync::Arc;

use mm_core::RulesProfile;

#[derive(Clone)]
pub struct AppState {
    pub rules: Arc<RulesProfile>,
}

impl AppState {
    pub fn new(rules: RulesProfile) -> Self {
        AppState {
            rules: Arc::new(rules),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        AppState::new(mm_core::rules_2027_draft())
    }
}
