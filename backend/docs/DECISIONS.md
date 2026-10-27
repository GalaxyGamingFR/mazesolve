# Decisions

## D1 (2026-10-27): scoring follows the 2027 draft PDF, not SUMMARY.md
Read from the draft (last updated 2026-09-08), §5.6:
- Victims: linear smiley 5, linear cognitive 10, floating smiley 15, floating cognitive 30.
  There is **no separate Dangerous Zone tier** (SUMMARY.md implied one).
- Kits: 1 kit = 10, 2 kits on one victim = 30; harmed takes 2, stable 1, unharmed 0.
- Blue tile: 30 for a single visit, -10 per revisit, floor 0.
- Reliability bonus = SVI*10 + SRD*10 + SBV*10 - LoP*15, floor 0.
- Exit bonus = SVI*10 + SBV*10 + SSN*5 + SRN*5, needs light on and still for 5 s on start.
- Misidentification -5 (more than 15 cm from any victim). Total never below 0.
All of these numbers live in `rules_profiles/rcj_maze_2027_draft.json`.

## D2: Lack of Progress is auto-called
The rules leave LoP to the team captain (or trigger it on a black tile or an early exit from a blue tile).
The sim auto-calls it after `play.lop_no_progress_s` (30 s) without 5 cm of net movement,
and on a black-tile entry or a blue-tile early exit. It can be switched off per run.

## D3: seeds are below 2^53
`SEED_MASK` keeps seeds JSON-number-safe for the JavaScript frontend.
