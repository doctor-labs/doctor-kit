# doctor-kit

Reusable skeleton for "doctor" CLI tools. Output is the `doctor/1` envelope from
[doctor-core](https://crates.io/crates/doctor-core).

- `doctor-kit/`: the library. Implement `Doctor` (name, version, categories, checks), call `doctor_kit::run(MyDoctor)`.
  You get `scan` with faces (`--face plain|rich|compact`), `--json`, `--score`, `--headless`, `--scope full|changed`,
  themes (`--theme mono|clinical|contrast`, `theme.toml`), `doctor.toml` (score weights, check enable/disable),
  and with cargo features `repl` (`shell`), `tui` (`explore`) and `mcp` (`mcp`).
- Agentic base, inherited by every tool, customized through `Doctor` hooks (`command`, `fix_prompt`,
  `skill_body`, `ci_setup_steps`, `ci_workflow`):
  `tool mcp` (stdio JSON-RPC; `scan` returns the CLI's envelope byte-identical, plus `fix`),
  `tool fix [<id>] [--copy]` (prompt for a coding agent), `tool install [--agent X] [--agent-hooks]`
  (Claude skill, Cursor rule, AGENTS.md block, optional PostToolUse hook), `tool ci install [--print]`
  (PR workflow), and `scan --scope changed [--base origin/main]` for PR mode.
- `examples/example-doctor/`: a toy doctor in about one screen of code.
- `examples/samples/`: the face x theme captures.

    cargo run -p example-doctor -- scan examples/fixture --face rich --theme clinical
    cargo run -p example-doctor -- shell examples/fixture -c "ls; cd secrets; why 1"
    cargo run -p example-doctor -- explore examples/fixture

Kept minimal: the REPL has no tab completion or `--json` machine mode; the TUI is read-only navigation;
`--scope changed` only uses `git diff HEAD`.
