//! A doctor whose subject is not a directory: it inspects a named "subject" given on the command
//! line. It shows the extension points of doctor-kit: a replaced `scan` with its own flags that
//! still gets the kit's baseline/SARIF/exit pipeline, a command of its own, a global argument that
//! reaches the doctor's constructor, compact JSON, a custom exit code for usage errors.
use clap::{Arg, ArgAction, ArgMatches, Command};
use doctor_core::{Envelope, Finding, Location, LocationKind, Score, Severity};
use doctor_kit::mcp::{exec_self, ExecOpts};
use doctor_kit::{
    failure_envelope, finish, preflight, Check, Collected, Ctx, Doctor, ExtCommand, Extensions,
    Meta, OutputArgs, ScanFailure, TargetKind,
};
use doctor_kit::{McpTool, McpWire, Protocol};
use serde_json::json;

struct Ext {
    profile: String,
    subject: String,
}

fn finding(id: &str, sev: Severity, fp: &str) -> Finding {
    Finding {
        id: id.into(),
        fingerprint: fp.into(),
        severity: sev,
        confidence: None,
        category: "subject".into(),
        message: format!("{id} on the subject"),
        location: Location::new(LocationKind::File, "subject"),
        evidence: vec![],
        remedy: Some("fix it".into()),
        baseline_state: None,
        extra: Default::default(),
    }
}

impl Doctor for Ext {
    fn name(&self) -> &str {
        "ext-doctor"
    }
    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }
    fn about(&self) -> &str {
        "Toy detached doctor"
    }
    fn categories(&self) -> Vec<&'static str> {
        vec!["subject"]
    }
    fn checks(&self) -> Vec<Box<dyn Check>> {
        vec![]
    }
    fn target(&self) -> TargetKind {
        TargetKind::Detached
    }
    fn default_fail_on(&self) -> Severity {
        Severity::Critical
    }
    fn score(&self, f: &[Finding], _: &Ctx) -> Option<Score> {
        Some(Score::new(100 - 10 * f.len().min(10) as u8, "ext/1", 0))
    }
    /// With `EXT_DOCTOR_MCP` set: own tools that re-run this binary (the kit's `scan`/`fix` are
    /// replaced), and the wire texts of an older server.
    fn mcp_tools(&self) -> Option<Vec<McpTool>> {
        std::env::var_os("EXT_DOCTOR_MCP")?;
        let exec = |args: Vec<String>, o: ExecOpts| exec_self(&args, &o);
        Some(vec![
            McpTool {
                name: "inspect".into(),
                description: "Inspect a subject".into(),
                schema: json!({"type": "object", "properties": {"subject": {"type": "string"}}}),
                call: Box::new(move |a| {
                    let s = a.get("subject").and_then(|v| v.as_str()).unwrap_or("ok");
                    if s.starts_with('-') {
                        return Err(format!("{{\"error\":\"bad subject {s}\"}}"));
                    }
                    let args = ["scan", "--subject", s, "--json"]
                        .map(String::from)
                        .to_vec();
                    exec(
                        args,
                        ExecOpts {
                            ok_codes: vec![0, 1, 3],
                            ..Default::default()
                        },
                    )
                }),
            },
            McpTool {
                name: "hang".into(),
                description: "Never returns".into(),
                schema: json!({"type": "object"}),
                call: Box::new(move |_| {
                    exec(
                        vec!["wait".into()],
                        ExecOpts {
                            timeout: Some(std::time::Duration::from_millis(300)),
                            ..Default::default()
                        },
                    )
                }),
            },
            McpTool {
                name: "flood".into(),
                description: "Too much output".into(),
                schema: json!({"type": "object"}),
                call: Box::new(move |_| {
                    exec(
                        vec!["flood".into()],
                        ExecOpts {
                            max_bytes: 1000,
                            ..Default::default()
                        },
                    )
                }),
            },
        ])
    }
    fn mcp_wire(&self) -> McpWire {
        if std::env::var_os("EXT_DOCTOR_MCP").is_none() {
            return McpWire::default();
        }
        McpWire {
            protocol: Protocol::EchoClient,
            parse_error: "parse error".into(),
            method_not_found: "method not found".into(),
            blank_line_is_parse_error: true,
        }
    }
    fn render_json(&self, env: &Envelope) -> String {
        serde_json::to_string(&env.to_value()).unwrap() + "\n"
    }
    fn collect(&self, _: &Ctx) -> Result<Collected, ScanFailure> {
        let mut findings = vec![];
        match self.subject.as_str() {
            "fail" => {
                let e = failure_envelope(
                    self,
                    "unreachable",
                    "subject unreachable",
                    json!({"touched": false}),
                );
                return Err(ScanFailure::new("subject unreachable").with_envelope(e));
            }
            "bad" => findings.push(finding("ext.bad", Severity::Critical, "fp-bad")),
            "meh" => findings.push(finding("ext.meh", Severity::Low, "fp-meh")),
            _ => {}
        }
        Ok(Collected {
            findings,
            data: json!({"subject": self.subject, "profile": self.profile}),
            gaps: 0,
        })
    }
}

