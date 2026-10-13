use crate::check::{finding, Check, CheckResult, Ctx};
use crate::face::Face;
use crate::node::Node;
use crate::session::Command;
use crate::theme::Theme;
use doctor_core::{Envelope, Finding, Score, Severity};
use serde_json::{json, Map, Value};

/// Score model name used when a doctor opts out of scoring.
pub const UNSCORED: &str = "none";

/// Placeholder `Score` for envelopes with no score (the doctor/1 envelope requires the key).
/// Faces treat `model == UNSCORED` as "no score".
pub fn unscored() -> Score {
    Score {
        value: 0,
        label: "unscored".into(),
        model: UNSCORED.into(),
        coverage_gaps: 0,
    }
}

pub fn is_scored(s: &Score) -> bool {
    s.model != UNSCORED
}

/// What a tool's `collect` produced, before sorting, scoring and wrapping into the envelope.
#[derive(Debug, Default, Clone)]
pub struct Collected {
    pub findings: Vec<Finding>,
    /// Becomes `envelope.data` verbatim (the default `collect` fills the kit's own keys).
    pub data: Value,
    /// Coverage gaps (rules that could not be evaluated); for `score_collected`.
    pub gaps: usize,
}

/// Why a scan could not produce a report. `finish` (and the CLI) print `envelope` on stdout when
/// present, then `stderr` verbatim, then exit with `exit`. (Boxed: an envelope is large.)
#[derive(Debug, Clone)]
pub struct ScanFailure {
    pub exit: u8,
    /// Exact stderr text, including the trailing newline. The kit adds no prefix.
    pub stderr: String,
    pub envelope: Option<Box<Envelope>>,
}

impl ScanFailure {
    /// Exit 2 and `error: <msg>` on stderr (what the kit has always printed).
    pub fn new(msg: impl std::fmt::Display) -> Self {
        ScanFailure {
            exit: 2,
            stderr: format!("error: {msg}\n"),
            envelope: None,
        }
    }
    /// Exit 2 and exactly `msg` plus a newline on stderr: no `error: ` prefix.
    pub fn plain(msg: impl std::fmt::Display) -> Self {
        ScanFailure {
            exit: 2,
            stderr: format!("{msg}\n"),
            envelope: None,
        }
    }
    /// Also print this envelope on stdout (see [`failure_envelope`]).
    pub fn with_envelope(mut self, env: Envelope) -> Self {
        self.envelope = Some(Box::new(env));
        self
    }
    pub fn with_exit(mut self, exit: u8) -> Self {
        self.exit = exit;
        self
    }
}

impl std::fmt::Display for ScanFailure {
    /// The message without the `error: ` prefix or trailing newline.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let t = self.stderr.trim_end();
        f.write_str(t.strip_prefix("error: ").unwrap_or(t))
    }
}

impl From<String> for ScanFailure {
    fn from(s: String) -> Self {
        ScanFailure::new(s)
    }
}

/// Which `preflight` check failed (see [`Doctor::preflight_error`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreflightKind {
    /// The `--baseline` file could not be loaded.
    Baseline,
    /// `--sarif` and `--baseline` name the same file.
    SarifIsBaseline,
}

/// What the subject of a doctor is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TargetKind {
    /// A directory of files (the default): `[PATH]`, `--scope`, `--base`, `--config` on kit commands.
    #[default]
    Dir,
    /// Something else (a card, a radio, one capture file): the tool owns its subject; the kit walks
    /// no directory ([`Ctx::detached`]).
    Detached,
}

/// A doctor/1 envelope for a scan that failed: score 0, label `critical`, one coverage gap, no
/// findings, `data = extra_data + {"error": {"kind", "message"}}`. The score model is the doctor's own.
pub fn failure_envelope<D: Doctor + ?Sized>(
    d: &D,
    kind: &str,
    message: &str,
    extra_data: Value,
) -> Envelope {
    let probe = Ctx::detached(Default::default(), Severity::High);
    let model = d
        .score(&[], &probe)
        .map(|s| s.model)
        .unwrap_or_else(|| UNSCORED.into());
    let mut data = match extra_data {
        Value::Object(m) => m,
        Value::Null => Map::new(),
        other => {
            let mut m = Map::new();
            m.insert("details".into(), other);
            m
        }
    };
    data.insert("error".into(), json!({"kind": kind, "message": message}));
    let score = Score {
        value: 0,
        label: "critical".into(),
        model,
        coverage_gaps: 1,
    };
    Envelope::new(d.name(), d.version(), 2, score, vec![], Value::Object(data))
}

pub trait Doctor {
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    /// One-line description shown by `--help` (default: none).
    fn about(&self) -> &str {
        ""
    }
    /// Declared categories, in display order. Findings in other categories are dropped.
    fn categories(&self) -> Vec<&'static str>;
    /// The registry: explicit, no reflection.
    fn checks(&self) -> Vec<Box<dyn Check>>;
    fn theme(&self) -> Theme {
        Theme::default()
    }

