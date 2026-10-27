//! Muse-driven tuning for MazeMuse controllers.
//!
//! Mode A ([`suggest`]) reads a simulator dossier and returns a structured report.
//! Mode B ([`autotune`]) closes the loop: suggest, clamp, apply, re-run, accept or
//! roll back. Guardrails are documented in `docs/TUNER_GUARDRAILS.md`.

pub mod audit;
pub mod autotune;
pub mod dossier;
pub mod provider;
pub mod schema;
pub mod suggest;

pub use audit::{AuditEntry, AuditKind, AuditLog};
pub use autotune::{
    autotune, AutoTuneConfig, AutoTuneOutcome, CodeProposal, IterationRecord, ParamDelta, Progress,
    RunExecutor, RunRequest, RunResult, StopReason,
};
pub use dossier::{build_dossier, Dossier, DossierInput};
pub use provider::{
    provider_from_env, test_connection, ChatProvider, ChatRequest, ChatResponse, ConnectionInfo,
    MockProvider, MuseConfig, MuseProvider, Usage,
};
pub use suggest::{suggest, SuggestConfig, SuggestOutcome};

/// Every way a tuner operation can fail.
#[derive(Debug, thiserror::Error)]
pub enum TunerError {
    #[error("Meta model API key not configured (set META_MODEL_API_KEY)")]
    NotConfigured,
    #[error("upstream error {status}: {message}")]
    Upstream { status: u16, message: String },
    #[error("request timed out")]
    Timeout,
    #[error("could not parse model output: {0}")]
    Parse(String),
    #[error("guardrail: {0}")]
    Guardrail(String),
    #[error("run failed: {0}")]
    Run(String),
    #[error("cancelled")]
    Cancelled,
}
