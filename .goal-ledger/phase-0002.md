# phase-0002 — Use the engine's fdlimit, preflight, shutdown_signal, app_version!, and test fixtures

- Status: done
- Owner: executor
- Depends on: phase-0001
- Goal: Delete rusty-conduit's copies of code the engine now provides, keeping the crate's public names as re-exports so callers and tests are unchanged.
- Done when: every command in `## Verify` exits 0.
- Pattern to follow: `src/lib.rs:14` (`pub use rusty_conduit_engine::{...};` — the existing re-export style)

## Context
The engine's `cli_support::{fdlimit, preflight}` modules, `cli_support::shutdown_signal`, and `test_support` module were copied verbatim from this crate (engine commit `55330cd`; only `use` paths differ), so behavior cannot change. `app_version!` is a macro because it must read `CONDUIT_RELEASE_VERSION` / `CARGO_PKG_VERSION` of this crate: `config::app_version()` keeps its signature and calls it. The unit tests of the deleted files now live in the engine. `rlimit` was used only by `src/fdlimit.rs`; `libc` is still used by `src/main.rs`.

## Scope
- In: `src/lib.rs`, `src/config.rs` (the `app_version` body only), `src/fdlimit.rs` (delete), `src/preflight.rs` (delete), `src/test_support.rs`, `Cargo.toml` (remove the `rlimit` line only), `Cargo.lock`, `DOX.md` (the three bullets named in sub-task 6)
- Out: `tests/`, `src/main.rs`, every other part of `src/config.rs`, `.github/`, `docs/`

## Sub-tasks
1. [done] `git rm src/fdlimit.rs src/preflight.rs`; in `src/lib.rs` replace the two lines `pub mod fdlimit;` and `pub mod preflight;` with the single line `pub use rusty_conduit_engine::cli_support::{fdlimit, preflight};` — done when: `cargo build --locked` succeeds and `test ! -e src/fdlimit.rs && test ! -e src/preflight.rs` exits 0.
2. [done] In `src/lib.rs`, delete the `shutdown_signal` function together with its three doc-comment lines (currently lines 241-243, starting `/// Resolve when the process receives ...`), through the function's closing `}`, and add `cli_support::shutdown_signal` to a re-export: write the line `pub use rusty_conduit_engine::cli_support::shutdown_signal;` directly below the line added in sub-task 1. Remove any `use` that becomes unused (clippy names it) — done when: `grep -c 'fn shutdown_signal' src/lib.rs` prints `0` and `cargo clippy --all-targets --locked -- -D warnings` passes.
3. [done] In `src/config.rs`, replace the body of `pub fn app_version() -> &'static str` (the `match option_env!(...) { ... }` block) with `rusty_conduit_engine::app_version!()`, keeping the doc comment and signature — done when: `cargo test --locked app_version_reports_release_or_dev` and `cargo test --locked --test cli_contract` pass.
4. [done] Replace the whole content of `src/test_support.rs` with exactly these two lines: `//! Unit-test fixtures, from the engine's \`test-support\` feature.` and `pub(crate) use rusty_conduit_engine::test_support::*;` (`src/lib.rs` keeps `#[cfg(test)] mod test_support;`) — done when: `cargo test --locked --lib` passes.
5. [done] In `Cargo.toml`, delete the line `rlimit = "0.12"` under `[target.'cfg(unix)'.dependencies]` (keep `libc = "0.2"`), then run `cargo build` once without `--locked` to update `Cargo.lock` — done when: `cargo build --locked` succeeds and `grep -c '^rlimit' Cargo.toml` prints `0`.
6. [done] In `DOX.md`: in the **Tests are hermetic** bullet (line 226), replace `` `src/test_support.rs` holds helpers for the crate's unit tests only (`#[cfg(test)]`). `` with `` `src/test_support.rs` re-exports the engine's `test-support` fixtures for the crate's unit tests only (`#[cfg(test)]`). ``; in the **Open-file limit** bullet (line 250), replace ``Start: `src/fdlimit.rs`. Files: `src/lib.rs` (`run`), `Cargo.toml` (`rlimit`), `docs/PERFORMANCE-AND-SIZING.md`.`` with ``Start: `src/lib.rs` (`run`, via the engine's `cli_support::fdlimit`). Files: `docs/PERFORMANCE-AND-SIZING.md`.``; in the **`validate` subcommand** bullet (line 249), replace ``Files: `src/preflight.rs`. Files:`` with ``Files: the engine's `cli_support::preflight`,``; in the **Unit-test helpers** bullet (line 263), replace ``— log capture, echo/origin servers, test CA, TLS client for the crate's unit tests.`` with ``— the engine's `test-support` fixtures (log capture, echo/origin servers, test CA, TLS client), re-exported for the crate's unit tests.`` — done when: `grep -c 'src/fdlimit.rs\|src/preflight.rs' DOX.md` prints `0`.
7. [done] Fix-up: reorder the two `cli_support` `pub use` lines in `src/lib.rs` to match `cargo fmt` — done when: `cargo fmt --check` exits 0.

