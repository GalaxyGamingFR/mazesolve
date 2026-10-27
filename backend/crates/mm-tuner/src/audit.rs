//! Audit trail: every prompt, response, delta, guardrail action and decision.
//! Payloads never contain the API key (it is never passed into this crate's prompts).

use std::time::Instant;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum AuditKind {
    Prompt,
    Response,
    Delta,
    GuardrailClamp,
    GuardrailDrop,
    Accept,
    Rollback,
    RunResult,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AuditEntry {
    /// 0 for Mode A and for the baseline evaluation.
    pub iteration: u32,
    pub kind: AuditKind,
    pub payload: serde_json::Value,
    /// Milliseconds since the log was created.
    pub t_ms: u64,
}

#[derive(Debug)]
pub struct AuditLog {
    start: Instant,
    pub entries: Vec<AuditEntry>,
}

impl Default for AuditLog {
    fn default() -> Self {
        Self::new()
    }
}

impl AuditLog {
    pub fn new() -> Self {
        AuditLog {
            start: Instant::now(),
            entries: Vec::new(),
        }
    }

    pub fn push(&mut self, iteration: u32, kind: AuditKind, payload: serde_json::Value) {
        self.entries.push(AuditEntry {
            iteration,
            kind,
            payload,
            t_ms: self.start.elapsed().as_millis() as u64,
        });
    }
}
