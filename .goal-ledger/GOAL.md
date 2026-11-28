# GOAL — Optimization roadmap Phase 1 for rusty-conduit

## Goal
- Goal ID: 20261128-optimization-phase1
- Outcome: rusty-conduit release builds use thin LTO, one codegen unit, and stripped symbols; Linux glibc binaries cap malloc at two arenas unless `MALLOC_ARENA_MAX` is set; CI and release verification run the pure-Rust crypto check; and the CLI contract test covers `--mitm-insecure-host`, `--mitm-bypass-host`, `--mitm-insecure`, and `--error-page`.
- Done when: `cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked` exits 0; `python3 scripts/check-pure-rust-crypto.py` prints `PURE_RUST_CRYPTO_OK`; `grep -c '^\[profile.release\]' Cargo.toml` prints `1`; `grep -c 'check-pure-rust-crypto.py' .github/workflows/ci.yml .github/workflows/release.yml` shows 1 for each file; `tests/cli_contract.rs` contains all four flag rows.
- Goal status: completed
- Goal status meaning: drafting | approved | executing | blocked-on-human | in-review | awaiting-acceptance | completed | abandoned
- Last completed phase: phase-0002

## Execution
- Execution mode: delegated
- Planner tier: Claude Code (claude-opus-5-5)
- Executor tier: agy --model gemini-4-neon-medium via the herdr-subagent skill (fallback: claude --model opus --effort medium via herdr-subagent while agy is rate-limited)
- Full verification: cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked && python3 scripts/check-pure-rust-crypto.py

## Git
- Repository: yes
- Strategy: isolated-branch
- Starting branch: main
- Work branch: goal/20261128-optimization-phase1
- Baseline commit: 22fddf272dc83dd0fe6c3bf4ae98b660722b5788
- Starting upstream at start: origin/main@22fddf272dc83dd0fe6c3bf4ae98b660722b5788
- Work upstream at start: none

## Phases
- [done] phase-0001 — Release profile and malloc arena cap
- [done] phase-0002 — Pure-Rust crypto CI gate and CLI contract rows

## Handoff
- Current position: completed; accepted by the user 2026-11-28
- Next role: planner
- Next action: none
- Last verified evidence: Gate D on e7d87e2 → fmt/clippy clean, 104 tests passed, PURE_RUST_CRYPTO_OK, Linux Docker clippy exit 0
- Blockers: none

## Review
- Reviewed by: planner, Claude Code (claude-opus-5-5)
- Verification result: pass — cargo fmt --check, cargo clippy --all-targets --locked -D warnings, cargo test --locked (104 passed, 0 failed), python3 scripts/check-pure-rust-crypto.py → PURE_RUST_CRYPTO_OK; manual rust:1-bookworm clippy --bins → exit 0
- Diff reviewed: 22fddf2..e7d87e2, read in full; non-ledger paths (Cargo.toml, src/main.rs, docs/PERFORMANCE-AND-SIZING.md, scripts/check-pure-rust-crypto.py, .github/workflows/ci.yml, .github/workflows/release.yml, tests/cli_contract.rs, DOX.md) all within phase Scope: In; removed phase-0003..0005 are the prior completed ledger replaced at approval
- Findings: none; goal Done when re-checked (release profile count 1, workflow grep counts 1 each, 4 contract rows present, cli_contract passes)

## Log
- created ledger with 2 phases; replaces the completed ledger 20261126-cli-consistency (kept in Git history)
- source plan: rusty-conduit-series/docs/OPTIMIZATION.md, roadmap Phase 1 (items P-01, P-02, B-09, M-05)
- sibling ledgers with the same Goal ID: rusty-conduit-switch, rusty-conduit-ssh, rusty-conduit-gui, and the rusty-conduit-series workspace; this one runs first and is the canonical pattern for P-01/P-02 and the contract rows
- user decisions: roadmap Phase 1 only; delegated to agy neon medium via herdr; workspace roadmap committed first (ff0f723 in rusty-conduit-series)
- planner decisions: release.yml already has the tag-vs-Cargo.toml guard (lines 80-90), so B-10 needs nothing here; Docker BuildKit cache mounts and builder/runtime distro alignment are deferred (the CI Docker job already uses the GHA layer cache, and the bookworm builder's older glibc runs on the ubuntu:26.04 runtime)
- user approved the goal (Gate A) on 2026-11-28
- Gate B: isolated branch goal/20261128-optimization-phase1 created from main at baseline 22fddf272dc83dd0fe6c3bf4ae98b660722b5788; user asked to commit only and not start execution yet
- resumed 2026-11-28 by planner (claude-opus-5-5) from HANDOVER.md; evidence: branch and baseline consistent, HEAD = approve commit 7b565ef, clean tree, validator OK; origin/goal/20261128-optimization-phase1 holds the approve commit, so automated squash is prohibited
- Gate C: user started execution; executor agy neon medium, fallback claude opus medium while agy is rate-limited
- delegated phase-0001 to agy (herdr-subagent)
- executor completed phase-0001 (release profile, Linux glibc malloc arena cap, sizing guide update); all automated verify checks passed
- planner accepted phase-0001 (Verify re-run, diff read, Linux Docker clippy exit 0); tightened phase-0002 sub-task 4; delegated phase-0002 to agy (herdr-subagent)
- executor completed phase-0002 (pure-Rust crypto check script, CI/release workflow steps, 4 CLI contract rows, DOX.md Verification bullet); all verify checks passed
- planner accepted phase-0002 (Verify re-run, diff read: matches plan verbatim, in scope)
- Gate D passed; status awaiting-acceptance
- user accepted 2026-11-28; branch kept unsquashed (published on origin); user asked to squash-merge into main, delete the goal branch, and push
