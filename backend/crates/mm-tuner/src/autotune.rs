//! Mode B: closed-loop tuning. Suggest, clamp, apply, re-run, accept or roll back.
//!
//! Code suggestions (`SuggestionTarget::Code`) are recorded and marked
//! `pending_review`; v1 never applies a diff, even with `allow_code_changes`.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use mm_core::{CreateRun, Metrics, MuseReport, Replay, SuggestionTarget, TunableParam};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::audit::{AuditEntry, AuditKind, AuditLog};
use crate::dossier::{build_dossier, DossierInput};
use crate::provider::{ChatProvider, Usage};
use crate::suggest::{suggest_logged, SuggestConfig};
use crate::TunerError;

#[derive(Debug, Clone)]
pub struct RunRequest {
    pub base: CreateRun,
    pub params: BTreeMap<String, f64>,
    pub seed: u64,
}

#[derive(Debug, Clone)]
pub struct RunResult {
    pub run_id: String,
    pub metrics: Metrics,
    pub replay: Replay,
}

/// Implemented by the server and the CLI.
#[async_trait]
pub trait RunExecutor: Send + Sync {
    async fn run(&self, req: RunRequest) -> Result<RunResult, TunerError>;
}

#[derive(Debug, Clone)]
pub struct AutoTuneConfig {
    pub base: CreateRun,
    pub baseline_seed: u64,
    pub fresh_seeds: Vec<u64>,
    pub params_allowlist: Vec<TunableParam>,
    pub max_iterations: u32,
    pub budget: Duration,
    /// v1: code diffs are recorded as `pending_review` and never applied.
    pub allow_code_changes: bool,
    /// Minimum objective gain (mean total score) to accept a candidate.
    pub min_improvement: f64,
    pub controller_code: Option<String>,
    pub robot_summary: String,
    pub suggest: SuggestConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    pub iteration: u32,
    pub phase: String,
    pub current_score: f64,
    pub best_score: f64,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ParamDelta {
    pub path: String,
    pub from: f64,
    pub to: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CodeProposal {
    pub title: String,
    pub diff: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct IterationRecord {
    pub iteration: u32,
    pub diagnosis: String,
    pub deltas: Vec<ParamDelta>,
    pub pending_code: Vec<CodeProposal>,
    pub score_before: f64,
    pub score_after: Option<f64>,
    pub accepted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum StopReason {
    MaxIterations,
    Budget,
    Cancelled,
    NoValidSuggestions,
}

#[derive(Debug, Clone)]
pub struct AutoTuneOutcome {
    pub best_params: BTreeMap<String, f64>,
    pub baseline_score: f64,
    pub best_score: f64,
    pub iterations: Vec<IterationRecord>,
    pub audit: Vec<AuditEntry>,
    pub usage: Usage,
    pub stop_reason: StopReason,
}

// ------------------------------------------------------------ guardrails

/// Apply allowlist, bounds, step limit and NaN rules to a report. Returns accepted
/// deltas and code proposals; every drop/clamp goes to `log`.
pub fn apply_guardrails(
    report: &MuseReport,
    allow: &[TunableParam],
    current: &BTreeMap<String, f64>,
    iteration: u32,
    log: &mut AuditLog,
) -> (Vec<ParamDelta>, Vec<CodeProposal>) {
    let mut deltas: Vec<ParamDelta> = Vec::new();
    let mut code = Vec::new();
    for s in &report.suggestions {
        match &s.target {
            SuggestionTarget::Code { diff } => {
                log.push(
                    iteration,
                    AuditKind::Delta,
                    json!({"kind": "code", "title": s.title, "diff": diff, "status": "pending_review"}),
                );
                code.push(CodeProposal {
                    title: s.title.clone(),
                    diff: diff.clone(),
                    status: "pending_review".into(),
                });
            }
            SuggestionTarget::Param { path, new_value } => {
                let drop = |log: &mut AuditLog, reason: &str| {
                    log.push(
                        iteration,
                        AuditKind::GuardrailDrop,
                        json!({"path": path, "requested": new_value.to_string(), "reason": reason}),
                    );
                };
                let Some(p) = allow.iter().find(|p| &p.path == path) else {
                    drop(log, "not_allowlisted");
                    continue;
                };
                if !new_value.is_finite() {
                    drop(log, "non_finite");
                    continue;
                }
                if deltas.iter().any(|d| &d.path == path) {
                    drop(log, "duplicate_path");
                    continue;
                }
                let cur = current.get(path).copied().unwrap_or(p.default);
                let lo = p.min.max(cur - p.max_step);
                let hi = p.max.min(cur + p.max_step);
                let applied = if lo <= hi {
                    new_value.clamp(lo, hi)
                } else {
                    cur
                };
                if (applied - *new_value).abs() > 1e-12 {
                    log.push(
                        iteration,
                        AuditKind::GuardrailClamp,
                        json!({
                            "path": path, "requested": new_value, "applied": applied,
                            "min": p.min, "max": p.max, "max_step": p.max_step, "current": cur
                        }),
                    );
                }
                if (applied - cur).abs() < 1e-12 {
                    drop(log, "no_change");
                    continue;
                }
                log.push(
                    iteration,
                    AuditKind::Delta,
                    json!({"kind": "param", "path": path, "from": cur, "to": applied}),
                );
                deltas.push(ParamDelta {
                    path: path.clone(),
                    from: cur,
                    to: applied,
                });
            }
        }
    }
    (deltas, code)
}

// ------------------------------------------------------------ evaluation

enum Interrupt {
    Cancelled,
    Budget,
    Fail(TunerError),
}

struct Eval {
    score: f64,
    restarts: u32,
    collisions: u32,
    base_run: RunResult,
}

/// Objective ordering: higher mean score wins by at least `min_imp`; an equal score
/// falls back to fewer LoP restarts, then fewer collisions.
fn better(a: &Eval, b: &Eval, min_imp: f64) -> bool {
    let d = a.score - b.score;
    if d.abs() <= 1e-9 {
        (a.restarts, a.collisions) < (b.restarts, b.collisions)
    } else {
        d > 0.0 && d >= min_imp
    }
}

struct Ctl<'a> {
    exec: &'a dyn RunExecutor,
    cfg: &'a AutoTuneConfig,
    cancel: &'a AtomicBool,
    deadline: Instant,
}

impl Ctl<'_> {
    async fn evaluate(
        &self,
        params: &BTreeMap<String, f64>,
        iteration: u32,
        log: &mut AuditLog,
    ) -> Result<Eval, Interrupt> {
        let mut seeds = vec![self.cfg.baseline_seed];
        seeds.extend(self.cfg.fresh_seeds.iter().copied());
        let (mut sum, mut restarts, mut collisions) = (0.0, 0, 0);
        let mut base_run = None;
        for seed in &seeds {
            if self.cancel.load(Ordering::SeqCst) {
                return Err(Interrupt::Cancelled);
            }
            if Instant::now() >= self.deadline {
                return Err(Interrupt::Budget);
            }
            let r = self
                .exec
                .run(RunRequest {
                    base: self.cfg.base.clone(),
                    params: params.clone(),
                    seed: *seed,
                })
                .await
                .map_err(|e| match e {
                    TunerError::Cancelled => Interrupt::Cancelled,
                    e => Interrupt::Fail(e),
                })?;
            sum += r.metrics.score.total as f64;
            restarts += r.metrics.restarts;
            collisions += r.metrics.collisions;
            log.push(
                iteration,
                AuditKind::RunResult,
                json!({
                    "run_id": r.run_id, "seed": seed, "score": r.metrics.score.total,
                    "restarts": r.metrics.restarts, "collisions": r.metrics.collisions,
                    "params": params
                }),
            );
            if base_run.is_none() {
                base_run = Some(r);
            }
        }
        Ok(Eval {
            score: sum / seeds.len() as f64,
            restarts,
            collisions,
            base_run: base_run.expect("at least the baseline seed"),
        })
    }
}

pub async fn autotune<F>(
    p: &dyn ChatProvider,
    exec: &dyn RunExecutor,
    cfg: AutoTuneConfig,
    progress: F,
    cancel: Arc<AtomicBool>,
) -> Result<AutoTuneOutcome, TunerError>
where
    F: Fn(Progress) + Send + Sync,
{
    let started = Instant::now();
    let ctl = Ctl {
        exec,
        cfg: &cfg,
        cancel: &cancel,
        deadline: started + cfg.budget,
    };
    let mut log = AuditLog::new();
    let mut usage = Usage::default();

    let mut current: BTreeMap<String, f64> = cfg
        .params_allowlist
        .iter()
        .map(|t| (t.path.clone(), t.default))
        .collect();
    for (k, v) in &cfg.base.params {
        current.insert(k.clone(), *v);
    }

    let mut cur = match ctl.evaluate(&current, 0, &mut log).await {
        Ok(e) => e,
        Err(Interrupt::Cancelled) => return Err(TunerError::Cancelled),
        Err(Interrupt::Budget) => {
            return Err(TunerError::Guardrail(
                "budget exhausted before baseline".into(),
            ))
        }
        Err(Interrupt::Fail(e)) => return Err(e),
    };
    let baseline_score = cur.score;
    let mut iterations = Vec::new();
    let mut no_valid = 0u32;
    let mut stop = StopReason::MaxIterations;
    let emit = |iteration: u32, phase: &str, cur: f64, message: String| {
        progress(Progress {
            iteration,
            phase: phase.into(),
            current_score: cur,
            best_score: cur,
            message,
        })
    };

    'outer: for it in 1..=cfg.max_iterations {
        if cancel.load(Ordering::SeqCst) {
            stop = StopReason::Cancelled;
            break;
        }
        if Instant::now() >= ctl.deadline {
            stop = StopReason::Budget;
            break;
        }
        emit(
            it,
            "suggesting",
            cur.score,
            format!("iteration {it}: asking Muse"),
        );

        let dossier = build_dossier(&DossierInput {
            replay: &cur.base_run.replay,
            metrics: &cur.base_run.metrics,
            params: &cfg.params_allowlist,
            current: &current,
            controller_code: cfg.controller_code.as_deref(),
            robot_summary: cfg.robot_summary.clone(),
            error_profile_id: cfg.base.error_profile_id.clone(),
        });
        let (report, u) = match suggest_logged(p, &dossier, &cfg.suggest, it, &mut log).await {
            Ok(x) => x,
            Err(TunerError::Parse(m)) => {
                log.push(
                    it,
                    AuditKind::GuardrailDrop,
                    json!({"reason": "unparseable_response", "error": m}),
                );
                no_valid += 1;
                if no_valid >= 2 {
                    stop = StopReason::NoValidSuggestions;
                    break;
                }
                continue;
            }
            Err(e) => return Err(e),
        };
        usage.add(u);

        let (deltas, pending_code) =
            apply_guardrails(&report, &cfg.params_allowlist, &current, it, &mut log);
        if deltas.is_empty() {
            no_valid += 1;
            iterations.push(IterationRecord {
                iteration: it,
                diagnosis: report.diagnosis,
                deltas,
                pending_code,
                score_before: cur.score,
                score_after: None,
                accepted: false,
            });
            if no_valid >= 2 {
                stop = StopReason::NoValidSuggestions;
                break;
            }
            continue;
        }
        no_valid = 0;

        let mut cand = current.clone();
        for d in &deltas {
            cand.insert(d.path.clone(), d.to);
        }
        emit(
            it,
            "evaluating",
            cur.score,
            format!("iteration {it}: testing {} change(s)", deltas.len()),
        );
        let eval = match ctl.evaluate(&cand, it, &mut log).await {
            Ok(e) => e,
            Err(Interrupt::Cancelled) => {
                stop = StopReason::Cancelled;
                break 'outer;
            }
            Err(Interrupt::Budget) => {
                stop = StopReason::Budget;
                break 'outer;
            }
            Err(Interrupt::Fail(e)) => return Err(e),
        };
        let accepted = better(&eval, &cur, cfg.min_improvement);
        let payload = json!({
            "before": cur.score, "after": eval.score,
            "restarts_before": cur.restarts, "restarts_after": eval.restarts,
            "collisions_before": cur.collisions, "collisions_after": eval.collisions,
        });
        iterations.push(IterationRecord {
            iteration: it,
            diagnosis: report.diagnosis,
            deltas,
            pending_code,
            score_before: cur.score,
            score_after: Some(eval.score),
            accepted,
        });
        if accepted {
            log.push(it, AuditKind::Accept, payload);
            current = cand;
            cur = eval;
            emit(
                it,
                "accepted",
                cur.score,
                format!("iteration {it}: accepted, score {:.1}", cur.score),
            );
        } else {
            log.push(it, AuditKind::Rollback, payload);
            emit(
                it,
                "rolled_back",
                cur.score,
                format!("iteration {it}: rolled back"),
            );
        }
    }

    Ok(AutoTuneOutcome {
        best_params: current,
        baseline_score,
        best_score: cur.score,
        iterations,
        audit: log.entries,
        usage,
        stop_reason: stop,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dossier::tests::header;
    use crate::provider::MockProvider;
    use mm_core::{RngMode, RngSpec};
    use std::sync::atomic::AtomicUsize;

    const PATH: &str = "controller.wall_dist";

    fn tp() -> TunableParam {
        TunableParam {
            path: PATH.into(),
            description: "wall distance".into(),
            min: 0.05,
            max: 0.3,
            default: 0.2,
            max_step: 0.03,
        }
    }

    fn base() -> CreateRun {
        CreateRun {
            robot_id: "r".into(),
            maze_id: "m".into(),
            controller_id: "c".into(),
            rng: RngSpec {
                mode: RngMode::Fixed,
                seed: Some(1),
            },
            error_profile_id: "clean".into(),
            speed: 0.0,
            max_duration_s: None,
            lop_enabled: true,
            params: BTreeMap::new(),
        }
    }

    fn cfg(max_iterations: u32) -> AutoTuneConfig {
        AutoTuneConfig {
            base: base(),
            baseline_seed: 1,
            fresh_seeds: vec![2, 3],
            params_allowlist: vec![tp()],
            max_iterations,
            budget: Duration::from_secs(60),
            allow_code_changes: false,
            min_improvement: 1.0,
            controller_code: None,
            robot_summary: "test".into(),
            suggest: SuggestConfig::default(),
        }
    }

    /// Score peaks at wall_dist = 0.12.
    struct Fake {
        runs: AtomicUsize,
    }

    #[async_trait]
    impl RunExecutor for Fake {
        async fn run(&self, req: RunRequest) -> Result<RunResult, TunerError> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            let w = req.params.get(PATH).copied().unwrap_or(0.2);
            let mut m = Metrics::default();
            m.score.total = (1000.0 - 20000.0 * (w - 0.12).powi(2)).round() as i32;
            Ok(RunResult {
                run_id: format!("run-{}", req.seed),
                metrics: m,
                replay: Replay {
                    header: header(),
                    frames: vec![],
                    events: vec![],
                },
            })
        }
    }

    fn reply(path: &str, v: f64) -> String {
        json!({
            "diagnosis": "too far from wall", "root_cause": "r", "evidence": [],
            "suggestions": [{"title": "t", "detail": "d",
              "target": {"type": "param", "path": path, "new_value": v, "diff": ""},
              "expected_effect": "e", "confidence": 0.5}]
        })
        .to_string()
    }

    fn report_with(targets: Vec<SuggestionTarget>) -> MuseReport {
        MuseReport {
            diagnosis: String::new(),
            root_cause: String::new(),
            evidence: vec![],
            suggestions: targets
                .into_iter()
                .map(|target| mm_core::Suggestion {
                    title: "t".into(),
                    detail: "d".into(),
                    target,
                    expected_effect: "e".into(),
                    confidence: 0.5,
                })
                .collect(),
        }
    }

    fn param(path: &str, v: f64) -> SuggestionTarget {
        SuggestionTarget::Param {
            path: path.into(),
            new_value: v,
        }
    }

    #[test]
    fn clamps_to_step_and_bounds_and_drops_bad() {
        let mut log = AuditLog::new();
        let cur: BTreeMap<_, _> = [(PATH.to_string(), 0.2)].into();
        // Step clamp: 0.2 -> wants 0.05 -> allowed 0.17.
        let (d, _) = apply_guardrails(
            &report_with(vec![param(PATH, 0.05)]),
            &[tp()],
            &cur,
            1,
            &mut log,
        );
        assert!((d[0].to - 0.17).abs() < 1e-9);
        assert!(log
            .entries
            .iter()
            .any(|e| e.kind == AuditKind::GuardrailClamp));
        // Bounds clamp: near max.
        let cur: BTreeMap<_, _> = [(PATH.to_string(), 0.29)].into();
        let (d, _) = apply_guardrails(
            &report_with(vec![param(PATH, 9.0)]),
            &[tp()],
            &cur,
            1,
            &mut log,
        );
        assert!((d[0].to - 0.3).abs() < 1e-9);
        // Drops: unknown path, NaN, no change.
        let mut log = AuditLog::new();
        let (d, _) = apply_guardrails(
            &report_with(vec![
                param("controller.evil", 1.0),
                param(PATH, f64::NAN),
                param(PATH, 0.29),
            ]),
            &[tp()],
            &cur,
            1,
            &mut log,
        );
        assert!(d.is_empty());
        let drops: Vec<_> = log
            .entries
            .iter()
            .filter(|e| e.kind == AuditKind::GuardrailDrop)
            .map(|e| e.payload["reason"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(drops, ["not_allowlisted", "non_finite", "no_change"]);
    }

    #[test]
    fn code_suggestions_are_pending_review_only() {
        let mut log = AuditLog::new();
        let (d, c) = apply_guardrails(
            &report_with(vec![SuggestionTarget::Code { diff: "@@".into() }]),
            &[tp()],
            &BTreeMap::new(),
            1,
            &mut log,
        );
        assert!(d.is_empty());
        assert_eq!(c[0].status, "pending_review");
    }

    #[tokio::test]
    async fn climbs_to_peak_and_accepts() {
        let exec = Fake {
            runs: AtomicUsize::new(0),
        };
        let p = MockProvider::scripted((0..8).map(|_| reply(PATH, 0.12)));
        let out = autotune(&p, &exec, cfg(10), |_| {}, Arc::new(AtomicBool::new(false)))
            .await
            .unwrap();
        assert!(
            (out.best_params[PATH] - 0.12).abs() < 1e-9,
            "{:?}",
            out.best_params
        );
        assert!(out.best_score > out.baseline_score);
        assert_eq!(out.iterations.iter().filter(|i| i.accepted).count(), 3);
        assert_eq!(out.stop_reason, StopReason::NoValidSuggestions);
        assert!(out.audit.iter().any(|e| e.kind == AuditKind::Accept));
        assert!(out.usage.prompt_tokens > 0);
    }

    #[tokio::test]
    async fn worsening_suggestion_is_rolled_back() {
        let exec = Fake {
            runs: AtomicUsize::new(0),
        };
        let p = MockProvider::scripted([reply(PATH, 0.3)]);
        let out = autotune(&p, &exec, cfg(1), |_| {}, Arc::new(AtomicBool::new(false)))
            .await
            .unwrap();
        assert!(!out.iterations[0].accepted);
        assert!((out.best_params[PATH] - 0.2).abs() < 1e-9);
        assert_eq!(out.best_score, out.baseline_score);
        assert!(out.audit.iter().any(|e| e.kind == AuditKind::Rollback));
        assert_eq!(out.stop_reason, StopReason::MaxIterations);
    }

    #[tokio::test]
    async fn cancel_stops_the_loop() {
        let exec = Fake {
            runs: AtomicUsize::new(0),
        };
        let p = MockProvider::scripted((0..8).map(|_| reply(PATH, 0.12)));
        let cancel = Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        let out = autotune(
            &p,
            &exec,
            cfg(10),
            move |pr| {
                if pr.iteration >= 1 {
                    c2.store(true, Ordering::SeqCst);
                }
            },
            cancel,
        )
        .await
        .unwrap();
        assert_eq!(out.stop_reason, StopReason::Cancelled);
        assert!(out.iterations.is_empty());
        assert_eq!(exec.runs.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn audit_has_no_key_and_serializes() {
        let exec = Fake {
            runs: AtomicUsize::new(0),
        };
        let p = MockProvider::scripted([reply("controller.evil", 1.0), reply(PATH, 0.17)]);
        let out = autotune(&p, &exec, cfg(2), |_| {}, Arc::new(AtomicBool::new(false)))
            .await
            .unwrap();
        let s = serde_json::to_string(&out.audit).unwrap();
        assert!(s.contains("not_allowlisted"));
        assert!(!s.contains("Bearer"));
    }
}
