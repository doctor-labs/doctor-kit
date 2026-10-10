# Changelog

## 0.2.1

Patch release for the android-doctor and ble-doctor migrations. Source-compatible with 0.2.0: no field is
added to a public struct, every new hook is defaulted, and `doctor-kit/tests/compat_020.rs` pins the 0.2.0
shapes (`OutputArgs`, `McpWire`, `ExecOpts` literals).

### Fixed (security)
- Faces (`rich`, `plain`, `compact`), `shell` and `explore` now strip control, escape (ANSI/OSC), bidi and
  zero-width characters from finding text before showing it (contract section 8): id, category, message,
  location, evidence, remedy. `face::sanitize_envelope` is the one place. Machine output (`--json`, SARIF,
  the MCP `scan` result) is unchanged.

### Added
- `Doctor::render_human(&Envelope, &Face, &Theme) -> Option<String>`: a tool's own human report through
  `finish` (baseline, SARIF and exit pipeline kept). `finish_with` / `FinishOpts { never_fail }`: a
  report-only command that never fails on findings.
- `Doctor::load_baseline(&Path)`: tool-owned baseline parsing (default: the kit loader).
  `Doctor::preflight_error(PreflightKind, msg)` and `ScanFailure::plain` (no `error: ` prefix).
- `Doctor::mcp_texts() -> McpTexts` (invalid-request messages, unknown tool `{t}`, tool error wrapper `{e}`,
  serverInfo) and `mcp::exec_self_fmt` (error template with `{stderr}` / `{code}`). `ping` and -32600 stay.
- `Doctor::ci_overwrite()` and `Overwrite::Never` (per-tool `ci install` policy; `--force` still overrides).
- `Doctor::expand`: lazy children for `shell`/`explore` domain trees.

### Changed
- `install::upsert_block_with` writes the body byte-exact (no trim). `upsert_block` and the default
  `agent_files` block still trim, so default `install` output is unchanged.

## 0.2.0

Additive: a `Doctor` written for 0.1 compiles and prints the same bytes (stdout, files, exit codes), except for the
items under "Changed". New APIs are all defaulted trait methods or new modules.

### Added
- Scan pipeline: `Doctor::{collect, try_scan, score_collected, is_gap, tag_checks, default_fail_on, render_json,
  render_sarif, target}`, `Collected`, `ScanFailure`, `failure_envelope`, `TargetKind`, `Ctx::detached`.
  `--fail-on` defaults to the doctor's own severity on every command.
- `baseline` module and `scan --baseline FILE`: `baseline_state` per finding, `baseline` counts, exit 3 when a NEW
  finding reaches `--fail-on` (coverage gaps never gate under a baseline). `Doctor::{baseline_max_bytes,
  baseline_message, baseline_hook}`.
- `output` module: `OutputArgs`, `preflight`, `finish`, `report_failure` (shared by the kit's `scan` and a tool's own
  commands). `--sarif` is written atomically and refused when it names the baseline file.
- Extensible CLI: `run_with`, `Meta`, `Extensions`, `ExtCommand`, `command_with`. Detached doctors drop the
  directory arguments of the kit commands.
- `interrupt` module (feature `signals`): Ctrl-C / SIGTERM become exit 130.
- MCP: `McpTool`, `Doctor::{mcp_tools, mcp_wire}`, `McpWire`, `Protocol`, `mcp::{exec_self, ExecOpts, serve_verbose}`.
- `fix --from FILE`, `fix --agent claude|codex|cursor`, `fix --yolo`; `fix::{AGENTS, in_agent, launch, ...}`.
- `Doctor::agent_files`, `AgentFile`, `FileMode`, `install::{plan, write, put_safe, upsert_block_with}`,
  `install --print-only`, `ci install --force`.
- Faces and the shell tree group by the findings' categories when `data.categories` is absent.
- `examples/ext-doctor` (extension API) and golden snapshots of example-doctor's output.

### Changed
- Default cargo features are now `repl`, `tui`, `mcp`, `signals` (opt out with `default-features = false`).
- ratatui 0.29 -> 0.30 (not part of the public API).
- Writes by `install` / `ci install` refuse symlinked directories and files. `ci install` refuses to replace a
  workflow whose content differs unless `--force` (identical content is a no-op).
- The `mcp` module is compiled without the `mcp` feature; the `mcp` command still needs it.
- A failed `--sarif` write now prints the report and exits 2 (it used to exit before printing).
- Override `collect` or `try_scan`, not `scan`: the CLI calls `try_scan`.

### Help text
`scan --help` gains `--baseline`; `fix --help` gains `--from`, `--agent`, `--yolo`; `install --help` gains
`--print-only`; `ci install --help` gains `--force`.

## 0.1.1
Name, version and about in `--help`/`--version` come from the doctor.
