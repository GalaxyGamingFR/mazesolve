# Tuner guardrails

Applies to `mm-tuner` Mode A (`suggest`) and Mode B (`autotune`). Muse proposes; the
guardrails decide what is allowed to touch a run.

## Allowlist
Mode B only changes parameters in `AutoTuneConfig.params_allowlist` (`TunableParam`:
`controller.<name>` or `error_profile.<section>.<field>`). A suggestion naming any other
path is dropped and logged as `GuardrailDrop` (`reason: not_allowlisted`). The dossier
shows Muse the same table (current value, min, max, max_step), and the prompt tells it to
stay inside it, but the code never relies on that.

## Clamps and step limits
For a parameter with current value `c`, bounds `[min, max]` and `max_step`:
`applied = clamp(requested, max(min, c - max_step), min(max, c + max_step))`.
If `applied != requested`, a `GuardrailClamp` entry records requested, applied, bounds and
current. NaN or infinite values are dropped (`non_finite`). A change that ends at the
current value is dropped (`no_change`). Two suggestions for one path in one iteration:
the first wins, the rest are dropped (`duplicate_path`). Current value defaults to the
parameter's `default` when the run has no override.

## Objective
Each candidate is run on `baseline_seed` plus every `fresh_seeds` entry. Objective is the
mean `score.total` across those runs. Tie-break (scores within 1e-9): fewer total LoP
restarts, then fewer total collisions. The dossier is always built from the baseline-seed
run of the currently accepted parameters.

## Accept and rollback
A candidate is accepted only if its objective beats the current one by at least
`min_improvement` (or ties and wins the tie-break). Otherwise the parameters roll back to
the previous accepted set, and an `AuditKind::Rollback` entry is written. The returned
`best_params` are always an accepted set, never an untested one.

## Stop conditions
`max_iterations`; wall-clock `budget` (checked before every iteration and every run);
cancel (`Arc<AtomicBool>`, checked before every iteration and every run); two consecutive
iterations with no valid param suggestion (including unparseable replies after the one
repair retry). The reason is returned as `StopReason`. A cancel or budget stop mid
evaluation discards the partial candidate. Executor or provider errors other than parse
errors abort with `Err`.

## Code changes
Muse may return `code` targets (unified diffs). v1 never applies them, whatever
`allow_code_changes` says: each is recorded as an `AuditKind::Delta` with
`status: "pending_review"` and returned in `IterationRecord.pending_code` for a human to
review. `allow_code_changes` is reserved for a later version.

## Cost accounting
`Usage { prompt_tokens, completion_tokens }` is summed from every provider response
(including repair retries) and returned in `AutoTuneOutcome.usage` / `SuggestOutcome.usage`.
Per iteration cost is one Muse call (two if repaired) plus `1 + fresh_seeds.len()` sim runs.
The dossier is capped at 30k estimated tokens (chars / 4) and truncated with a marker.
Provider retries (429, 5xx, timeouts; 4 tries, exponential backoff with jitter,
`retry-after` honoured, capped at 60 s) are not double counted.

## Redaction
The API key is read from `META_MODEL_API_KEY`, held only in `MuseConfig`, sent only in the
`Authorization` header, and never placed in prompts, errors or audit entries.
`Debug` for `MuseConfig` / `MuseProvider` prints `<redacted>`. Upstream error bodies are
truncated to 300 chars. The audit stores prompts verbatim, so do not put secrets in
controller code or robot summaries.

## Audit format
`Vec<AuditEntry>` (serde + JsonSchema): `{ iteration, kind, payload, t_ms }`, iteration 0
being the baseline evaluation and Mode A. Kinds and payloads:

| kind | payload |
|---|---|
| Prompt | `{system, user}` |
| Response | `{content}` (raw model text) |
| Delta | `{kind:"param", path, from, to}` or `{kind:"code", title, diff, status}` |
| GuardrailClamp | `{path, requested, applied, min, max, max_step, current}` |
| GuardrailDrop | `{path, requested, reason}` or `{reason:"unparseable_response", error}` |
| RunResult | `{run_id, seed, score, restarts, collisions, params}` |
| Accept / Rollback | `{before, after, restarts_*, collisions_*}` |

`t_ms` is milliseconds since the audit log was created.
