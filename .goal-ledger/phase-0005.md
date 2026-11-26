# phase-0005 — Update user docs and DOX Feature Map

- Status: done
- Owner: executor
- Depends on: phase-0001, phase-0002, phase-0003, phase-0004
- Goal: Make the README, user guides, sizing guide, and `DOX.md` describe the new MITM cache default, the `validate` subcommand, and the startup open-file limit raise.
- Done when: every command in `## Verify` exits 0.
- Pattern to follow: `../rusty-conduit-ssh/docs/USER-GUIDE.md` (read only: its `## Checking a configuration` section and `### Open-file limit` subsection, for wording)

## Context
Phases 0001–0004 changed: MITM cache default 10000 → 10240 (about 30–50 MB at the cap); `SOCKS4/5` → `SOCKS4/4a/5` in help; new `rusty-conduit validate` prints `config OK: <path|none>` and exits 0, or prints `config invalid: <error>` to stderr and exits 1; at startup the tool raises its soft open-file limit toward the hard limit when it is below `2 × max_connections + 64`, and logs `NOTICE: open-file limit is …` if it still cannot reach it.
`docs/MASTER-PLAN.md` and `FUTURE-PLANS.md` are planning records; leave them alone. In `DOX.md`, edit only the `## Project Contract` and `## Feature Map` sections; the rest of that file is framework text.

## Scope
- In: `README.md`, `docs/USER-GUIDE-BASIC.md`, `docs/USER-GUIDE-ADVANCED.md`, `docs/PERFORMANCE-AND-SIZING.md`, `DOX.md`
- Out: `docs/MASTER-PLAN.md`, `docs/MITM-CA-GUIDE.md`, `FUTURE-PLANS.md`, `src/`, `tests/`, `Dockerfile`, every file outside this repository

## Sub-tasks
1. [done] `docs/USER-GUIDE-ADVANCED.md`: change `10000` to `10240` (reference row near line 40, TOML example near line 70); add a `validate` entry to the CLI reference or a short `## Checking a configuration` section with the example `rusty-conduit validate --config proxy.toml`, the two output forms, and the exit codes — done when: `grep -n '10000' docs/USER-GUIDE-ADVANCED.md` prints nothing and `grep -n 'rusty-conduit validate' docs/USER-GUIDE-ADVANCED.md` prints a line.
2. [done] `docs/PERFORMANCE-AND-SIZING.md`: change `10000` to `10240` (lines 19, 33, 85); in section 3, after the `ulimit -n >= (2 × max_connections) + 64` rule, add a paragraph saying rusty-conduit raises its soft limit toward the hard limit at startup when it is below that value, logs the raise at `DEBUG`, and logs a `NOTICE: open-file limit is …` at `INFO` if the hard limit is still too low; keep options A and B for that case — done when: `grep -n '10000' docs/PERFORMANCE-AND-SIZING.md` prints nothing and `grep -n 'NOTICE: open-file limit' docs/PERFORMANCE-AND-SIZING.md` prints a line.
3. [done] `docs/USER-GUIDE-BASIC.md`: in `## 4. Common Everyday Options` add `### Check a configuration before starting` with `rusty-conduit validate --config proxy.toml`; replace any `SOCKS4/5` with `SOCKS4/4a/5` — done when: `grep -n 'rusty-conduit validate' docs/USER-GUIDE-BASIC.md` prints a line.
4. [done] `README.md`: in `### 3. Test` add a line showing `rusty-conduit validate --config proxy.toml`; replace any `SOCKS4/5` with `SOCKS4/4a/5` — done when: `grep -n 'rusty-conduit validate' README.md` prints a line.
5. [done] `DOX.md` `## Feature Map`: add `- **\`validate\` subcommand** — loads and validates the configuration without binding listeners and exits 0/1. Start: \`src/lib.rs\` (\`cmd_validate\`). Files: \`src/config.rs\` (\`Command\`), \`tests/validate_cmd.rs\`.`, `- **Open-file limit** — raises the soft \`RLIMIT_NOFILE\` at startup and logs a NOTICE when it cannot hold \`--max-connections\`. Start: \`src/fdlimit.rs\`. Files: \`src/lib.rs\` (\`run\`), \`Cargo.toml\` (\`rlimit\`), \`docs/PERFORMANCE-AND-SIZING.md\`.`, and `- **CLI family contract** — test that shared flags keep the family's names, value names, env vars, help text, and defaults. Start: \`tests/cli_contract.rs\`.`; in the **CLI entrypoint and process lifecycle** bullet add `, dispatches \`validate\`` after `parses args/env` — done when: `grep -c 'cmd_validate\|fdlimit.rs\|cli_contract.rs' DOX.md` prints at least `3`.
6. [done] Stale `--socks-block-web` and filter docs (found in planner review of the sibling repos). In `docs/USER-GUIDE-ADVANCED.md`: replace the `--socks-block-web-in-mitm` reference row (line 41) with exactly `| \`--socks-block-web\` | \`RUSTY_CONDUIT_SOCKS_BLOCK_WEB\` | \`socks_block_web\` | \`false\` | Reject HTTP/HTTPS traffic over SOCKS4/4a/5 (works with or without MITM). The old names \`--socks-block-web-in-mitm\` and \`mitm.socks_block_web\` still work |`; in the TOML example move `socks_block_web = true` out of `[mitm]` (near line 71) to the line directly before `[mitm]`; delete the fatal-error bullet `Supplying both \`--allow\` and \`--block\` (including across CLI/env and TOML).` (they now combine as block-with-exceptions); delete the INFO-notice bullet `Setting \`--socks-block-web-in-mitm\` while MITM is OFF (flag is inactive).` (the code no longer emits it; see the test at `src/config.rs:1309`); rename the heading `## 3. Blocking Web Traffic over SOCKS in MITM Mode` to `## 3. Blocking Web Traffic over SOCKS`, start its first sentence with `Clients could otherwise bypass HTTPS logging by sending web traffic over SOCKS. Enable \`--socks-block-web\` to prevent that (it also works without MITM):`, and change `--socks-block-web-in-mitm` to `--socks-block-web` in its example command; make the `--allow` / `--block` rows (lines 32–33) say they combine as block-with-exceptions instead of mutually exclusive. In `docs/USER-GUIDE-BASIC.md` line 158 replace `(mutually exclusive; repeatable or comma-separated)` with `(repeatable or comma-separated; together, \`--allow\` entries are exceptions to \`--block\`)` and on line 180 change `--socks-block-web-in-mitm` to `--socks-block-web`. In `README.md` line 65 change `--socks-block-web-in-mitm` to `--socks-block-web` — done when: `grep -rn 'socks-block-web-in-mitm' README.md docs/USER-GUIDE-BASIC.md docs/USER-GUIDE-ADVANCED.md | grep -v 'still work'` and `grep -n 'mutually exclusive' README.md docs/USER-GUIDE-BASIC.md docs/USER-GUIDE-ADVANCED.md` print nothing.

