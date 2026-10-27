//! Shared server state: the rules profile and the SQLite store.

use std::path::PathBuf;
use std::sync::Arc;

use mm_core::RulesProfile;

use crate::runner::RunRegistry;
use crate::store::Store;

#[derive(Clone)]
pub struct AppState {
    pub rules: Arc<RulesProfile>,
    pub store: Arc<Store>,
    /// Runs whose simulation thread is still alive.
    pub runs: Arc<RunRegistry>,
}

impl AppState {
    pub fn new(rules: RulesProfile, store: Store) -> Self {
        AppState {
            rules: Arc::new(rules),
            store: Arc::new(store),
            runs: Arc::new(RunRegistry::default()),
        }
    }

    /// Opens `${MM_DATA_DIR:-./data}/mazemuse.db`.
    pub fn open_default() -> anyhow::Result<Self> {
        let dir = std::env::var("MM_DATA_DIR").unwrap_or_else(|_| "./data".into());
        let store = Store::open(&PathBuf::from(dir).join("mazemuse.db"))?;
        Ok(AppState::new(mm_core::rules_2027_draft(), store))
    }

    /// Fresh in-memory database; each call is an isolated world.
    pub fn for_tests() -> Self {
        let store = Store::in_memory().expect("in-memory database");
        AppState::new(mm_core::rules_2027_draft(), store)
    }
}
