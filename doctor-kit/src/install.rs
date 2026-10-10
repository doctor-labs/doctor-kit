//! `install` (agent skill + Cursor rule + AGENTS.md block) and `ci install` (GitHub Actions).
//! Templates are defaults; a `Doctor` overrides `skill_body` / `ci_workflow`.
use crate::doctor::Doctor;
use serde_json::{json, Value};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

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

/// What an existing file is allowed to lose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overwrite {
    /// Replace it (what `install` has always done).
    Always,
    /// Replace only identical content (a no-op); refuse a file that differs. `--force` = `Always`.
    RefuseDifferent,
    /// Refuse any existing file, identical or not.
    Never,
}

/// Write `text` to `root/rel...` without following symlinks: a symlinked directory on the path
/// or a symlinked target is refused (a hostile repository could otherwise aim `.claude` or
/// `.github` at `~`). Directories are created one level at a time; the file is written to a fresh
/// temp file in its directory and renamed into place, so a link swapped in meanwhile is replaced,
/// not followed. `root` itself may be a symlink (the user named it).
pub fn put_safe(
    root: &Path,
    rel: &[&str],
    text: &str,
    overwrite: Overwrite,
) -> Result<PathBuf, String> {
    let (name, dirs) = rel.split_last().ok_or("empty path")?;
    let mut p = root.to_path_buf();
    for part in dirs {
        safe_component(part)?;
        p.push(part);
        match std::fs::symlink_metadata(&p) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(format!("refusing to write through symlink {}", p.display()))
            }
            Ok(m) if m.is_dir() => {}
            Ok(_) => return Err(format!("{} is not a directory", p.display())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                std::fs::create_dir(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            }
            Err(e) => return Err(format!("{}: {e}", p.display())),
        }
    }
    safe_component(name)?;
    p.push(name);
    match std::fs::symlink_metadata(&p) {
        Ok(m) if m.file_type().is_symlink() => {
            return Err(format!("refusing to write through symlink {}", p.display()))
        }
        Ok(m) if !m.is_file() => return Err(format!("{} is not a file", p.display())),
        Ok(_) => match overwrite {
            Overwrite::Always => {}
            Overwrite::Never => {
                return Err(format!(
                    "{} already exists; use --force to overwrite",
                    p.display()
                ))
            }
            Overwrite::RefuseDifferent => {
                if std::fs::read_to_string(&p).is_ok_and(|cur| cur == text) {
                    return Ok(p);
                }
                return Err(format!(
                    "{} exists and differs; use --force to overwrite",
                    p.display()
                ));
            }
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("{}: {e}", p.display())),
    }
    let tmp = p.with_file_name(format!(".{name}.tmp{}", std::process::id()));
    let res = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .and_then(|mut f| f.write_all(text.as_bytes()))
        .and_then(|_| std::fs::rename(&tmp, &p));
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("{}: {e}", p.display()));
    }
    Ok(p)
}

fn safe_component(c: &str) -> Result<(), String> {
    let ok = !c.is_empty()
        && matches!(Path::new(c).components().next(), Some(Component::Normal(_)))
        && Path::new(c).components().count() == 1;
    ok.then_some(())
        .ok_or_else(|| format!("unsafe path component `{c}`"))
}

/// Read a file for a read-modify-write, refusing a symlink. Missing = empty.
fn read_safe(p: &Path) -> Result<String, String> {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.file_type().is_symlink() => {
            Err(format!("refusing to read through symlink {}", p.display()))
        }
        Ok(_) => std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("{}: {e}", p.display())),
    }
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
    // The name-based form has always trimmed trailing whitespace of the body.
    upsert_block_with(existing, &s, &e, body.trim_end())
}

/// [`upsert_block`] with caller-owned marker lines (the whole `begin` and `end` strings) and a
/// byte-exact body: nothing is trimmed, so a body that ends in a blank line keeps it before the end
/// marker. A newline is added before the end marker only when the body does not end with one.
pub fn upsert_block_with(existing: &str, begin: &str, end: &str, body: &str) -> String {
    let (s, e) = (begin, end);
    let nl = if body.ends_with('\n') { "" } else { "\n" };
    let block = format!("{s}\n{body}{nl}{e}\n");
    if let (Some(a), Some(b)) = (existing.find(s), existing.find(e)) {
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

/// How a file of [`AgentFile`] reaches disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileMode {
    /// The whole file is `body`.
    Write,
    /// `body` is a block between these marker lines inside a file that may hold other text.
    Block { begin: String, end: String },
}

/// One file `install` writes, relative to the repository root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentFile {
    pub path: PathBuf,
    pub body: String,
    pub mode: FileMode,
}

