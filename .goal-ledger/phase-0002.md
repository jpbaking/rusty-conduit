# phase-0002 — Add the validate subcommand

- Status: done
- Owner: executor
- Depends on: phase-0001
- Goal: `rusty-conduit validate` loads, merges, and validates the configuration exactly as startup does, without binding any listener, and exits 0 or 1.
- Done when: `cargo test --locked` passes, including the new `tests/validate_cmd.rs`.
- Pattern to follow: `../rusty-conduit-ssh/src/lib.rs` `cmd_validate` and its dispatch in `main_entry`, and `../rusty-conduit-ssh/tests/validate_cmd.rs` (read only; both exist once the rusty-conduit-ssh goal finished its phase-0002). Same shape in `../rusty-conduit-switch/src/lib.rs:268-300`.

## Context
All three tools now share `validate`: success prints `config OK: <config path or none>` to stdout followed by any NOTICE lines and exits 0; failure prints `config invalid: <error>` to stderr and exits 1. As in rusty-conduit-ssh, a config file is optional (everything can come from flags and env). rusty-conduit has no positional arguments, so `Validate` is a unit variant. `--config` must be `global = true` so `validate --config x.toml` works. Adding a subcommand changes the usage line to `rusty-conduit [OPTIONS] [COMMAND]`; that is expected.

## Scope
- In: `src/config.rs`, `src/lib.rs`, `tests/validate_cmd.rs` (new)
- Out: every other file under `tests/`, `docs/`, `README.md`, `DOX.md`

## Sub-tasks
1. [done] In `src/config.rs`: add `Subcommand` to the clap import (line 19); add `#[derive(Debug, Clone, PartialEq, Eq, Subcommand)] pub enum Command { Validate }` above `CliArgs`, with doc `/// Subcommands. Without one, the proxy runs.` and variant doc `/// Validate the configuration (flags, environment, and --config file) and exit 0/1`; add `#[command(subcommand)] pub command: Option<Command>,` as the first field of `CliArgs`; add `global = true` to the `--config` `#[arg(...)]` (line 96) — done when: `cargo build` succeeds and `cargo run -q -- validate --help` exits 0.
2. [done] In `src/lib.rs`: add `use std::io::Write;` and `pub fn cmd_validate(cli: CliInput, out: &mut dyn Write, err: &mut dyn Write) -> u8` with doc `/// \`validate\`: exit 0 when the configuration is valid, 1 otherwise.`. Call `load_config(cli)`; on `Ok(cfg)` write `config OK: {path}` where `path` is `cfg.config_path` displayed or `none`, then each `cfg.notices()` entry on its own line, and return `0`; on `Err((e, _))` write `config invalid: {e}` to `err` and return `EXIT_FATAL` — done when: `cargo build` succeeds.
3. [done] In `main_entry` (`src/lib.rs:257`), right after `let cli = CliInput::parse();`, add: if `cli.args.command == Some(Command::Validate)`, return `ExitCode::from(cmd_validate(cli, &mut std::io::stdout(), &mut std::io::stderr()))` — done when: `cargo run -q -- validate` prints `config OK: none` and exits 0, and `cargo run -q -- validate --bind not-an-ip` exits 2 (clap rejects the value).
4. [done] Create `tests/validate_cmd.rs` with four tests that run `env!("CARGO_BIN_EXE_rusty-conduit")` with `RUSTY_CONDUIT_CONFIG` removed from the environment: (a) `validate` → exit 0, stdout starts with `config OK: none`; (b) a TOML file under `std::env::temp_dir()` containing `http_port = 3128` passed as `validate --config <file>` → exit 0, stdout contains `config OK:` and the file path; (c) a file containing `http_port = "nope"` → exit 1, stderr contains `config invalid:`; (d) `--no-such-flag` → exit 2. Remove the temp files at the end of each test — done when: `cargo test --locked --test validate_cmd` passes 4 tests.

## Verify
```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo run -q -- validate | grep -F 'config OK: none'
cargo run -q -- --help | grep -F 'validate'
```

## Escalate when
- An existing test fails because the parser now has a subcommand (for example a test that builds `CliArgs` by hand without `command`) → stop, set Status: needs-human — reason: name the test and paste the error.
- `load_config` binds a port, opens the access logs, or touches the network → stop, set Status: needs-human — reason: name the call that does it.

## Log
- (append-only, one line per event)
- sub-task 1: added `Command::Validate`, `CliArgs::command`, and `global = true` on `--config` in `src/config.rs`
- sub-task 2: added `use std::io::Write;` and `pub fn cmd_validate` in `src/lib.rs`
- sub-task 3: dispatched `Command::Validate` to `cmd_validate` in `main_entry` (`src/lib.rs`)
- sub-task 4: added `tests/validate_cmd.rs` with 4 binary tests and verified full `## Verify` suite passes

## Evidence
- (append-only: command → observed result)
- `cargo build && cargo run -q -- validate --help` → exit 0 (`Usage: rusty-conduit validate [OPTIONS]` with `--config <PATH>`)
- `cargo build` → exit 0
- `cargo run -q -- validate` → exit 0 (`config OK: none`); `cargo run -q -- validate --bind not-an-ip` → exit 2
- `cargo test --locked --test validate_cmd` → exit 0 (4 passed, 0 failed)
- `cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked && cargo run -q -- validate | grep -F 'config OK: none' && cargo run -q -- --help | grep -F 'validate'` → exit 0 (38 unit + 37 integration tests passed, 0 failed)
