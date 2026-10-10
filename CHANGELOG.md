# Changelog

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