    // Agentic hooks: every tool inherits `mcp`, `fix`, `install` and `ci install`;
    // override any of these to customize the text they emit.

    /// The command an agent or CI runs (default: the tool name).
    fn command(&self) -> String {
        self.name().to_string()
    }
    /// Prompt for a coding agent, from the selected findings of `env` (`fix`).
    fn fix_prompt(&self, env: &Envelope, findings: &[&Finding]) -> String {
        crate::fix::default_prompt(&self.command(), env, findings)
    }
    /// Markdown body of the agent skill, Cursor rule and AGENTS.md block (`install`).
    fn skill_body(&self) -> String {
        crate::install::default_skill(&self.command(), &self.categories())
    }
    /// The files `install` writes for one agent. Default: the skill / Cursor rule / AGENTS.md block
    /// built from [`Doctor::skill_body`]. Override for exact bodies or caller-owned markers.
    fn agent_files(&self, agent: crate::install::Agent) -> Vec<crate::install::AgentFile> {
        crate::install::default_agent_files(self, agent)
    }
    /// YAML steps (unindented, one `- ` list) that make the tool available in CI.
    fn ci_setup_steps(&self) -> String {
        format!(
            "- uses: dtolnay/rust-toolchain@stable\n- run: cargo install --locked {}",
            self.name()
        )
    }
    /// What `ci install` does with an existing workflow file (without `--force`). Default
    /// `RefuseDifferent` (identical content is a no-op); `Always` = replace (0.1 behaviour);
    /// `Never` = refuse any existing file.
    fn ci_overwrite(&self) -> crate::install::Overwrite {
        crate::install::Overwrite::RefuseDifferent
    }
    /// The whole GitHub Actions workflow written by `ci install`.
    fn ci_workflow(&self) -> String {
        crate::install::default_workflow(&self.command(), &self.ci_setup_steps())
    }

    /// Default additive model: 100 minus per-severity weights (from `doctor.toml`), floor 0.
    /// Override for a custom formula, or return `None` for no score at all.
    fn score(&self, findings: &[Finding], ctx: &Ctx) -> Option<Score> {
        let lost: u32 = findings
            .iter()
            .map(|f| ctx.config.score.of(f.severity))
            .sum();
        Some(Score::new(
            100u32.saturating_sub(lost) as u8,
            "doctor-kit/1",
            0,
        ))
    }

    // Interactive hooks (shell and explorer). Defaults give a category -> finding tree.

    /// The tree `ls`/`cd`/`cat` walk. Override to expose the tool's own nouns; start from
    /// [`crate::session::default_tree`] to keep the findings.
    fn tree(&self, env: &Envelope, ctx: &Ctx) -> Node {
        crate::session::default_tree(env, ctx)
    }
    /// Lazy children: called (once per scan) when the shell or explorer first enters or lists a node
    /// that has none, with the node's name path from the root. Return its children, or `None` for a
    /// leaf. Lets a tool build a big domain tree (image -> directory -> file) on demand instead of in
    /// [`Doctor::tree`]. `find` searches the nodes loaded so far.
    fn expand(
        &self,
        _env: &Envelope,
        _ctx: &Ctx,
        _path: &[String],
        _node: &Node,
    ) -> Option<Vec<Node>> {
        None
    }
    /// Typed decode of a node for `decode`/`cat` and the detail pane (`None` = generic view).
    fn decode(&self, _node: &Node) -> Option<String> {
        None
    }
    /// `raw <bytes>` passthrough to the target (APDU, HCI, ...). Unsupported by default.
    fn raw(&self, _bytes: &[u8]) -> Result<String, String> {
        Err(format!("`raw` is not supported by {}", self.name()))
    }
    /// `do <action>`. When `apply` is false (a gated action without `--yes`), do nothing and
    /// return what would happen, with the exact bytes where there are any.
    fn act(
        &self,
        _ctx: &Ctx,
        node: &Node,
        action: &str,
        _args: &[String],
        _apply: bool,
    ) -> Result<String, String> {
        Err(format!("no handler for `{action}` on {}", node.name))
    }
    /// Domain commands on top of the shared verbs. Shared vocabulary names tools should reuse
    /// where the concept exists: `verify`, `connect`, `read`, `write`.
    fn commands(&self) -> Vec<Command> {
        vec![]
    }

    // Scan pipeline. Override `collect` to produce findings and `data` your own way (non-file
    // subjects, custom envelope data); the kit sorts, scores and wraps them. `scan` is the
    // infallible form for callers that cannot handle failure; do not override it, override
    // `collect` or `try_scan`.

