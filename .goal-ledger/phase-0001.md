# phase-0001 — Pin the engine by rev with the cli-support and test-support features

- Status: done
- Owner: executor
- Depends on: none
- Goal: Build against the untagged engine `main` (`05485a4…`) with the `cli-support` feature for the binary and `test-support` for tests, with no other code change.
- Done when: every command in `## Verify` exits 0.
- Pattern to follow: `Cargo.toml:20` (the current engine dependency line)

## Context
The engine is not tagged until every optimization is done (user decision), so consumers pin its commit. A trial build with only the pin changed passed clippy and all tests, so no source change is expected in this phase. `default-features = false` stays (rusty-conduit never uses regex routing). Cargo merges the features of a dependency declared in both `[dependencies]` and `[dev-dependencies]` when the source and `rev` match exactly.

## Scope
- In: `Cargo.toml`, `Cargo.lock`, `DOX.md` (the "Engine lives elsewhere" bullet only)
- Out: `src/`, `tests/`, `.github/`, every other doc

## Sub-tasks
1. [done] In `Cargo.toml`, replace line 20 exactly with: `rusty-conduit-engine = { git = "https://github.com/jpbaking/rusty-conduit-engine", rev = "05485a4c0fdd5315e4e6954a199a55d34cd17bc1", default-features = false, features = ["cli-support"] }` — done when: `grep -c 'rev = "05485a4c0fdd5315e4e6954a199a55d34cd17bc1"' Cargo.toml` prints `1`.
2. [done] In `Cargo.toml`, insert directly below the `[dev-dependencies]` header these two lines: `# Shared unit-test fixtures (test CA, echo/origin servers, TLS client, log capture).` and `rusty-conduit-engine = { git = "https://github.com/jpbaking/rusty-conduit-engine", rev = "05485a4c0fdd5315e4e6954a199a55d34cd17bc1", default-features = false, features = ["cli-support", "test-support"] }` — done when: `grep -c 'rev = "05485a4c0fdd5315e4e6954a199a55d34cd17bc1"' Cargo.toml` prints `2`.
3. [done] Run `cargo update -p rusty-conduit-engine` once (no other `cargo update`) — done when: `cargo build --locked` succeeds and `git diff Cargo.lock | grep -c '^[-+]source = "git+https://github.com/jpbaking/rusty-conduit-engine'` prints `2` (one removed tag source, one added rev source).
4. [done] In `DOX.md` (line 220), replace exactly `pinned by tag in \`Cargo.toml\`, with \`default-features = false\` (no regex \`routing\` feature)` with `pinned in \`Cargo.toml\` (by \`rev\` until the engine's \`v0.7.0\` tag), with \`default-features = false\` (no regex \`routing\` feature) and the \`cli-support\` feature (plus \`test-support\` for tests)` — done when: `grep -c 'by `rev` until the engine' DOX.md` prints `1`.

## Verify
```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
python3 scripts/check-pure-rust-crypto.py
test "$(grep -c 'rev = "05485a4c0fdd5315e4e6954a199a55d34cd17bc1"' Cargo.toml)" = 2
git diff --exit-code -- src tests .github
```

## Escalate when
- `cargo update -p rusty-conduit-engine` changes any package other than the engine and crates it newly pulls in (for example `rlimit`'s own dependencies) → stop, set Status: needs-human — reason: paste `git diff --stat Cargo.lock` and the changed package names.
- Build, clippy, or a test fails → stop, set Status: needs-human — reason: paste the first error (the trial build passed, so this is unexpected).
- `check-pure-rust-crypto.py` reports a banned crate → stop, set Status: needs-human — reason: paste its output.

## Log
- (append-only, one line per event)
- sub-task 1: replaced `Cargo.toml:20` with rev pin `05485a4c0fdd5315e4e6954a199a55d34cd17bc1` and `features = ["cli-support"]`.
- sub-task 2: inserted `rusty-conduit-engine` with `cli-support` and `test-support` features directly under `[dev-dependencies]` in `Cargo.toml`.
- sub-task 3: ran `cargo update -p rusty-conduit-engine`; only `rusty-conduit-engine` changed in `Cargo.lock` (+3/-1 lines) and `cargo build --locked` succeeded.
- sub-task 4: updated `DOX.md:220` "Engine lives elsewhere" bullet to document the `rev` pin and `cli-support` / `test-support` features.
- verify: all 6 `## Verify` commands exited 0; closing phase-0001.

- planner accepted phase-0001: Cargo.lock diff is the engine source line plus its new `libc` / `rlimit` edges only; DOX line correct; Verify re-run (104 passed, clippy clean, PURE_RUST_CRYPTO_OK)

## Evidence
- (append-only: command → observed result)
- `grep -c 'rev = "05485a4c0fdd5315e4e6954a199a55d34cd17bc1"' Cargo.toml` → `1` (exit 0)
- `grep -c 'rev = "05485a4c0fdd5315e4e6954a199a55d34cd17bc1"' Cargo.toml` → `2` (exit 0)
- `cargo update -p rusty-conduit-engine && cargo build --locked && git diff Cargo.lock | grep -c '^[-+]source = "git+https://github.com/jpbaking/rusty-conduit-engine'` → built cleanly (2.81s), `2` (exit 0); `git diff --stat Cargo.lock`: `1 file changed, 3 insertions(+), 1 deletion(-)` (only `rusty-conduit-engine` source/deps changed)
- `grep -c 'by \`rev\` until the engine' DOX.md` → `1` (exit 0)
- `cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked && python3 scripts/check-pure-rust-crypto.py && test "$(grep -c 'rev = "05485a4c0fdd5315e4e6954a199a55d34cd17bc1"' Cargo.toml)" = 2 && git diff --exit-code -- src tests .github` → all exit 0; clippy clean, 104 tests passed (0 failed, 0 ignored), `PURE_RUST_CRYPTO_OK`, no diff in `src/`, `tests/`, `.github/`