/// The kit's files for one agent (what [`Doctor::agent_files`] returns by default). `Opencode`
/// shares Codex's `AGENTS.md`.
pub fn default_agent_files<D: Doctor + ?Sized>(d: &D, agent: Agent) -> Vec<AgentFile> {
    let (name, body) = (d.name().to_string(), d.skill_body());
    match agent {
        Agent::Claude => vec![AgentFile {
            path: format!(".claude/skills/{name}/SKILL.md").into(),
            body: format!(
                "---\nname: {name}\ndescription: Run `{}` to scan this project and fix what it reports.\n---\n\n{body}",
                d.command()
            ),
            mode: FileMode::Write,
        }],
        Agent::Cursor => vec![AgentFile {
            path: format!(".cursor/rules/{name}.mdc").into(),
            body: format!(
                "---\ndescription: How to run {name} and read its output\nalwaysApply: false\n---\n\n{body}"
            ),
            mode: FileMode::Write,
        }],
        Agent::Codex | Agent::Opencode => {
            let (begin, end) = markers(&name);
            vec![AgentFile {
                path: "AGENTS.md".into(),
                // Trailing whitespace trimmed, as the kit's block has always been; an
                // `agent_files` override gets its body written exactly.
                body: body.trim_end().to_string(),
                mode: FileMode::Block { begin, end },
            }]
        }
    }
}

/// What `install` would write: every agent's files, or one agent's.
pub fn plan(d: &dyn Doctor, agent: Option<Agent>) -> Vec<AgentFile> {
    match agent {
        Some(a) => d.agent_files(a),
        None => [Agent::Claude, Agent::Cursor, Agent::Codex]
            .into_iter()
            .flat_map(|a| d.agent_files(a))
            .collect(),
    }
}

/// The text a block file gets when it does not exist yet (for `--print-only`).
pub fn rendered(f: &AgentFile) -> String {
    match &f.mode {
        FileMode::Write => f.body.clone(),
        FileMode::Block { begin, end } => upsert_block_with("", begin, end, &f.body),
    }
}

/// Write `files` under `root` with [`put_safe`]; a `Block` file is merged into what is there.
/// Returns the paths written.
pub fn write(
    root: &Path,
    files: &[AgentFile],
    overwrite: Overwrite,
) -> Result<Vec<PathBuf>, String> {
    let mut out = vec![];
    for f in files {
        let rel: Vec<String> = f
            .path
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let rel: Vec<&str> = rel.iter().map(String::as_str).collect();
        let text = match &f.mode {
            FileMode::Write => f.body.clone(),
            FileMode::Block { begin, end } => {
                let cur = read_safe(&root.join(&f.path))?;
                upsert_block_with(&cur, begin, end, &f.body)
            }
        };
        let ow = if matches!(f.mode, FileMode::Block { .. }) {
            Overwrite::Always
        } else {
            overwrite
        };
        out.push(put_safe(root, &rel, &text, ow)?);
    }
    Ok(out)
}

/// Add a PostToolUse hook to `.claude/settings.json` that scans changed files after edits and
/// feeds failures back to the agent (exit 2). Idempotent.
// ponytail: serde_json sorts keys, so an existing settings.json is rewritten key-sorted.
/// The `.claude/settings.json` text with the kit's PostToolUse hook merged into `existing`.
pub fn hook_settings(existing: &str, command: &str) -> Result<String, String> {
    merge_hook(existing, command)
}

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
    let mut out = write(root, &plan(d, agent), Overwrite::Always)?;
    if hooks {
        let cur = read_safe(&root.join(".claude/settings.json"))?;
        out.push(put_safe(
            root,
            &[".claude", "settings.json"],
            &merge_hook(&cur, &d.command())?,
            Overwrite::Always,
        )?);
    }
    Ok(out)
}

/// Write the GitHub Actions workflow; returns its path. Refuses to replace a workflow whose
/// content differs (see [`ci_install_with`]).
pub fn ci_install(d: &dyn Doctor, root: &Path) -> Result<PathBuf, String> {
    ci_install_with(d, root, d.ci_overwrite())
}

pub fn ci_install_with(
    d: &dyn Doctor,
    root: &Path,
    overwrite: Overwrite,
) -> Result<PathBuf, String> {
    put_safe(
        root,
        &[".github", "workflows", &format!("{}.yml", d.name())],
        &d.ci_workflow(),
        overwrite,
    )
}
