# phase-0001 — Align help wording and the MITM cache default

- Status: done
- Owner: executor
- Depends on: none
- Goal: Use `SOCKS4/4a/5` in every help string and default the MITM cert cache to 10240.
- Done when: `cargo test --locked` passes and the `--help` checks in `## Verify` pass.
- Pattern to follow: none

## Context
rusty-conduit is the family's reference for flag names and help text; only two things differ from the agreed contract. The tool supports SOCKS4a, but several help strings say `SOCKS4/5`. The user decided the MITM cache default is 10240 in all three tools (was 10000).
Only text and one default change in this phase; no flag names, env vars, or behaviour change.

## Scope
- In: `src/config.rs`, `tests/socks_proxy.rs` (formatting by `cargo fmt` only, sub-task 4)
- Out: `src/lib.rs`, every other file under `tests/`, `docs/`, `README.md`, `DOX.md`

## Sub-tasks
1. [done] Replace `SOCKS4/5` with `SOCKS4/4a/5` at `src/config.rs:33`, `:107`, `:203`, `:234`, and `:305` (check with `grep -n 'SOCKS4/5' src/config.rs`) — done when: `grep -c 'SOCKS4/5' src/config.rs` prints `0`.
2. [done] Set `DEFAULT_MITM_CERT_CACHE_MAX_ENTRIES` (`src/config.rs:46`) to `10_240`; change `[default: 10000]` to `[default: 10240]` in the `--mitm-cert-cache-max-entries` doc comment (`src/config.rs:195`); in the unit test at `src/config.rs:1337-1340` change `10_000` to `10_240` and `mitm_cert_cache_max_entries=10000` to `mitm_cert_cache_max_entries=10240` — done when: `grep -nE '10_000|10000' src/config.rs` prints nothing.
3. [done] Stale comments: the `Config` field doc comments at `src/config.rs:375` and `:377` say the lists are mutually exclusive, but they combine as block-with-exceptions (`HostFilter::from_lists`). Change them to exactly `/// Destination allowlist rules (exceptions to \`block\` when both are set).` and `/// Destination blocklist rules (combine with \`allow\` to carve out exceptions).` — done when: `grep -n 'mutually exclusive' src/config.rs` prints nothing.
4. [done] Planner answer to the escalation: `tests/socks_proxy.rs` is unchanged since the baseline but fails `cargo fmt --check` with the current stable rustfmt (1.11.0), so `main` already fails CI formatting. Run `cargo fmt` and keep only its changes to `tests/socks_proxy.rs` (the two hunks near lines 654 and 666); make no other edit to that file — done when: `cargo fmt --check` exits 0 and `git diff --stat -- tests/` lists only `tests/socks_proxy.rs`.

## Verify
```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo run -q -- --help | grep -F '[default: 10240]'
! cargo run -q -- --help | grep -F 'SOCKS4/5'
! grep -n 'mutually exclusive' src/config.rs
```

## Escalate when
- A test outside `src/config.rs` asserts `SOCKS4/5` or `10000` → stop, set Status: needs-human — reason: name the test and the assertion.
- `grep -n 'SOCKS4/5' src/config.rs` finds lines other than the five listed → change them too and note them in `## Log`; stop only if one is inside a string that a test compares against.

## Log
- (append-only, one line per event)
- planner amended before execution: added sub-task 3 (stale "mutually exclusive" comments found while reviewing rusty-conduit-ssh)
- sub-task 1: replaced `SOCKS4/5` with `SOCKS4/4a/5` at `src/config.rs:33`, `:107`, `:203`, `:234`, and `:305`
- sub-task 2: updated `DEFAULT_MITM_CERT_CACHE_MAX_ENTRIES` (`src/config.rs:46`), help doc comment (`src/config.rs:195`), and `caps_defaults` unit test (`src/config.rs:1337-1340`) to `10_240` / `10240`
- sub-task 3: updated `Config::allow` and `Config::block` doc comments at `src/config.rs:375` and `:377`
- verify: `cargo fmt --check` failed on pre-existing diffs in `tests/socks_proxy.rs:654` and `:666` (`tests/` is `Scope: Out`); all other verify checks passed; escalating to planner

- planner answered the escalation: format tests/socks_proxy.rs here (pre-existing rustfmt drift, file unchanged since baseline 344ad3b); scope widened to that file for cargo fmt only; added sub-task 4
- sub-task 4: ran `cargo fmt`, formatting only `tests/socks_proxy.rs` (two hunks near lines 654 and 666); re-ran full `## Verify` and all checks passed

## Evidence
- (append-only: command → observed result)
- `grep -c 'SOCKS4/5' src/config.rs` → `0`
- `grep -nE '10_000|10000' src/config.rs` → no output (exit 1)
- `! grep -n 'mutually exclusive' src/config.rs` → exit 0 (no matches)
- `cargo fmt --check` → exit 1; formatting diffs in `tests/socks_proxy.rs:654` and `tests/socks_proxy.rs:666` (pre-existing, `Scope: Out`)
- `rustfmt --check src/config.rs` → exit 0
- `cargo clippy --all-targets --locked -- -D warnings` → exit 0
- `cargo test --locked` → exit 0 (38 unit tests + 33 integration tests passed, 0 failed)
- `cargo run -q -- --help | grep -F '[default: 10240]'` → exit 0 (`Maximum cached MITM leaf certificates; expired then oldest entries are evicted, 0 = unlimited [default: 10240]`)
- `! cargo run -q -- --help | grep -F 'SOCKS4/5'` → exit 0
- `cargo fmt && git diff --stat -- tests/` → `tests/socks_proxy.rs | 8 +++-----` (only file changed)
- `cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked && cargo run -q -- --help | grep -F '[default: 10240]' && ! cargo run -q -- --help | grep -F 'SOCKS4/5' && ! grep -n 'mutually exclusive' src/config.rs` → exit 0 (38 unit tests + 33 integration tests passed, 0 failed)
