//! Mode A: one dossier in, one structured report out. Nothing is applied.

use mm_core::MuseReport;
use serde_json::json;

use crate::audit::{AuditEntry, AuditKind, AuditLog};
use crate::dossier::Dossier;
use crate::provider::{ChatProvider, Usage};
use crate::schema::request_report;
use crate::TunerError;

pub const SYSTEM_PROMPT: &str = "You are an expert RoboCup Junior Rescue Maze 2027 engineer. \
You diagnose why a simulated robot lost points from a compact simulator dossier (config, score \
breakdown, failure timeline with sensor windows, tunable parameter table, controller excerpt). \
Find the root cause, cite evidence from the dossier (times, tiles, ranges), and propose a small \
number of fixes. Prefer parameter changes: every parameter suggestion must use a path exactly \
as listed in the tunable table, and new_value must lie within the listed [min, max] and within \
max_step of the current value. Only propose a code change (unified diff in `diff`) when no \
parameter can address the cause. For param targets set diff to \"\"; for code targets set path \
to \"\" and new_value to 0. Do not invent paths. Reply with JSON matching the schema only.";

#[derive(Debug, Clone)]
pub struct SuggestConfig {
    pub max_tokens: u32,
    pub temperature: f32,
}

impl Default for SuggestConfig {
    fn default() -> Self {
        SuggestConfig {
            max_tokens: 4096,
            temperature: 0.2,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SuggestOutcome {
    pub report: MuseReport,
    pub usage: Usage,
    pub audit: Vec<AuditEntry>,
}

/// Shared by Mode A and Mode B; records prompts and responses into `log`.
pub(crate) async fn suggest_logged(
    p: &dyn ChatProvider,
    dossier: &Dossier,
    cfg: &SuggestConfig,
    iteration: u32,
    log: &mut AuditLog,
) -> Result<(MuseReport, Usage), TunerError> {
    let user = format!(
        "{}\n\nTask: diagnose the failures above and propose fixes. Respect the parameter table.",
        dossier.text
    );
    let call = request_report(p, SYSTEM_PROMPT, &user, cfg.max_tokens, cfg.temperature).await?;
    for ex in &call.exchanges {
        log.push(
            iteration,
            AuditKind::Prompt,
            json!({"system": ex.system, "user": ex.user}),
        );
        log.push(
            iteration,
            AuditKind::Response,
            json!({"content": ex.response}),
        );
    }
    Ok((call.report, call.usage))
}

pub async fn suggest(
    p: &dyn ChatProvider,
    dossier: &Dossier,
    cfg: &SuggestConfig,
) -> Result<SuggestOutcome, TunerError> {
    let mut log = AuditLog::new();
    let (report, usage) = suggest_logged(p, dossier, cfg, 0, &mut log).await?;
    Ok(SuggestOutcome {
        report,
        usage,
        audit: log.entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::MockProvider;

    #[tokio::test]
    async fn suggest_returns_report_usage_and_audit() {
        let body = json!({
            "diagnosis": "d", "root_cause": "r", "evidence": [], "suggestions": []
        })
        .to_string();
        let p = MockProvider::scripted([body]);
        let d = Dossier {
            text: "DOSSIER".into(),
            truncated: false,
        };
        let o = suggest(&p, &d, &SuggestConfig::default()).await.unwrap();
        assert_eq!(o.report.diagnosis, "d");
        assert_eq!(o.audit.len(), 2);
        assert!(p.requests()[0].user.contains("DOSSIER"));
        assert!(p.requests()[0].system.contains("Rescue Maze 2027"));
    }
}
