use crate::check::{Check, Ctx};
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

pub trait Doctor {
    fn name(&self) -> &str;
    fn version(&self) -> &str;
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
    /// YAML steps (unindented, one `- ` list) that make the tool available in CI.
    fn ci_setup_steps(&self) -> String {
        format!(
            "- uses: dtolnay/rust-toolchain@stable\n- run: cargo install --locked {}",
            self.name()
        )
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

    /// Run applicable, enabled checks and assemble the doctor/1 envelope.
    fn scan(&self, ctx: &Ctx) -> Envelope {
        let cats = self.categories();
        let (mut findings, mut advice, mut skipped) = (Vec::new(), Map::new(), Vec::new());
        let mut ran = 0;
        for c in self.checks() {
            if !ctx.config.check_enabled(c.id()) || !c.applies(ctx) {
                skipped.push(c.id());
                continue;
            }
            ran += 1;
            let r = c.run(ctx);
            if let Some(a) = r.advice {
                advice.insert(c.id().to_string(), Value::String(a));
            }
            for mut f in r.findings {
                if !cats.contains(&f.category.as_str()) {
                    continue;
                }
                f.extra.insert("check".into(), c.id().into());
                findings.push(f);
            }
        }
        Finding::sort(&mut findings);
        let score = self.score(&findings, ctx).unwrap_or_else(unscored);
        let exit = findings.iter().any(|f| f.severity >= ctx.fail_on) as u8;
        let data = json!({
            "categories": cats, "checks_run": ran, "skipped": skipped,
            "advice": advice, "scope": format!("{:?}", ctx.scope).to_lowercase(),
            "files": ctx.files.len(),
        });
        Envelope::new(self.name(), self.version(), exit, score, findings, data)
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