## Verify
```sh
! grep -nE '10000|SOCKS4/5' README.md docs/USER-GUIDE-BASIC.md docs/USER-GUIDE-ADVANCED.md docs/PERFORMANCE-AND-SIZING.md
grep -n 'rusty-conduit validate' README.md docs/USER-GUIDE-BASIC.md docs/USER-GUIDE-ADVANCED.md
grep -n 'NOTICE: open-file limit' docs/PERFORMANCE-AND-SIZING.md
grep -n 'cmd_validate' DOX.md
grep -n 'fdlimit.rs' DOX.md
! grep -rn 'socks-block-web-in-mitm' README.md docs/USER-GUIDE-BASIC.md docs/USER-GUIDE-ADVANCED.md | grep -v 'still work'
! grep -n 'mutually exclusive' README.md docs/USER-GUIDE-BASIC.md docs/USER-GUIDE-ADVANCED.md
cargo test --locked
```

## Escalate when
- A doc describes behaviour that the code from phases 0001–0004 contradicts → stop, set Status: needs-human — reason: quote the doc line and what the code does.
- A `10000` in these docs refers to something other than the MITM cache (for example a port or a byte count) → leave it, note file:line in `## Log`, and stop with Status: needs-human — reason: the Verify grep needs adjusting by the planner.

## Log
- (append-only, one line per event)
- planner amended before execution: added sub-task 6 (stale --socks-block-web-in-mitm and allow/block docs found while reviewing rusty-conduit-ssh)
- sub-tasks 1–6: updated `docs/USER-GUIDE-ADVANCED.md`, `docs/PERFORMANCE-AND-SIZING.md`, `docs/USER-GUIDE-BASIC.md`, `README.md`, and `DOX.md` (`## Feature Map`); also updated `README.md:15` (`--socks-block-web-in-mitm` → `--socks-block-web`) and `docs/USER-GUIDE-ADVANCED.md:58` (TOML filter comment) so all stale references are removed

## Evidence
- (append-only: command → observed result)
- `! grep -nE '10000|SOCKS4/5' README.md docs/USER-GUIDE-BASIC.md docs/USER-GUIDE-ADVANCED.md docs/PERFORMANCE-AND-SIZING.md` → exit 0
- `grep -n 'rusty-conduit validate' README.md docs/USER-GUIDE-BASIC.md docs/USER-GUIDE-ADVANCED.md` → exit 0 (matched `README.md:46`, `docs/USER-GUIDE-BASIC.md:179`, `docs/USER-GUIDE-ADVANCED.md:109`)
- `grep -n 'NOTICE: open-file limit' docs/PERFORMANCE-AND-SIZING.md` → exit 0 (line 51)
- `grep -n 'cmd_validate' DOX.md && grep -n 'fdlimit.rs' DOX.md && grep -c 'cmd_validate\|fdlimit.rs\|cli_contract.rs' DOX.md` → exit 0 (`3`)
- `! grep -rn 'socks-block-web-in-mitm' README.md docs/USER-GUIDE-BASIC.md docs/USER-GUIDE-ADVANCED.md | grep -v 'still work'` → exit 0
- `! grep -n 'mutually exclusive' README.md docs/USER-GUIDE-BASIC.md docs/USER-GUIDE-ADVANCED.md` → exit 0
- `cargo test --locked` → exit 0 (42 unit + 40 integration tests passed, 0 failed)
