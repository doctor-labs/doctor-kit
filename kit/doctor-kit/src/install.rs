//! `install` (agent skill + Cursor rule + AGENTS.md block) and `ci install` (GitHub Actions).
//! Templates are defaults; a `Doctor` overrides `skill_body` / `ci_workflow`.
use crate::doctor::Doctor;
use serde_json::{json, Value};
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Agent {
    Claude,
    Cursor,
    /// Codex and opencode both read AGENTS.md.
    Codex,
    Opencode,
}

pub fn default_skill(command: &str, categories: &[&str]) -> String {
    format!(
        "# {command}\n\n\
`{command}` scans a target and reports findings. Run it; do not guess.\n\n\
```sh\n\
{command} scan --json                  # one doctor/1 JSON envelope on stdout\n\
{command} scan --scope changed --headless   # only files changed in git (PR mode)\n\
{command} fix [<finding-id>]           # a prompt to hand to a coding agent\n\
{command} mcp                          # the same scan as an MCP stdio server\n\
```\n\n\
Categories: {}.\n\n\
Envelope: `schema` is `doctor/1`; `findings[]` hold `id`, `severity`, `location`, `message`,\n\
`evidence`, `remedy`; `score.value` is 0-100. Exit code 0 = clean, 1 = findings at or above\n\
`--fail-on` (default high), 2 = the tool failed. Fix at the source, then re-run the scan.\n",
        categories.join(", ")
    )
}

pub fn default_workflow(command: &str, setup_steps: &str) -> String {
    let steps: String = setup_steps
        .lines()
        .map(|l| format!("      {l}\n"))
        .collect();
    "# Written by `__CMD__ ci install`. Scans only what the pull request changed.
name: __CMD__

on:
  pull_request:

permissions:
  contents: read

jobs:
  scan:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
__STEPS__      - name: Scan changed files
        run: __CMD__ scan . --scope changed --base origin/${{ github.base_ref }} --headless --fail-on high
"
    .replace("__CMD__", command)
    .replace("__STEPS__", &steps)
}

fn put(root: &Path, rel: &str, text: &str) -> io::Result<PathBuf> {
    let p = root.join(rel);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(&p, text)?;
    Ok(p)
}

fn markers(name: &str) -> (String, String) {
    (
        format!("<!-- {name}:start -->"),
        format!("<!-- {name}:end -->"),
    )
}

/// Replace the marked block in place, or append it. Text outside the markers is untouched.
pub fn upsert_block(existing: &str, name: &str, body: &str) -> String {
    let (s, e) = markers(name);
    let block = format!("{s}\n{}\n{e}\n", body.trim_end());
    if let (Some(a), Some(b)) = (existing.find(&s), existing.find(&e)) {
        if a < b {
            let rest = &existing[b + e.len()..];
            return format!(
                "{}{block}{}",
                &existing[..a],
                rest.strip_prefix('\n').unwrap_or(rest)
            );
        }
    }
    match existing {
        "" => block,
        x if x.ends_with("\n\n") => format!("{x}{block}"),
        x if x.ends_with('\n') => format!("{x}\n{block}"),
        x => format!("{x}\n\n{block}"),
    }
}

/// Add a PostToolUse hook to `.claude/settings.json` that scans changed files after edits and
/// feeds failures back to the agent (exit 2). Idempotent.
// ponytail: serde_json sorts keys, so an existing settings.json is rewritten key-sorted.
fn merge_hook(existing: &str, command: &str) -> Result<String, String> {
    let mut v: Value = if existing.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(existing).map_err(|e| format!("settings.json: {e}"))?
    };
    let hook = format!("{command} scan --scope changed --headless --fail-on high >&2 || exit 2");
    let arr = v
        .pointer_mut("/hooks/PostToolUse")
        .and_then(Value::as_array_mut);
    let entry = json!({"matcher": "Edit|Write|MultiEdit",
        "hooks": [{"type": "command", "command": hook}]});
    match arr {
        Some(a) if a.iter().any(|e| e.to_string().contains(&hook)) => {}
        Some(a) => a.push(entry),
        None => {
            v.as_object_mut()
                .ok_or("settings.json is not an object")?
                .entry("hooks")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .ok_or("settings.json: hooks is not an object")?
                .insert("PostToolUse".into(), json!([entry]));
        }
    }
    Ok(serde_json::to_string_pretty(&v).unwrap() + "\n")
}

/// Write agent guidance under `root`. `agent: None` writes all of them. Returns paths written.
pub fn install(
    d: &dyn Doctor,
    root: &Path,
    agent: Option<Agent>,
    hooks: bool,
) -> Result<Vec<PathBuf>, String> {
    let (name, body) = (d.name().to_string(), d.skill_body());
    let io = |e: io::Error| e.to_string();
    let want = |a: &[Agent]| agent.is_none_or(|x| a.contains(&x));
    let mut out = vec![];
    if want(&[Agent::Claude]) {
        let t = format!(
            "---\nname: {name}\ndescription: Run `{}` to scan this project and fix what it reports.\n---\n\n{body}",
            d.command()
        );
        out.push(put(root, &format!(".claude/skills/{name}/SKILL.md"), &t).map_err(io)?);
    }
    if want(&[Agent::Cursor]) {
        let t = format!(
            "---\ndescription: How to run {name} and read its output\nalwaysApply: false\n---\n\n{body}"
        );
        out.push(put(root, &format!(".cursor/rules/{name}.mdc"), &t).map_err(io)?);
    }
    if want(&[Agent::Codex, Agent::Opencode]) {
        let cur = std::fs::read_to_string(root.join("AGENTS.md")).unwrap_or_default();
        out.push(put(root, "AGENTS.md", &upsert_block(&cur, &name, &body)).map_err(io)?);
    }
    if hooks {
        let rel = ".claude/settings.json";
        let cur = std::fs::read_to_string(root.join(rel)).unwrap_or_default();
        out.push(put(root, rel, &merge_hook(&cur, &d.command())?).map_err(io)?);
    }
    Ok(out)
}

/// Write the GitHub Actions workflow; returns its path.
pub fn ci_install(d: &dyn Doctor, root: &Path) -> Result<PathBuf, String> {
    put(
        root,
        &format!(".github/workflows/{}.yml", d.name()),
        &d.ci_workflow(),
    )
    .map_err(|e| e.to_string())
}