## Verify
```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
python3 scripts/check-pure-rust-crypto.py
test ! -e src/fdlimit.rs && test ! -e src/preflight.rs
test "$(wc -l < src/test_support.rs | tr -d ' ')" = 2
test "$(grep -c 'fn shutdown_signal' src/lib.rs)" = 0
git diff --exit-code -- tests src/main.rs .github docs
```

## Escalate when
- A unit test fails after switching `test_support` (a fixture differs from the old copy) → stop, set Status: needs-human — reason: name the test and paste the failure.
- Something outside the deleted code calls a name the engine does not export (for example a `pub(crate)` helper) → stop, set Status: needs-human — reason: name it and its call site.
- Removing `rlimit` breaks the build → stop, set Status: needs-human — reason: paste the error.

## Log
- (append-only, one line per event)
- sub-task 1: removed src/fdlimit.rs and src/preflight.rs and re-exported cli_support::{fdlimit, preflight} in src/lib.rs
- sub-task 2: deleted shutdown_signal from src/lib.rs and re-exported cli_support::shutdown_signal
- sub-task 3: replaced app_version() body in src/config.rs with rusty_conduit_engine::app_version!()
- sub-task 4: replaced src/test_support.rs with the 2-line re-export of rusty_conduit_engine::test_support::*
- sub-task 5: removed rlimit = "0.12" from Cargo.toml and updated Cargo.lock via cargo build
- sub-task 6: updated the four DOX.md bullets for test_support, validate/preflight, and fdlimit
- verify attempt 1: cargo fmt --check failed on cli_support re-export order in src/lib.rs; added fix-up sub-task 7
- sub-task 7: reordered cli_support pub use lines in src/lib.rs; full phase Verify passed

- planner accepted phase-0002: diff matches the sub-tasks; Verify re-run (96 passed — the 8 fewer are the 4 + 4 unit tests of the deleted fdlimit.rs / preflight.rs, which run in the engine now; clippy clean; PURE_RUST_CRYPTO_OK)

## Evidence
- (append-only: command → observed result)
- `cargo build --locked && test ! -e src/fdlimit.rs && test ! -e src/preflight.rs` → exit 0 (`Finished dev profile in 0.71s`)
- `test "$(grep -c 'fn shutdown_signal' src/lib.rs)" = 0 && cargo clippy --all-targets --locked -- -D warnings` → exit 0 (`count=0`, `Finished dev profile in 0.36s`)
- `cargo test --locked app_version_reports_release_or_dev && cargo test --locked --test cli_contract` → exit 0 (`1 passed` unit test, `3 passed` cli_contract tests)
- `cargo test --locked --lib` → exit 0 (`54 passed; 0 failed`)
- `cargo build && cargo build --locked && test "$(grep -c '^rlimit' Cargo.toml)" = 0` → exit 0 (`rlimit_count=0`, `Cargo.lock` updated)
- `test "$(grep -c 'src/fdlimit.rs\|src/preflight.rs' DOX.md)" = 0` → exit 0 (`dox_grep=0`)
- `cargo fmt --check` → exit 1 (diff in `src/lib.rs`: `shutdown_signal` before `{fdlimit, preflight}`)
- `cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked && python3 scripts/check-pure-rust-crypto.py && test ! -e src/fdlimit.rs && test ! -e src/preflight.rs && test "$(wc -l < src/test_support.rs | tr -d ' ')" = 2 && test "$(grep -c 'fn shutdown_signal' src/lib.rs)" = 0 && git diff --exit-code -- tests src/main.rs .github docs` → exit 0 (96 tests passed: 54 unit + 42 integration; `PURE_RUST_CRYPTO_OK`; all file/line/diff checks passed)
