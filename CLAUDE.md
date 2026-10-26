# CLAUDE.md — Standing instructions for Claude (backend lead) in this repo

## Project
**MazeMuse** — localhost robot simulator for RoboCup Junior Rescue Maze **2027** rules, built for
the Meta Global AI Developer Hackathon (Oct 26 – Nov 4, 2026). Rust + Axum + Tokio backend
(`/backend`, yours), React + TS frontend (`/frontend`, owned by the Kimi agent — do not modify),
integrated through **`docs/API.md`, which you own**. Muse (Meta Model API) powers the
suggest / auto-tune loop. Full build order: see the original PROMPT2 content in repo history /
`docs/PROMPT2.md` if present.

## Layout
```
/backend/crates/{mm-core,mm-maze,mm-sim,mm-controller,mm-tuner,mm-server,mm-cli}
/backend/rules_profiles/rcj_maze_2027_draft.json     # all rule numbers live here, never in code
/backend/docs/  (API.md, REPLAY_FORMAT.md, CONTROLLER_CONVERSION_PROMPT.md, TUNER_GUARDRAILS.md)
/backend/fixtures/                                     # mock data packs for the frontend
/frontend/                                             # Kimi's — read-only for you
/CLAUDE.md /AGENTS.md /SKILLS.md                       # governance (this file)
```
Dependency rule: `mm-core ← {maze,sim,controller,tuner} ← {server,cli}`. `mm-core` has no I/O.

## Commands
```bash
cd backend
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace            # must pass before any merge/commit claim
cargo run -p mm-server            # :8080  (env: META_MODEL_API_KEY, MUSE_BASE_URL, MUSE_MODEL)
cargo run -p mm-cli -- demo       # end-to-end: generate → run → fail → tune → improve
cargo bench -p mm-sim             # perf gates: ≥10× realtime headless
```

## Invariants (violating these = bug, not style)
1. **Determinism:** fixed timestep; same seed ⇒ byte-identical replay. No wall-clock, no
   unordered-map iteration, no global RNG inside `mm-sim`.
2. **RNG policy:** OS TRNG (getrandom) seeds runs only; all in-run randomness flows from the
   seeded CSPRNG stream; seed recorded in replay header. `rng.mode: trng | fixed`.
3. **Rules as data:** 2027 field/scoring numbers come from `rules_profiles/*.json`
   (source of truth: https://rescue.rcj.cloud/events/2027/Rules/RCJRescueMaze2027-draft.pdf).
4. **Contract-first:** every API surface change ⇒ update `docs/API.md` (version bump + changelog
   + fixture update) in the SAME commit; notify via `docs/API_CHANGES_PROPOSED.md` process.
5. **Sandbox:** user controller scripts run in Rhai with instruction fuel, timeouts, zero I/O.
6. **Secrets:** keys via env only; redact from logs/traces/audit exports; bind 127.0.0.1;
   CORS allowlist `http://localhost:5173`.
7. **No frontend edits, no simulation math in server layer** (server orchestrates, sim computes).

## Code style
- Edition 2021+, `rustfmt` defaults, clippy `-D warnings` (no blanket allows).
- `thiserror` in library crates, `anyhow` in binaries. No `unwrap()/expect()` on request paths
  (fixtures/tests OK). Serde on every wire type; `schemars` for schema generation.
- Units: meters/radians/seconds internally; cm/deg only at API/UI edges. `f64` in sim,
  `u16` mm + status for ToF zones (datasheet-shaped).
- Tests next to code (`#[cfg(test)]`), golden files under `crates/*/tests/fixtures/`.
  Property tests (proptest) for maze generator + scoring table.
- Commits: `feat(sim): …`, `fix(maze): …`, `docs(api): …`, `test(tuner): …` — small, green, scoped.
- Every milestone ends with: fmt+clippy+tests green, API.md current, fixtures regenerated.

## Working agreement
- You may spawn subagents (see PROMPT2 §12). You personally own `mm-core`, `docs/API.md`,
  integration reviews, and the M1–M6 milestone gates. Merge nothing that fails CI locally.
- If requirements conflict with an invariant above, STOP and surface the conflict in
  `docs/DECISIONS.md` with options — don't silently pick.
- Definition of done for any feature: code + tests + docs/API.md entry (if surfaced) + fixture
  (if frontend-visible) + one-line demo command in `backend/README.md`.

## Hackathon clock
Milestones M1–M6 over days 2–7, freeze day 8. Demo pack (`mm-cli demo` + seeded fixtures) is a
graded deliverable. When in doubt between polish and a working end-to-end path: **end-to-end wins**.
