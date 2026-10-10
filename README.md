# doctor-kit

Reusable skeleton for "doctor" CLI tools. Output is the `doctor/1` envelope from
[doctor-core](https://crates.io/crates/doctor-core).

- `doctor-kit/`: the library. Implement `Doctor` (name, version, categories, checks), call `doctor_kit::run(MyDoctor)`.
  You get `scan` with faces (`--face plain|rich|compact`), `--json`, `--score`, `--headless`, `--scope full|changed`,
  themes (`--theme mono|clinical|contrast`, `theme.toml`), `doctor.toml` (score weights, check enable/disable),
  and with cargo features `repl` (`shell`) and `tui` (`explore`).
- `examples/example-doctor/`: a toy doctor in about one screen of code.
- `examples/samples/`: the face x theme captures.

    cargo run -p example-doctor -- scan examples/fixture --face rich --theme clinical
    cargo run -p example-doctor -- shell examples/fixture -c "ls; cd secrets; why 1"
    cargo run -p example-doctor -- explore examples/fixture

Kept minimal: the REPL has no tab completion or `--json` machine mode; the TUI is read-only navigation;
`--scope changed` only uses `git diff HEAD`.