    /// What this doctor looks at. `Detached` = no directory walk.
    fn target(&self) -> TargetKind {
        TargetKind::Dir
    }
    /// `false`: the default `collect` does not add `extra["check"]` to findings.
    fn tag_checks(&self) -> bool {
        true
    }
    /// Severity used for `--fail-on` when the flag is absent.
    fn default_fail_on(&self) -> Severity {
        Severity::High
    }
    /// Whether a finding reports something that could not be evaluated (a coverage gap).
    fn is_gap(&self, _f: &Finding) -> bool {
        false
    }
    /// Largest baseline file `--baseline` reads.
    fn baseline_max_bytes(&self) -> usize {
        16 << 20
    }
    /// Message for a baseline that could not be loaded. Default: the kit's wording; override to
    /// keep a tool's existing text byte for byte.
    fn baseline_message(&self, e: &crate::baseline::BaselineError) -> String {
        e.to_string()
    }
    /// Read `--baseline`. The default is the kit's loader ([`crate::baseline::load_for`]: a doctor/1
    /// envelope or a bare array, size cap from [`Doctor::baseline_max_bytes`]). A tool that must keep
    /// its exact parsing (which shapes it accepts, which entries it skips, no size cap) overrides
    /// this: it gets the path and reads the bytes itself. It returns the fingerprints to compare
    /// and the parsed document for [`Doctor::baseline_hook`].
    fn load_baseline(&self, path: &std::path::Path) -> Result<crate::baseline::Saved, ScanFailure> {
        crate::baseline::load_for(self, path)
    }
    /// How a failed `preflight` check is reported: default exit 2 and `error: <msg>` on stderr.
    /// Override for a different prefix (none at all: `ScanFailure::plain`), exit code or text.
    fn preflight_error(&self, _kind: PreflightKind, msg: String) -> ScanFailure {
        ScanFailure::new(msg)
    }
    /// After the baseline is applied: tool-specific refusals (dialect, truncation) or extra data
    /// (a diff). `saved` is the whole baseline document.
    fn baseline_hook(&self, _saved: &Value, _env: &mut Envelope) -> Result<(), ScanFailure> {
        Ok(())
    }
    /// The envelope printed when the run is interrupted (Ctrl-C), given the kit's default
    /// ([`failure_envelope`] of kind `interrupted`, `exit_code` 130). Return `Some(own)` to change its
    /// shape (data keys, score), `None` to keep the default. The process still exits 130.
    fn interrupted_envelope(&self, _default: &Envelope) -> Option<Envelope> {
        None
    }
    /// Serialisation of the envelope on stdout (`--json`, MCP `scan`). Default: pretty, trailing
    /// newline. The kit never changes a finding's fingerprint, whatever the serialisation.
    fn render_json(&self, env: &Envelope) -> String {
        serde_json::to_string_pretty(env).unwrap() + "\n"
    }
    /// Notes shown before everything else in the human report (faces, `shell`, `explore`): the
    /// result is incomplete, a walk was truncated, an archive stopped early. Default: an
    /// `INCOMPLETE` banner when `score.coverage_gaps > 0`, plus `data.notes` (strings or
    /// `{"level", "text"}`). Override for full control (no banner, own wording). Notes are text for
    /// people: never part of `--json` or SARIF, and sanitized when shown.
    fn notes(&self, env: &Envelope) -> Vec<crate::face::Note> {
        crate::face::notes_of(env)
    }
    /// The report `finish` prints for people (not `--json`/`--score`). `None` (default): the kit
    /// face. A tool with its own report (a per-image audit, a file list) returns it here and still
    /// gets the baseline, SARIF and exit pipeline. `env` is already sanitized; `face` and `theme`
    /// are what the user asked for.
    fn render_human(&self, _env: &Envelope, _face: &Face, _theme: &Theme) -> Option<String> {
        None
    }
    /// The tools `mcp` offers. `None` (default): the built-in `scan` and `fix`. `Some(..)`
    /// REPLACES them (so `fix` is not reachable unless listed). The kit owns the stdio loop.
    fn mcp_tools(&self) -> Option<Vec<crate::mcp::McpTool>> {
        None
    }
    /// Wire texts of the MCP server, for tools that must stay byte-identical to an older server.
    fn mcp_wire(&self) -> crate::mcp::McpWire {
        crate::mcp::McpWire::default()
    }
    /// More MCP wire texts (invalid-request messages, unknown tool, tool error wrapper, serverInfo).
    fn mcp_texts(&self) -> crate::mcp::McpTexts {
        crate::mcp::McpTexts::default()
    }
    /// The SARIF 2.1.0 document written by `--sarif`, as text. Default: the kit's document, pretty
    /// with a trailing newline. A tool with its own locations or serialisation builds its own
    /// (e.g. with `doctor_core::sarif::document`).
    fn render_sarif(&self, env: &Envelope) -> String {
        serde_json::to_string_pretty(&crate::session::to_sarif(env)).unwrap() + "\n"
    }
    /// Score from a collected result (gives access to `gaps`). Default: [`Doctor::score`].
    fn score_collected(&self, c: &Collected, ctx: &Ctx) -> Option<Score> {
        self.score(&c.findings, ctx)
    }

