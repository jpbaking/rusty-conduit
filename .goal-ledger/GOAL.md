# GOAL — Optimization roadmap Phase 3b for rusty-conduit

## Goal
- Goal ID: 20261128-optimization-phase3b
- Outcome: rusty-conduit builds against the current untagged engine (`rev` pin) and drops its own copies of the open-file limit, startup preflight, shutdown signal, version string, unit-test fixtures, and `[mitm]` / `[logging]` file-section merge in favor of the engine's `cli-support` / `test-support` features, with CLI behavior and output unchanged except the recorded M-18 differences.
- Done when: `cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked && python3 scripts/check-pure-rust-crypto.py` exits 0; `src/fdlimit.rs` and `src/preflight.rs` no longer exist; `src/test_support.rs` only re-exports the engine's fixtures; `Cargo.toml` pins the engine by `rev = "05485a4c0fdd5315e4e6954a199a55d34cd17bc1"`; every phase `## Verify` passes.
- Goal status: completed
- Goal status meaning: drafting | approved | executing | blocked-on-human | in-review | awaiting-acceptance | completed | abandoned
- Last completed phase: phase-0003

## Execution
- Execution mode: delegated
- Planner tier: Claude Code (Fable 5)
- Executor tier: agy --model gemini-4-neon-medium via the herdr-subagent skill (fallbacks in order: claude --model opus --effort medium; opencode remote-lm/ornith-1.5-35b-a3b-apex-mtp with --standalone)
- Full verification: cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked && python3 scripts/check-pure-rust-crypto.py

## Git
- Repository: yes
- Strategy: isolated-branch
- Starting branch: main
- Work branch: goal/20261128-optimization-phase3b
- Baseline commit: fcc6cd4bfdb56b79cdbc91814443f6a4f4aab76c
- Starting upstream at start: origin/main@fcc6cd4bfdb56b79cdbc91814443f6a4f4aab76c
- Work upstream at start: none

## Phases
- [done] phase-0001 — Pin the engine by rev with the cli-support and test-support features
- [done] phase-0002 — Use the engine's fdlimit, preflight, shutdown_signal, app_version!, and test fixtures
- [done] phase-0003 — Use the engine's shared [mitm] and [logging] file sections

## Handoff
- Current position: completed; accepted by the user 2026-11-28; squashed into one commit and merged to main
- Next role: planner
- Next action: none (re-pin to the engine's v0.7.0 tag at release)
- Last verified evidence: Gate D → fmt/clippy clean, 98 passed, PURE_RUST_CRYPTO_OK
- Blockers: none

## Review
- Reviewed by: planner, Claude Code (Fable 5)
- Verification result: pass — the Full verification command exits 0 (98 tests: 104 before, minus the 8 fdlimit/preflight unit tests that moved to the engine, plus 2 new config tests)
- Diff reviewed: fcc6cd4..f373922, read in full; 9 non-ledger paths, all within phase Scope: In; 9 Goal-ID commits, no merges; goal Done-when holds (rev pin ×2, fdlimit.rs and preflight.rs gone, test_support.rs is a 2-line re-export)
- Findings: none open; phase-0003 kept the former file-error order by running the logging merge after the keepalive keys; the only behavior differences are M-18's (deprecated `mitm.cert_cache_max` accepted with a notice; passphrase redacted in `Debug`)

## Log
- created ledger with 3 phases; replaces the completed ledger 20261128-optimization-phase1 (kept in Git history)
- source plan: rusty-conduit-series/docs/OPTIMIZATION.md roadmap Phase 3b (M-01 consumer side) for rusty-conduit; this repo is the template for rusty-conduit-ssh and rusty-conduit-switch
- user decisions (2026-11-28): no engine tag until every optimization is done, so consumers pin `rev = "05485a4c0fdd5315e4e6954a199a55d34cd17bc1"` (engine main after P-09) and re-pin to `tag = "v0.7.0"` at release; rusty-conduit is drafted first
- planner evidence: a scratch build with only the pin changed compiled cleanly, passed clippy, and passed all 104 tests, so the engine bump itself needs no code change
- planner decisions: the public surface stays (`rusty_conduit::{fdlimit, preflight, shutdown_signal}` become re-exports, `config::app_version()` keeps its signature and calls the engine macro, which expands here); integration tests in `tests/` keep their own harnesses (project contract: each test file carries its own); the workspace's `scripts/check-workspace.py` learns `rev` pins in a separate workspace change
- user approved the goal and said begin (Gates A-C) on 2026-11-28; Gate B: branch goal/20261128-optimization-phase3b created from main at baseline fcc6cd4bfdb56b79cdbc91814443f6a4f4aab76c
- delegated phase-0001 to agy (herdr-subagent)
- phase-0001 done: pinned rusty-conduit-engine by rev 05485a4c0fdd5315e4e6954a199a55d34cd17bc1 with cli-support/test-support features, updated Cargo.lock and DOX.md, all verify checks passed (104 tests, PURE_RUST_CRYPTO_OK)
- planner accepted phase-0001; delegated phase-0002 to agy (herdr-subagent)
- phase-0002 done: deleted src/fdlimit.rs and src/preflight.rs, re-exported engine cli_support::{fdlimit, preflight, shutdown_signal}, switched app_version() to engine macro, reduced src/test_support.rs to 2-line engine re-export, removed rlimit from Cargo.toml/Cargo.lock, updated DOX.md; all Verify checks passed (96 tests, PURE_RUST_CRYPTO_OK)
- planner accepted phase-0002; began phase-0003
- phase-0003 done (planner): engine file sections, deprecation notice via Config::notices; status in-review
- Gate D passed; status awaiting-acceptance
- user accepted 2026-11-28 and chose squash; all squash checks passed (clean tree, on the work branch, 10 Goal-ID commits, no merges, nothing on origin, origin/main at the baseline); squashed, merged to main, pushed
