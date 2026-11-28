# phase-0002 — Pure-Rust crypto CI gate and CLI contract rows

- Status: done
- Owner: executor
- Depends on: phase-0001
- Goal: Run the engine's pure-Rust crypto check in CI and release verification, and lock the four missing shared flags in `tests/cli_contract.rs`.
- Done when: `python3 scripts/check-pure-rust-crypto.py` prints `PURE_RUST_CRYPTO_OK`, both workflows run it, and `cargo test --locked --test cli_contract` passes with the four new rows.
- Pattern to follow: `../rusty-conduit-engine/.github/workflows/ci.yml:26` (the `- run: python3 scripts/check-pure-rust-crypto.py` step) and `tests/cli_contract.rs:30` (one `SHARED_FLAGS` row)

## Context
Product invariant: `openssl-sys` and `aws-lc-sys` must never enter the dependency graph. The engine and the GUI enforce it with `scripts/check-pure-rust-crypto.py`; the CLIs do not. The script is generic (it runs `cargo tree --workspace --target all` from the repo root), so copy it unchanged.
`SHARED_FLAGS` in `tests/cli_contract.rs` omits `--mitm-insecure-host`, `--mitm-bypass-host`, `--mitm-insecure`, and `--error-page`, so their help text can drift between the three CLIs. rusty-conduit's own wording is the canonical text the sibling CLIs will copy. If the help assertion for a new row fails here, the rendered text in the failure message is the canonical one: put that exact string in the row (an allowed fix-up). Do not edit `src/config.rs` to make the test pass.

## Scope
- In: `scripts/check-pure-rust-crypto.py` (new), `.github/workflows/ci.yml`, `.github/workflows/release.yml`, `tests/cli_contract.rs`, `DOX.md` (Verification section only)
- Out: `src/`, `Cargo.toml`, `Cargo.lock`, `Dockerfile`, every other workflow step

## Sub-tasks
1. [done] Run `mkdir -p scripts && cp ../rusty-conduit-engine/scripts/check-pure-rust-crypto.py scripts/check-pure-rust-crypto.py && chmod 755 scripts/check-pure-rust-crypto.py` — done when: `cmp ../rusty-conduit-engine/scripts/check-pure-rust-crypto.py scripts/check-pure-rust-crypto.py` exits 0 and `python3 scripts/check-pure-rust-crypto.py` prints `PURE_RUST_CRYPTO_OK`.
2. [done] In `.github/workflows/ci.yml`, insert the line `      - run: python3 scripts/check-pure-rust-crypto.py` directly after `      - run: cargo clippy --all-targets --locked -- -D warnings` (line 23, `lint` job). In `.github/workflows/release.yml`, insert the same line directly after `      - run: cargo test --locked` (line 27, `verify` job). Same six-space indentation as the neighbouring steps — done when: `grep -c 'check-pure-rust-crypto.py' .github/workflows/ci.yml` prints `1`, the same for `release.yml` prints `1`, and `ruby -ryaml -e 'ARGV.each { |f| YAML.load_file(f) }' .github/workflows/ci.yml .github/workflows/release.yml` exits 0.
3. [done] In `tests/cli_contract.rs`, insert these three rows directly after the `("mitm-cert-cache-max-entries", ...)` row (line 30), exactly: (block 1 below) and this row directly after the `("socks-block-web", ...)` row, exactly: (block 2 below) — done when: `cargo test --locked --test cli_contract` passes (apply the fix-up rule in Context for a help-text mismatch).
   Block 1:
   ```rust
       ("mitm-insecure-host", Some("RULE"), "Accept invalid/self-signed upstream TLS certificates under MITM for matching destinations (IP, CIDR, FQDN, or *.domain); repeatable or comma-separated"),
       ("mitm-bypass-host", Some("RULE"), "Exclude matching destinations (IP, CIDR, FQDN, or *.domain) from MITM interception: they get an opaque CONNECT tunnel instead; repeatable or comma-separated"),
       ("mitm-insecure", None, "UNSAFE blind trust: skip upstream TLS certificate verification for ALL MITM targets"),
   ```
   Block 2:
   ```rust
       ("error-page", Some("PATH"), "HTML template file for proxy-generated HTTP error pages (placeholders: {{status}}, {{reason}}, {{message}}, {{target}}) [default: built-in page]"),
   ```
