# GOAL — Align rusty-conduit with the rusty-conduit CLI family

## Goal
- Goal ID: 20261126-cli-consistency
- Outcome: rusty-conduit uses the family's help wording and MITM cache default, gains a `validate` subcommand, raises its open-file limit at startup, and locks the conventions with a CLI contract test.
- Done when: `cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked` exits 0; `rusty-conduit --help` shows `SOCKS4/4a/5` everywhere and `[default: 10240]` for the MITM cache; `rusty-conduit validate` prints `config OK: none` and exits 0; `tests/cli_contract.rs` passes; docs describe the changes.
- Goal status: completed
- Goal status meaning: drafting | approved | executing | blocked-on-human | in-review | awaiting-acceptance | completed | abandoned
- Last completed phase: phase-0005

## Execution
- Execution mode: delegated
- Planner tier: Claude Code (claude-opus-5-5)
- Executor tier: agy --model gemini-4-neon-medium via the herdr-subagent skill
- Full verification: cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked

## Git
- Repository: yes
- Strategy: isolated-branch
- Starting branch: main
- Work branch: goal/20261126-cli-consistency
- Baseline commit: 344ad3b45f28ff8bbaf98550fef949890793d0e9
- Starting upstream at start: origin/main@344ad3b45f28ff8bbaf98550fef949890793d0e9
- Work upstream at start: none

## Phases
- [done] phase-0001 — Align help wording and the MITM cache default
- [done] phase-0002 — Add the validate subcommand
- [done] phase-0003 — Raise the open-file limit at startup
- [done] phase-0004 — CLI family contract test
- [done] phase-0005 — Update user docs and DOX Feature Map

## Handoff
- Current position: completed
- Next role: planner
- Next action: none
- Last verified evidence: Gate D: fmt, clippy -D warnings, cargo test --locked (82 passed); doc greps pass; advanced-guide TOML example validates; src/fdlimit.rs identical to rusty-conduit-switch's
- Blockers: none

## Review
- Reviewed by: Claude Code (claude-opus-5-5), planner
- Verification result: pass — cargo fmt --check exit 0; cargo clippy --all-targets --locked -- -D warnings exit 0; cargo test --locked 82 passed, 0 failed
- Diff reviewed: 344ad3b45f28ff8bbaf98550fef949890793d0e9..HEAD read in full (per-phase diffs); every changed path is inside a phase Scope: In or the ledger (tests/socks_proxy.rs via the phase-0001 escalation answer); all commits carry Goal-ID; no merges
- Findings: one pre-existing issue, fixed — tests/socks_proxy.rs failed cargo fmt --check with current stable rustfmt (formatted in phase-0001 sub-task 4); stale docs/comments were folded into phases 0001 and 0005 before execution

## Log
- created ledger with 5 phases
- sibling ledgers with the same Goal ID: rusty-conduit-switch and rusty-conduit-ssh (both run before this one)
- user decisions: one ledger per repo; goal/<id> branch per repo; defaults max_connections 4096 and MITM cache 10240 in all three CLIs; add an open-file limit raise; listen-backlog/somaxconn work is a separate follow-up goal in rusty-conduit-engine
- planner dry run: the contract test from rusty-conduit-switch phase-0004, adapted to this crate, passes against a copy of this repo once phase-0001's two changes are applied
- user approved goal, branches, and execution (Gates A-C); baseline 344ad3b45f28ff8bbaf98550fef949890793d0e9 on main; isolated branch goal/20261126-cli-consistency
- planner amended todo phases before execution: phase-0001 sub-task 3 (stale filter comments) and phase-0005 sub-task 6 (stale --socks-block-web-in-mitm and allow/block docs)
- execution started after rusty-conduit-switch and rusty-conduit-ssh passed Gate D; phase-0001 delegated to agy (gemini-4-neon-medium) in herdr pane sub-rc
- phase-0001 blocked (needs-human): all 3 sub-tasks in src/config.rs and all other Verify checks passed, but `cargo fmt --check` failed on pre-existing formatting diffs in `tests/socks_proxy.rs:654,666` (`Scope: Out`)
- planner answered phase-0001 escalation: pre-existing rustfmt drift in tests/socks_proxy.rs is formatted in this phase (sub-task 4)
- phase-0001 done: formatted tests/socks_proxy.rs (sub-task 4) and verified full phase-0001 checks pass
- planner accepted phase-0001; phase-0002 delegated
- phase-0002 done: added `validate` subcommand (`src/config.rs`, `src/lib.rs`) and `tests/validate_cmd.rs` (4 tests); all Verify checks pass
- planner accepted phase-0002; phase-0003 delegated
- phase-0003 done: added `rlimit = "0.12"`, copied `src/fdlimit.rs` verbatim, wired `fdlimit::apply` in `src/lib.rs`, verified manual ulimit raise and full Verify suite
- planner accepted phase-0003; phase-0004 delegated
- phase-0004 done: added `tests/cli_contract.rs` (3 tests pass) and verified full Verify suite passes
- planner accepted phase-0004; phase-0005 delegated
- phase-0005 done: updated README.md, USER-GUIDE-BASIC.md, USER-GUIDE-ADVANCED.md, PERFORMANCE-AND-SIZING.md, and DOX.md Feature Map; all Verify checks pass
- planner accepted phase-0005; Gate D passed; goal awaiting user acceptance
- user accepted the result ("proceed how you see best"); squash checks passed; squashed onto main as one commit with `git merge --squash`; goal/20261126-cli-consistency kept with the full phase history; nothing pushed
