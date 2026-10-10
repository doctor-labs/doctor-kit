//! 0.2.x must stay source-compatible with code written against 0.2.1 (see compat_020.rs for the
//! rule). 0.2.1 shapes only: if this stops compiling, a patch release broke a user.
use doctor_kit::mcp::{exec_self_fmt, ExecOpts};
use doctor_kit::{finish_with, Check, Doctor, FinishOpts, McpTexts, PreflightKind, ScanFailure};

struct D;
impl Doctor for D {
    fn name(&self) -> &str {
        "c"
    }
    fn version(&self) -> &str {
        "1"
    }
    fn categories(&self) -> Vec<&'static str> {
        vec![]
    }
    fn checks(&self) -> Vec<Box<dyn Check>> {
        vec![]
    }
    // The 0.2.1 hooks, with their 0.2.1 signatures.
    fn render_human(
        &self,
        _: &doctor_core::Envelope,
        _: &doctor_kit::Face,
        _: &doctor_kit::Theme,
    ) -> Option<String> {
        None
    }
    fn load_baseline(
        &self,
        p: &std::path::Path,
    ) -> Result<doctor_kit::baseline::Saved, ScanFailure> {
        doctor_kit::baseline::load_for(self, p)
    }
    fn preflight_error(&self, _: PreflightKind, msg: String) -> ScanFailure {
        ScanFailure::plain(msg)
    }
    fn mcp_texts(&self) -> McpTexts {
        McpTexts {
            unknown_tool: "no {t}".into(),
            ..McpTexts::default()
        }
    }
    fn ci_overwrite(&self) -> doctor_kit::install::Overwrite {
        doctor_kit::install::Overwrite::Never
    }
    fn expand(
        &self,
        _: &doctor_core::Envelope,
        _: &doctor_kit::Ctx,
        _: &[String],
        _: &doctor_kit::Node,
    ) -> Option<Vec<doctor_kit::Node>> {
        None
    }
}

#[test]
fn zero_two_one_shapes_compile_and_behave() {
    let _ = D.mcp_texts();
    let _ = exec_self_fmt(&["--list".to_string()], &ExecOpts::default(), None);
    let _ = FinishOpts { never_fail: true };
    let _ = finish_with; // 0.2.1 signature: (d, pre, result, &OutputArgs, &FinishOpts) -> u8
    assert_eq!(ScanFailure::plain("x").stderr, "x\n");
}