    /// Default: run applicable, enabled checks; `data` = categories, checks_run, skipped, advice,
    /// scope, files.
    fn collect(&self, ctx: &Ctx) -> Result<Collected, ScanFailure> {
        use crate::progress::ProgressEvent;
        let cats = self.categories();
        let (mut findings, mut advice, mut skipped) = (Vec::new(), Map::new(), Vec::new());
        let (mut ran, mut panicked) = (0, Vec::new());
        let checks: Vec<Box<dyn Check>> = self.checks();
        let total = checks.len();
        let progress: &dyn crate::progress::Progress = ctx.progress.as_progress();
        for (i, c) in checks.into_iter().enumerate() {
            let index = i + 1;
            if !ctx.config.check_enabled(c.id()) || !c.applies(ctx) {
                progress.event(&ProgressEvent::CheckSkipped {
                    id: c.id().to_string(),
                    index,
                    total,
                });
                skipped.push(c.id());
                continue;
            }
            progress.event(&ProgressEvent::CheckStarted {
                id: c.id().to_string(),
                index,
                total,
            });
            crate::interrupt::check(self)?;
            ran += 1;
            let emitted_before = findings.len();
            // A panicking check becomes a high-severity finding; the scan continues.
            let r = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.run(ctx))) {
                Ok(r) => r,
                Err(p) => {
                    let why = p
                        .downcast_ref::<&str>()
                        .map(|s| s.to_string())
                        .or_else(|| p.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "unknown panic".into());
                    panicked.push(c.id());
                    let mut f = finding(
                        &format!("{}.panic", c.id()),
                        c.category(),
                        doctor_core::Severity::High,
                        &format!("check `{}` panicked: {why}", c.id()),
                        std::path::Path::new("."),
                        None,
                    );
                    f.remedy = Some("This is a bug in the check; report it.".into());
                    CheckResult::findings(vec![f])
                }
            };
            if let Some(a) = r.advice {
                advice.insert(c.id().to_string(), Value::String(a));
            }
            for mut f in r.findings {
                if !cats.contains(&f.category.as_str()) {
                    continue;
                }
                if self.tag_checks() {
                    f.extra.insert("check".into(), c.id().into());
                }
                let emitted = ProgressEvent::Finding {
                    id: f.id.clone(),
                    severity: f.severity,
                    message: f.message.clone(),
                };
                progress.event(&emitted);
                findings.push(f);
            }
            let emitted = findings.len() - emitted_before;
            progress.event(&ProgressEvent::CheckFinished {
                id: c.id().to_string(),
                findings: emitted,
            });
        }
        progress.event(&ProgressEvent::Done {
            findings: findings.len(),
        });
        let mut data = json!({
            "categories": cats, "checks_run": ran, "skipped": skipped,
            "advice": advice, "scope": format!("{:?}", ctx.scope).to_lowercase(),
            "files": ctx.files.len(),
        });
        if !panicked.is_empty() {
            data["panicked"] = json!(panicked); // only present when a check panicked
        }
        Ok(Collected {
            findings,
            data,
            gaps: 0,
        })
    }

    /// `collect`, sort, score, envelope. `exit_code` = any finding at or above `ctx.fail_on`.
    fn try_scan(&self, ctx: &Ctx) -> Result<Envelope, ScanFailure> {
        let mut c = self.collect(ctx)?;
        Finding::sort(&mut c.findings);
        let score = self.score_collected(&c, ctx).unwrap_or_else(unscored);
        let exit = c.findings.iter().any(|f| f.severity >= ctx.fail_on) as u8;
        Ok(Envelope::new(
            self.name(),
            self.version(),
            exit,
            score,
            c.findings,
            c.data,
        ))
    }

    /// Infallible `try_scan`: a failure becomes a [`failure_envelope`] of kind `scan-failed`.
    fn scan(&self, ctx: &Ctx) -> Envelope {
        self.try_scan(ctx).unwrap_or_else(|f| {
            let msg = f.to_string();
            f.envelope
                .map(|b| *b)
                .unwrap_or_else(|| failure_envelope(self, "scan-failed", &msg, Value::Null))
        })
    }
}

/// Count of findings per severity, most severe first.
pub fn severity_counts(env: &Envelope) -> Vec<(Severity, usize)> {
    Severity::ALL
        .iter()
        .rev()
        .map(|s| (*s, env.findings.iter().filter(|f| f.severity == *s).count()))
        .collect()
}