fn scan_cmd() -> Command {
    Command::new("scan")
        .about("Inspect a subject")
        .arg(Arg::new("subject").long("subject").default_value("ok"))
        .args(OutputArgs::args_with(Severity::Critical))
}

fn scan(d: &Ext, m: &ArgMatches) -> Result<u8, String> {
    let o = OutputArgs::from_matches(m, d);
    let pre = match preflight(d, &o) {
        Ok(p) => p,
        Err(f) => return Ok(doctor_kit::output::report_failure(d, &f)),
    };
    let me = Ext {
        profile: d.profile.clone(),
        subject: m.get_one::<String>("subject").cloned().unwrap_or_default(),
    };
    Ok(finish(
        &me,
        pre,
        me.try_scan(&Ctx::detached(Default::default(), o.fail_on)),
        &o,
    ))
}

fn probe(d: &Ext, m: &ArgMatches) -> Result<u8, String> {
    println!("probing with profile {}", d.profile);
    Ok(*m.get_one::<u8>("code").unwrap())
}

/// Waits until interrupted, like a long scan polling at its checkpoints.
fn wait(d: &Ext, _: &ArgMatches) -> Result<u8, String> {
    doctor_kit::interrupt::install();
    println!("ready");
    let _ = std::io::Write::flush(&mut std::io::stdout());
    loop {
        if let Err(f) = doctor_kit::interrupt::check(d) {
            return Ok(doctor_kit::output::report_failure(d, &f));
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn main() -> std::process::ExitCode {
    let ext = Extensions {
        commands: vec![
            ExtCommand {
                command: scan_cmd(),
                run: scan,
            },
            ExtCommand {
                command: Command::new("flood").hide(true),
                run: |_, _| {
                    print!("{}", "x".repeat(1 << 20));
                    Ok(0)
                },
            },
            ExtCommand {
                command: Command::new("wait").about("Wait for Ctrl-C"),
                run: wait,
            },
            ExtCommand {
                command: Command::new("probe").about("Probe the subject").arg(
                    Arg::new("code")
                        .long("code")
                        .value_parser(clap::value_parser!(u8))
                        .default_value("0"),
                ),
                run: probe,
            },
        ],
        global_args: vec![Arg::new("profile").long("profile").action(ArgAction::Set)],
        usage_exit: |e, argv| {
            if e.exit_code() == 0 || argv.iter().any(|a| a == "scan") {
                e.exit_code() as u8
            } else {
                129
            }
        },
    };
    let probe_doctor = Ext {
        profile: String::new(),
        subject: String::new(),
    };
    doctor_kit::run_with(Meta::of(&probe_doctor), ext, |m| {
        Ok(Ext {
            profile: m
                .get_one::<String>("profile")
                .cloned()
                .unwrap_or_else(|| "default".into()),
            subject: String::new(),
        })
    })
}
