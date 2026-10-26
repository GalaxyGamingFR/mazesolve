# AGENTS.md — Guide for any AI agent (or human) working in this repo

MazeMuse is built by a two-agent, two-human team under hackathon time pressure. This file is the
coordination constitution. Read it fully before touching anything. Claude-specific rules live in
`CLAUDE.md`; reusable procedures live in `SKILLS.md`.

## 1. What this repo is
A localhost robot simulator for **RoboCup Junior Rescue Maze 2027** (draft rules:
https://rescue.rcj.cloud/events/2027/Rules/RCJRescueMaze2027-draft.pdf): random rule-accurate
mazes, a robot with 8× VL53L8CX ToF / 1× AS7343 spectral / 2× OpenMV N6 cameras / 4× DDSM210
wheels / BNO055-class IMU, injected sensor & actuator noise, algorithm A/B comparison, and a
**Muse (Meta Model API)** loop that diagnoses failures and auto-tunes. For the Meta Global AI
Developer Hackathon, Oct 26 – Nov 4, 2026.

## 2. Ownership map (who may write what)
| Area | Owner | Others may |
|---|---|---|
| `/backend/**` (Rust workspace) | Claude (backend lead) | read; propose via issues/notes |
| `/frontend/**` (React+TS) | Kimi (frontend lead) | read; propose via issues/notes |
| `docs/API.md` | **Claude only** | read; propose changes (below) |
| `docs/API_CHANGES_PROPOSED.md` | anyone | append proposals |
| `rules_profiles/*.json` | Claude, with human sign-off | propose |
| Repo governance (this file, CLAUDE.md, SKILLS.md) | humans | propose |
| Secrets | **nobody** — env vars only, never committed | — |

## 3. The contract-first protocol (most important section)
1. `docs/API.md` is the **single source of truth** for every REST endpoint, WS message, replay
   field, and error code. Versioned; changelog at the bottom.
2. Frontend implements against API.md (+ its MSW mock layer). Backend implements against API.md
   (+ ships `backend/fixtures/` matching it). Contract tests keep both honest.
3. **Changing the contract:** append a precise proposal (endpoint, schema diff, motivation,
   migration) to `docs/API_CHANGES_PROPOSED.md` → Claude adjudicates → on accept, Claude updates
   API.md (version bump + changelog + fixtures) and pings frontend in the same commit message
   (`NOTIFY-FRONTEND:`). Neither side unilaterally diverges; mocks must be re-diffed after every bump.
4. Additive changes (new optional fields/endpoints) are always allowed; removals/renames need the
   full proposal flow.

## 4. Running things
```bash
# Backend (Rust ≥1.80): 
cd backend && cargo run -p mm-server        # http://127.0.0.1:8080
#   env: META_MODEL_API_KEY, MUSE_BASE_URL (dev.meta.ai per docs), MUSE_MODEL
# Frontend (Node ≥20):
cd frontend && npm i && npm run dev          # http://localhost:5173  (VITE_API_BASE optional)
# Frontend works fully offline in mock mode (MSW) — backend down ≠ frontend broken.
# Quick E2E demo:
cd backend && cargo run -p mm-cli -- demo
```
Ports are fixed (8080/5173). Both processes bind/serve **localhost only** — no tunnels, no cloud.

## 5. Engineering invariants (all agents)
- **Determinism:** same seed ⇒ identical replay. Never introduce wall-clock or nondeterministic
  iteration into simulation state. TRNG is used only to *seed* a run (see CLAUDE.md §Invariants).
- **Units:** meters/radians/seconds on the wire; cm/degrees for display; conversions live in one
  module per side (`frontend/src/lib/units.ts`).
- **Error handling:** every API error is `{ error: { code, message, details? } }`; every UI async
  state has loading/empty/error variants; no dead buttons, no `alert()`.
- **Rules fidelity:** field/scoring numbers come from `rules_profiles/*.json`, traceable to the
  2027 draft PDF. If sim behavior and rules text disagree, rules text wins — file a fix.
- **Sandbox:** imported user code executes only inside the fuel-limited Rhai sandbox.
- **Secrets:** env vars only; `.env*` is gitignored; keys never appear in logs, replays, audit
  exports, screenshots, or the submission video.

## 6. Git & review etiquette
- Branches: `feat/<area>-<slug>`, `fix/<area>-<slug>`, `docs/api-<slug>`. Small PRs, green CI
  (fmt/clippy/tests frontend: tsc/eslint/vitest) before merge.
- Conventional commits (`feat(sim): …`). Commit messages touching the contract carry
  `NOTIFY-FRONTEND:` / `NOTIFY-BACKEND:` tags.
- Each PR description ends with **how to demo it** (one command or click-path).
- Humans merge milestone PRs; agents do not force-push or rewrite shared history.

## 7. Definition of done (repo-wide)
Code + tests + docs. For anything user-visible: fixture + demo path. For anything on the API
boundary: API.md updated. For anything touching scoring/sensors: a golden or property test that
would catch a regression. "It works on my machine" ⇒ add the repro as a test.

## 8. When you're stuck or requirements conflict
Write the conflict into `docs/DECISIONS.md` (context / options / recommendation), pick the
reversible option if the clock forces a choice, and keep the end-to-end path working.
**End-to-end beats polished-but-isolated, every time.** Hackathon clock: freeze on Day 8
(Nov 2–3); Days 6–7 are for the tuner loop + demo recording, not new features.

## 9. Useful pointers
- 2027 rules PDF (field §3, scoring §5.6): rescue.rcj.cloud/events/2027/Rules/RCJRescueMaze2027-draft.pdf
- Meta Model API docs: https://dev.meta.ai/docs (quickstart, models) — OpenAI-compatible dialect
- Prior art to differentiate from (do NOT copy; we are not an official Erebus replacement):
  https://github.com/robocup-junior/erebus
- Datasheets: VL53L8CX (st.com DS14161), AS7343 (ams-osram), OpenMV N6 (openmv.io),
  DDSM210 (waveshare/DFRobot), BNO055 (Bosch BST-BNO055-DS000)
