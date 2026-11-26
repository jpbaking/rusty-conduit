# phase-0003 — Raise the open-file limit at startup

- Status: done
- Owner: executor
- Depends on: phase-0001
- Goal: At startup, raise the soft open-file limit when it cannot hold `--max-connections`, and log a NOTICE when it still cannot.
- Done when: `cargo test --locked` passes and `diff ../rusty-conduit-switch/src/fdlimit.rs src/fdlimit.rs` prints nothing.
- Pattern to follow: `../rusty-conduit-switch/src/fdlimit.rs` (read only: copy it verbatim) and the `fdlimit::apply` call in `../rusty-conduit-switch/src/lib.rs` right after `log_startup`

## Context
The same `fdlimit` module ships in all three tools and must stay byte-identical; rusty-conduit-switch's goal already created it, so copy that file instead of writing a new one. Each proxied connection uses two descriptors, so `--max-connections 4096` needs 8256; `docs/PERFORMANCE-AND-SIZING.md` section 3 already documents that rule, and phase-0005 updates it to say the tool now raises the limit itself. The module never lowers the limit and does nothing when `max_connections` is `0`.

## Scope
- In: `Cargo.toml`, `Cargo.lock`, `src/fdlimit.rs` (new), `src/lib.rs`
- Out: `src/config.rs`, `tests/`, `docs/`, `README.md`, `DOX.md`

## Sub-tasks
1. [done] Add `[target.'cfg(unix)'.dependencies]` with `rlimit = "0.12"` to `Cargo.toml` after the `[dependencies]` section and before `[dev-dependencies]` (line 34); run `cargo build` (without `--locked`) once — done when: `cargo build --locked` succeeds and `git diff --stat Cargo.lock` shows only `rlimit` (and possibly `libc`) added.
2. [done] Copy `../rusty-conduit-switch/src/fdlimit.rs` to `src/fdlimit.rs` unchanged and add `pub mod fdlimit;` next to `pub mod config;` in `src/lib.rs` (line 9) — done when: `diff ../rusty-conduit-switch/src/fdlimit.rs src/fdlimit.rs` prints nothing and `cargo test --locked --lib fdlimit` passes.
3. [done] In `run` (`src/lib.rs:58`), on the line right after `log_startup(&cfg, &logger);`, add `fdlimit::apply(&logger, cfg.max_connections, "--max-connections");` — done when: `cargo build --locked` succeeds.
4. [done] Manual check: run `cargo build --locked`, then `(ulimit -n 256; timeout 3 target/debug/rusty-conduit --http-port 39128 --socks-port 0 --log-level DEBUG) 2>&1 | grep -i 'open-file'` and record the output in `## Evidence`. If `timeout` is missing, use `gtimeout`, or start it in the background and `kill -INT` it after 2 s — done when: the output contains `raised open-file limit from 256 to` and no `NOTICE: open-file limit`.

## Verify
```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
diff ../rusty-conduit-switch/src/fdlimit.rs src/fdlimit.rs
grep -n 'fdlimit::apply(&logger, cfg.max_connections, "--max-connections");' src/lib.rs
```

## Escalate when
- `../rusty-conduit-switch/src/fdlimit.rs` does not exist → stop, set Status: needs-human — reason: the rusty-conduit-switch goal has not finished its phase-0003.
- `Cargo.lock` changes more than adding `rlimit` (and possibly `libc`) → stop, set Status: needs-human — reason: paste `git diff --stat Cargo.lock`.
- The manual check prints the NOTICE, or never prints `raised open-file limit` → stop, set Status: needs-human — reason: paste the output and `ulimit -Hn`.

## Log
- (append-only, one line per event)
- sub-task 1: added `[target.'cfg(unix)'.dependencies]` `rlimit = "0.12"` to `Cargo.toml` and updated `Cargo.lock`
- sub-task 2: copied `../rusty-conduit-switch/src/fdlimit.rs` verbatim to `src/fdlimit.rs` and added `pub mod fdlimit;` in `src/lib.rs`
- sub-task 3: added `fdlimit::apply(&logger, cfg.max_connections, "--max-connections");` right after `log_startup` in `src/lib.rs`
- sub-task 4: ran manual `ulimit -n 256` check and full `## Verify` suite; all passed

## Evidence
- (append-only: command → observed result)
- `cargo build && cargo build --locked && git diff --stat Cargo.lock` → exit 0; `Cargo.lock | 10 ++++++++++` (only `rlimit v0.12.0` added)
- `diff ../rusty-conduit-switch/src/fdlimit.rs src/fdlimit.rs && cargo test --locked --lib fdlimit` → exit 0 (diff empty; 4 passed, 0 failed)
- `cargo build --locked` → exit 0
- `(ulimit -n 256; ... target/debug/rusty-conduit --http-port 39128 --socks-port 0 --log-level DEBUG ...) 2>&1 | grep -i 'open-file'` → `2026-11-26T14:15:33.564Z DEBUG   raised open-file limit from 256 to 184320` (no NOTICE)
- `cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked && diff ../rusty-conduit-switch/src/fdlimit.rs src/fdlimit.rs && grep -n 'fdlimit::apply(&logger, cfg.max_connections, "--max-connections");' src/lib.rs` → exit 0 (42 unit + 37 integration tests passed, 0 failed; diff empty; line 62 matched)