4. [done] In `DOX.md`, in the `### Verification` list (the bullets `cargo fmt --check`, `cargo clippy ...`, `cargo test --locked`), add the bullet `- `python3 scripts/check-pure-rust-crypto.py` (no `openssl-sys` or `aws-lc-sys` in any target's dependency graph)` after `cargo test --locked`. Change nothing else in `DOX.md` — done when: `grep -c 'check-pure-rust-crypto.py' DOX.md` prints `1`.

## Verify
```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
python3 scripts/check-pure-rust-crypto.py
cmp ../rusty-conduit-engine/scripts/check-pure-rust-crypto.py scripts/check-pure-rust-crypto.py
test "$(grep -c 'check-pure-rust-crypto.py' .github/workflows/ci.yml)" = 1
test "$(grep -c 'check-pure-rust-crypto.py' .github/workflows/release.yml)" = 1
ruby -ryaml -e 'ARGV.each { |f| YAML.load_file(f) }' .github/workflows/ci.yml .github/workflows/release.yml
git diff --exit-code -- src Cargo.toml Cargo.lock
```

## Escalate when
- `python3 scripts/check-pure-rust-crypto.py` reports a banned crate → stop, set Status: needs-human — reason: paste its output (that is a real invariant breach, not a script problem).
- A new contract row fails for a reason other than its help text (missing flag, hidden flag, wrong env var, wrong value name) → stop, set Status: needs-human — reason: paste the failure.
- `../rusty-conduit-engine/scripts/check-pure-rust-crypto.py` does not exist → stop, set Status: needs-human — reason: the workspace engine checkout is missing.

## Log
- (append-only, one line per event)
- planner: sub-task 4 tightened before delegation — the `Verification (mirrors ...)` bullet it referenced does not exist; the only Verification list is `### Verification` at DOX.md:229
- sub-task 1: copied `../rusty-conduit-engine/scripts/check-pure-rust-crypto.py` to `scripts/check-pure-rust-crypto.py` with mode 755
- sub-task 2: added `python3 scripts/check-pure-rust-crypto.py` step to `.github/workflows/ci.yml` (`lint` job) and `.github/workflows/release.yml` (`verify` job)
- sub-task 3: added `--mitm-insecure-host`, `--mitm-bypass-host`, `--mitm-insecure`, and `--error-page` rows to `SHARED_FLAGS` in `tests/cli_contract.rs`
- sub-task 4: added `python3 scripts/check-pure-rust-crypto.py` bullet to `### Verification` in `DOX.md`

## Evidence
- (append-only: command → observed result)
- `cmp ../rusty-conduit-engine/scripts/check-pure-rust-crypto.py scripts/check-pure-rust-crypto.py && python3 scripts/check-pure-rust-crypto.py` → exit 0, `PURE_RUST_CRYPTO_OK`
- `grep -c 'check-pure-rust-crypto.py' .github/workflows/ci.yml && grep -c 'check-pure-rust-crypto.py' .github/workflows/release.yml && ruby -ryaml -e 'ARGV.each { |f| YAML.load_file(f) }' .github/workflows/ci.yml .github/workflows/release.yml` → exit 0, `1` and `1`
- `cargo test --locked --test cli_contract` → exit 0, 3 passed, 0 failed
- `grep -c 'check-pure-rust-crypto.py' DOX.md` → exit 0, `1`
- `## Verify` (`cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked`, `python3 scripts/check-pure-rust-crypto.py`, `cmp`, workflow grep counts = 1, ruby YAML parse, `git diff --exit-code -- src Cargo.toml Cargo.lock`) → exit 0 (104 passed, 0 failed; `PURE_RUST_CRYPTO_OK`)
