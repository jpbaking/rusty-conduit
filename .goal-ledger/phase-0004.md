# phase-0004 — CLI family contract test

- Status: done
- Owner: executor
- Depends on: phase-0001, phase-0002, phase-0003
- Goal: Add `tests/cli_contract.rs`, adapted from rusty-conduit-switch's, which fails if a shared flag's name, value name, env var, help text, or default drifts from the family contract.
- Done when: `cargo test --locked --test cli_contract` passes 3 tests.
- Pattern to follow: `../rusty-conduit-switch/tests/cli_contract.rs` (read only: copy it, then apply the substitutions in sub-task 1)

## Context
All three tools carry the same contract table; here only the crate name, env prefix, and binary name differ, and every help string is identical to the switch's table. The table is the specification: if a row fails, an earlier phase missed a doc comment, which is an escalation, not something to fix by editing the table. The planner already dry-ran this adapted test against a copy of this repo with phase-0001's changes applied: all 3 tests passed. The switch test's last test also checks deprecated switch aliases, which do not exist here; remove those two assertions.

## Scope
- In: `tests/cli_contract.rs` (new)
- Out: `src/`, every other file under `tests/`, `docs/`, `README.md`, `DOX.md`

## Sub-tasks
1. [done] Copy `../rusty-conduit-switch/tests/cli_contract.rs` to `tests/cli_contract.rs` and make exactly these edits: `rusty_conduit_switch::config` → `rusty_conduit::config`; `RUSTY_CONDUIT_SWITCH_` → `RUSTY_CONDUIT_` (every occurrence); `CARGO_BIN_EXE_rusty-conduit-switch` → `CARGO_BIN_EXE_rusty-conduit`; delete the two `deprecated alias must stay hidden` assertions in `binary_help_and_exit_codes` — done when: `cargo test --locked --test cli_contract` compiles.
2. [done] Run `cargo fmt` and `cargo test --locked --test cli_contract`; record each failing assertion in `## Log` — done when: all 3 tests pass.

## Verify
```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked --test cli_contract
cargo test --locked
! grep -nE 'SWITCH|rusty_conduit_switch|rusty-conduit-switch"' tests/cli_contract.rs
```

## Escalate when
- `../rusty-conduit-switch/tests/cli_contract.rs` does not exist → stop, set Status: needs-human — reason: the rusty-conduit-switch goal has not finished its phase-0004.
- Any `SHARED_FLAGS` row fails after the substitutions → stop, set Status: needs-human — reason: paste the assertion message (the fix belongs in `src/config.rs`, outside this phase).

## Log
- (append-only, one line per event)
- sub-task 1: copied and adapted `../rusty-conduit-switch/tests/cli_contract.rs` to `tests/cli_contract.rs`
- sub-task 2: ran `cargo fmt` and `cargo test --locked --test cli_contract` (0 failing assertions; all 3 passed) and verified full `## Verify` suite passes

## Evidence
- (append-only: command → observed result)
- `cargo test --locked --test cli_contract --no-run` → exit 0
- `cargo fmt && cargo test --locked --test cli_contract` → exit 0 (3 passed, 0 failed)
- `cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked && ! grep -nE 'SWITCH|rusty_conduit_switch|rusty-conduit-switch"' tests/cli_contract.rs` → exit 0 (42 unit + 40 integration tests passed, 0 failed)
