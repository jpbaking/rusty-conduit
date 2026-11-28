# phase-0003 — Use the engine's shared [mitm] and [logging] file sections

- Status: done
- Owner: planner
- Depends on: phase-0002
- Goal: Replace `FileMitmConfig`, `FileLoggingConfig`, and their merge in `src/config.rs` with the engine's `cli_support::file` types and `merge_mitm` / `merge_logging`, keeping every precedence rule and error message.
- Done when: every command in `## Verify` exits 0; existing config tests pass unchanged; new tests cover the deprecated `mitm.cert_cache_max` key (accepted, lower precedence, startup notice) and `Debug` redaction of the file passphrase.
- Pattern to follow: `src/config.rs` `FileConfig::merge` (around line 554) and `file_duration` (around line 1000); the engine's `src/cli_support/file.rs` (`merge_mitm`, `merge_logging`, `FileValueError`)

## Context
M-01 / M-18. The engine's sections behave like this crate's except: (1) `FileMitmConfig` redacts `ca_passphrase` in `Debug` (this crate's derived `Debug` printed it); (2) the deprecated `mitm.cert_cache_max` key is now accepted below `cert_cache_max_entries`, with the notice `DEPRECATED_CERT_CACHE_MAX_NOTICE`. Decision: the notice reaches users through `Config::notices()` (stored on `Config` at merge time), like the existing startup notices. `FileValueError` maps field for field onto `ConfigError::InvalidFileValue`. Top-level keepalive, `allow` / `block`, and listener keys stay in this crate.

## Scope
- In: `src/config.rs`, `docs/USER-GUIDE-ADVANCED.md` (the `--mitm-cert-cache-max-entries` table row, line 41, only: mention the deprecated `mitm.cert_cache_max` spelling), `DOX.md` (Feature Map line for configuration, if it names the moved types)
- Out: `src/lib.rs`, `tests/`, `Cargo.toml`, `Cargo.lock`, `.github/`

## Sub-tasks
1. [done] Use `FileMitmConfig` / `FileLoggingConfig` from the engine in `FileConfig`; delete the local structs — done when: `cargo test --locked` passes.
2. [done] Rebuild the MITM and logging parts of `FileConfig::merge` on `merge_mitm` / `merge_logging`, mapping `FileValueError` to `ConfigError::InvalidFileValue` — done when: every existing config test passes unchanged.
3. [done] Deprecation notice plumbing and tests (deprecated key accepted with notice; `_entries` wins; passphrase redacted in `Debug` of the file config) — done when: the new tests pass.
4. [done] Docs: mention the deprecated key in the user guide's `--mitm-cert-cache-max-entries` row — done when: `grep -c 'cert_cache_max' docs/USER-GUIDE-ADVANCED.md` grows by at least 1.

## Verify
```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
python3 scripts/check-pure-rust-crypto.py
git diff --exit-code -- src/lib.rs tests Cargo.toml Cargo.lock .github
```

## Escalate when
- An existing config test's expected message or precedence would have to change → stop; that is a behavior change beyond M-18 and needs the user.

## Log
- (append-only, one line per event)
- planner begins phase-0003 (planner-owned)
- sub-task 1: local `FileMitmConfig` / `FileLoggingConfig` deleted; `config::{FileMitmConfig, FileLoggingConfig}` re-export the engine's `cli_support::file` types (FileConfig unchanged otherwise)
- sub-task 2: `FileConfig::merge` builds `MitmCliValues` / `LoggingCliValues` and calls `merge_mitm` / `merge_logging`; `FileValueError` maps onto `ConfigError::InvalidFileValue { key, message }`; the logging merge runs after the keepalive keys so a file with several bad values reports them in the former order (`mitm.cert_ttl`, keepalive, interval, `logging.level`, `logging.format`); defaults equal the engine's (TTL 1 h, cache 10 240, INFO, text)
- sub-task 3: `Config::file_notices` (set from `MitmSettings::notices`, included in `Debug` and `Default`) is appended by `Config::notices()`; tests `deprecated_cert_cache_max_key_is_accepted_with_a_notice`, `file_config_debug_hides_the_ca_passphrase`
- sub-task 4: user guide row for `--mitm-cert-cache-max-entries` mentions the older `mitm.cert_cache_max` key; the done-when's `grep -c` (line count) cannot grow because the text is on the same table row — checked with `grep -c 'key \`mitm.cert_cache_max\` is still read'` → 1 instead; DOX names neither moved type, so it needed no change
- phase-0003 verified and closed

## Evidence
- (append-only: command → observed result)
- every pre-existing config test passed unchanged after sub-task 2 (96 passed)
- `cargo test --locked --lib -- deprecated_cert_cache_max file_config_debug` → 2 passed
- `cargo fmt --check` → exit 0; clippy → exit 0; `cargo test --locked` → 98 passed; PURE_RUST_CRYPTO_OK; `git diff --exit-code -- src/lib.rs tests Cargo.toml Cargo.lock .github` → exit 0
