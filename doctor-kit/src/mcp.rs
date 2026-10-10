//! `mcp`: a Model Context Protocol server, JSON-RPC 2.0 over stdio, no extra crates.
//! Tools run in-process through the same code as the CLI, so `scan` returns the doctor/1
//! envelope byte-identical to `scan --json` stdout.
use crate::cli::{ctx_from, Opts};
use crate::doctor::Doctor;
use crate::fix;
use serde_json::{json, Value};
use std::io::{self, BufRead, Read, Write};
use std::time::Duration;

/// A tool handler: `Ok(text)` is the result, `Err(text)` a result with `isError: true`.
pub type ToolFn = Box<dyn Fn(&Value) -> Result<String, String> + Send + Sync>;

/// A tool a doctor offers over MCP instead of the built-in `scan` and `fix`.
pub struct McpTool {
    pub name: String,
    pub description: String,
    /// The `inputSchema` (JSON Schema object).
    pub schema: Value,
    /// `Ok(text)` is the result; `Err(text)` is a result with `isError: true`, text verbatim.
    pub call: ToolFn,
}

/// What `initialize` answers for `protocolVersion`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Protocol {
    Fixed(String),
    /// The client's own `params.protocolVersion` (the default string when it sent none).
    EchoClient,
}

/// The few wire texts that differ between existing MCP servers, so a tool moving onto the kit can
/// stay byte-identical. The defaults are the kit's 0.1 behaviour.
#[derive(Debug, Clone)]
pub struct McpWire {
    pub protocol: Protocol,
    /// Message of a JSON parse error; `{e}` is the parser's text.
    pub parse_error: String,
    /// Message of an unknown method; `{m}` is the method name.
    pub method_not_found: String,
    /// `false`: blank lines are ignored. `true`: a blank line is a parse error like any bad JSON.
    pub blank_line_is_parse_error: bool,
}

/// More wire texts, for tools that must keep an older server's exact answers. Separate from
/// [`McpWire`] so code that builds `McpWire` literally keeps compiling; build this one with
/// `McpTexts { field: .., ..McpTexts::default() }`.
#[derive(Debug, Clone)]
pub struct McpTexts {
    /// Message of a request that is not a JSON object (code -32600).
    pub invalid_request: String,
    /// Message of a request whose `id` is not a string or integer (code -32600).
    pub invalid_request_id: String,
    /// Message of a request with no `method` (code -32600).
    pub invalid_request_method: String,
    /// Result text of a call to a tool that does not exist; `{t}` is the tool name.
    pub unknown_tool: String,
    /// Wraps the `Err` text of a tool before it is sent (`isError: true`); `{e}` is that text.
    pub tool_error: String,
    /// `serverInfo` (name, version) in the `initialize` answer; `None` = the doctor's own.
    pub server_info: Option<(String, String)>,
}

impl Default for McpTexts {
    fn default() -> Self {
        McpTexts {
            invalid_request: "invalid request".into(),
            invalid_request_id: "invalid request: bad id".into(),
            invalid_request_method: "invalid request: no method".into(),
            unknown_tool: "unknown tool {t}".into(),
            tool_error: "{e}".into(),
            server_info: None,
        }
    }
}

pub const DEFAULT_PROTOCOL: &str = "2024-11-05";

impl Default for McpWire {
    fn default() -> Self {
        McpWire {
            protocol: Protocol::Fixed(DEFAULT_PROTOCOL.into()),
            parse_error: "parse error: {e}".into(),
            method_not_found: "unknown method {m}".into(),
            blank_line_is_parse_error: false,
        }
    }
}

/// Fill `{key}` in `template`, plus `{key_json}` (a JSON string literal, quotes included) and
/// `{key_esc}` (JSON-escaped, no quotes) so a name containing quotes still yields valid JSON text.
pub fn fill(template: &str, key: &str, value: &str) -> String {
    let json = serde_json::to_string(value).unwrap_or_default();
    let esc = json[1..json.len().saturating_sub(1)].to_string();
    let (plain, j, e) = (
        format!("{{{key}}}"),
        format!("{{{key}_json}}"),
        format!("{{{key}_esc}}"),
    );
    // One pass, so a value that itself contains `{t}` is not substituted again.
    let (mut out, mut rest) = (String::new(), template);
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        if let Some(r) = rest.strip_prefix(j.as_str()) {
            out.push_str(&json);
            rest = r;
        } else if let Some(r) = rest.strip_prefix(e.as_str()) {
            out.push_str(&esc);
            rest = r;
        } else if let Some(r) = rest.strip_prefix(plain.as_str()) {
            out.push_str(value);
            rest = r;
        } else {
            out.push('{');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

/// Limits for [`exec_self`].
#[derive(Debug, Clone)]
pub struct ExecOpts {
    /// Exit codes whose stdout is a result (e.g. `[0, 1, 3]` for a scan whose findings exit 1/3).
    pub ok_codes: Vec<i32>,
    pub timeout: Option<Duration>,
    /// Largest stdout (and stderr) kept; more kills the child and is an error.
    pub max_bytes: usize,
}

impl Default for ExecOpts {
    fn default() -> Self {
        ExecOpts {
            ok_codes: vec![0],
            timeout: None,
            max_bytes: 16 << 20,
        }
    }
}

/// Why [`exec_self_with`] did not produce a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecFailureKind {
    /// The executable could not be found or started.
    Spawn,
    /// Exited with a code that is not in `ok_codes`.
    Exit,
    /// Killed by a signal it did not handle (`signal` is set when known).
    Signal,
    /// Killed after `ExecOpts::timeout`.
    Timeout,
    /// Killed because stdout or stderr passed `ExecOpts::max_bytes`.
    OutputLimit,
}

/// Everything known about a failed [`exec_self_with`] run, for a tool to word its error.
#[derive(Debug, Clone)]
pub struct ExecFailure {
    pub kind: ExecFailureKind,
    /// What the child wrote to stdout before it ended (lossy UTF-8, capped at `max_bytes`).
    pub stdout: String,
    /// What it wrote to stderr (lossy UTF-8, NOT trimmed).
    pub stderr: String,
    /// Exit code, when it exited.
    pub code: Option<i32>,
    /// Terminating signal number (unix), when it was killed.
    pub signal: Option<i32>,
    /// The `ExecOpts` limits that applied.
    pub timeout: Option<Duration>,
    pub max_bytes: usize,
    /// Text of the failed spawn (`Spawn` only).
    pub detail: String,
}

impl ExecFailure {
    /// The kit's own wording (what [`exec_self`] returns).
    pub fn default_text(&self) -> String {
        match self.kind {
            ExecFailureKind::Spawn => self.detail.clone(),
            ExecFailureKind::OutputLimit => format!("output exceeded {} bytes", self.max_bytes),
            ExecFailureKind::Timeout => format!(
                "timed out after {}s",
                self.timeout.map_or(0, |t| t.as_secs())
            ),
            ExecFailureKind::Signal => "terminated by a signal".into(),
            ExecFailureKind::Exit => {
                let e = self.stderr.trim();
                match self.code {
                    Some(c) if e.is_empty() => format!("exited with code {c}"),
                    _ => e.to_string(),
                }
            }
        }
    }
}

/// Run this very executable with `args` and return its stdout: how an MCP tool "runs the CLI's
/// own path with --json". Stdin is closed. A listed exit code gives `Ok(stdout)`; any other exit,
/// a timeout or an oversized output gives `Err` (stderr text when there is some).
pub fn exec_self(args: &[String], o: &ExecOpts) -> Result<String, String> {
    exec_self_with(args, o, ExecFailure::default_text)
}

/// [`exec_self`] with a template for the `Err` of an exit code not in `ok_codes`: `{stderr}` is
/// the child's trimmed stderr and `{code}` its exit code (android keeps `"{stderr} (exit {code})"`).
pub fn exec_self_fmt(
    args: &[String],
    o: &ExecOpts,
    error_template: Option<&str>,
) -> Result<String, String> {
    exec_self_with(args, o, |f| match (error_template, f.kind, f.code) {
        (Some(t), ExecFailureKind::Exit, Some(c)) => t
            .replace("{stderr}", f.stderr.trim())
            .replace("{code}", &c.to_string()),
        _ => f.default_text(),
    })
}

/// [`exec_self`] where the tool words every error itself: `fmt` gets an [`ExecFailure`] (kind,
/// stdout, stderr, code, signal, limits) and returns the `Err` text.
pub fn exec_self_with(
    args: &[String],
    o: &ExecOpts,
    fmt: impl Fn(&ExecFailure) -> String,
) -> Result<String, String> {
    exec_core(args, o).map_err(|f| fmt(&f))
}

fn exec_core(args: &[String], o: &ExecOpts) -> Result<String, ExecFailure> {
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    let fail = |kind, stdout: &[u8], stderr: &[u8], code, signal, detail: String| ExecFailure {
        kind,
        stdout: String::from_utf8_lossy(stdout).into_owned(),
        stderr: String::from_utf8_lossy(stderr).into_owned(),
        code,
        signal,
        timeout: o.timeout,
        max_bytes: o.max_bytes,
        detail,
    };
    let spawn_fail = |m: String| fail(ExecFailureKind::Spawn, b"", b"", None, None, m);
    let exe = std::env::current_exe()
        .map_err(|e| spawn_fail(format!("cannot find own executable: {e}")))?;
    let mut child = Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| spawn_fail(format!("cannot start: {e}")))?;
    let over = Arc::new(AtomicBool::new(false));
    let drain = |mut r: Box<dyn Read + Send>| {
        let (over, max) = (over.clone(), o.max_bytes);
        std::thread::spawn(move || {
            let (mut buf, mut chunk) = (Vec::new(), [0u8; 8192]);
            while let Ok(n) = r.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                if buf.len() + n > max {
                    over.store(true, Ordering::SeqCst);
                    // Keep draining so the child is never blocked on a full pipe before the kill.
                } else {
                    buf.extend_from_slice(&chunk[..n]);
                }
            }
            buf
        })
    };
    let out = drain(Box::new(child.stdout.take().unwrap()));
    let err = drain(Box::new(child.stderr.take().unwrap()));
    let start = std::time::Instant::now();
    let mut killed = None;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => {}
            Err(e) => return Err(spawn_fail(e.to_string())),
        }
        if over.load(Ordering::SeqCst) {
            killed = Some(ExecFailureKind::OutputLimit);
        } else if o.timeout.is_some_and(|t| start.elapsed() >= t) {
            killed = Some(ExecFailureKind::Timeout);
        }
        if killed.is_some() {
            let _ = child.kill();
            break child.wait().map_err(|e| spawn_fail(e.to_string()))?;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let (out, err) = (
        out.join().unwrap_or_default(),
        err.join().unwrap_or_default(),
    );
    #[cfg(unix)]
    let signal = {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    };
    #[cfg(not(unix))]
    let signal = None;
    if let Some(kind) = killed {
        return Err(fail(kind, &out, &err, status.code(), signal, String::new()));
    }
    if over.load(Ordering::SeqCst) {
        return Err(fail(
            ExecFailureKind::OutputLimit,
            &out,
            &err,
            status.code(),
            signal,
            String::new(),
        ));
    }
    match status.code() {
        Some(c) if o.ok_codes.contains(&c) => Ok(String::from_utf8_lossy(&out).into_owned()),
        Some(c) => Err(fail(
            ExecFailureKind::Exit,
            &out,
            &err,
            Some(c),
            None,
            String::new(),
        )),
        None => Err(fail(
            ExecFailureKind::Signal,
            &out,
            &err,
            None,
            signal,
            String::new(),
        )),
    }
}

fn target_props() -> Value {
    json!({
        "path": {"type": "string", "description": "Directory to examine (default .)"},
        "scope": {"type": "string", "enum": ["full", "changed"]},
        "base": {"type": "string", "description": "With scope=changed: also files changed since merge-base with this ref"},
        "fail_on": {"type": "string", "enum": ["info", "low", "medium", "high", "critical"]},
        "config": {"type": "string", "description": "Path to doctor.toml"}
    })
}

pub fn tool_list() -> Value {
    tool_props(target_props())
}

/// The tool list for `d`: a detached doctor has no `path`, `scope` or `base` to offer.
pub fn tool_list_for(d: &dyn Doctor) -> Value {
    if let Some(tools) = d.mcp_tools() {
        let list: Vec<Value> = tools
            .iter()
            .map(|t| json!({"name": t.name, "description": t.description, "inputSchema": t.schema}))
            .collect();
        return json!({ "tools": list });
    }
    let mut p = target_props();
    if d.target() == crate::TargetKind::Detached {
        for k in ["path", "scope", "base"] {
            p.as_object_mut().unwrap().remove(k);
        }
    }
    tool_props(p)
}

fn tool_props(props: Value) -> Value {
    let mut fix_props = props.clone();
    fix_props["id"] = json!({"type": "string", "description": "Finding rule id or fingerprint prefix; omit for the whole scan"});
    json!({"tools": [
        {"name": "scan", "description": "Scan a directory; returns the doctor/1 envelope as JSON text.",
         "inputSchema": {"type": "object", "properties": props}},
        {"name": "fix", "description": "Build a coding-agent prompt from one finding or the whole scan.",
         "inputSchema": {"type": "object", "properties": fix_props}}
    ]})
}

fn opts_of(d: &dyn Doctor, args: &Value) -> Result<(String, Opts), String> {
    let s = |k: &str| args.get(k).and_then(Value::as_str);
    let scope = match s("scope").unwrap_or("full") {
        "full" => crate::Scope::Full,
        "changed" => crate::Scope::Changed,
        o => return Err(format!("bad scope `{o}`")),
    };
    let fail_on = match s("fail_on") {
        Some(f) => crate::cli::parse_sev(f)?,
        None => d.default_fail_on(),
    };
    Ok((
        s("path").unwrap_or(".").to_string(),
        Opts {
            scope,
            base: s("base").map(String::from),
            config: s("config").map(Into::into),
            fail_on,
        },
    ))
}

fn call(d: &dyn Doctor, name: &str, args: &Value) -> Result<String, String> {
    if let Some(tools) = d.mcp_tools() {
        return match tools.iter().find(|t| t.name == name) {
            Some(t) => (t.call)(args).map_err(|e| fill(&d.mcp_texts().tool_error, "e", &e)),
            None => Err(fill(&d.mcp_texts().unknown_tool, "t", name)),
        };
    }
    let (path, opts) = opts_of(d, args)?;
    let env = d
        .try_scan(&ctx_from(d, std::path::Path::new(&path), &opts)?)
        .map_err(|f| f.to_string())?;
    match name {
        "scan" => Ok(d.render_json(&env)),
        "fix" => {
            let hits = fix::select(&env, args.get("id").and_then(Value::as_str))?;
            Ok(d.fix_prompt(&env, &hits))
        }
        o => Err(fill(&d.mcp_texts().unknown_tool, "t", o)),
    }
}

fn error(id: Value, code: i64, msg: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": msg}})
}

/// Handle one request; `None` for a notification.
pub fn handle(d: &dyn Doctor, req: &Value) -> Option<Value> {
    let wire = d.mcp_wire();
    let texts = d.mcp_texts();
    let Some(obj) = req.as_object() else {
        return Some(error(Value::Null, -32600, &texts.invalid_request));
    };
    let id = match obj.get("id") {
        // A notification (a method, no id) gets no answer. Neither an id nor a string method is
        // an invalid request (JSON-RPC 2.0): -32600 with a null id.
        None if obj.get("method").is_some_and(Value::is_string) => return None,
        None => return Some(error(Value::Null, -32600, &texts.invalid_request_method)),
        Some(v) if v.is_string() || v.is_i64() || v.is_u64() => v.clone(),
        Some(_) => return Some(error(Value::Null, -32600, &texts.invalid_request_id)),
    };
    let result = match obj.get("method").and_then(Value::as_str) {
        None => return Some(error(id, -32600, &texts.invalid_request_method)),
        Some("initialize") => json!({
            "protocolVersion": match &wire.protocol {
                Protocol::Fixed(v) => v.clone(),
                Protocol::EchoClient => req
                    .pointer("/params/protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or(DEFAULT_PROTOCOL)
                    .to_string(),
            },
            "capabilities": {"tools": {}},
            "serverInfo": match &texts.server_info {
                Some((n, v)) => json!({"name": n, "version": v}),
                None => json!({"name": d.name(), "version": d.version()}),
            },
        }),
        Some("ping") => json!({}),
        Some("tools/list") => tool_list_for(d),
        Some("tools/call") => {
            let name = req
                .pointer("/params/name")
                .and_then(Value::as_str)
                .unwrap_or("");
            let args = req.pointer("/params/arguments").unwrap_or(&Value::Null);
            let (text, is_error) = match call(d, name, args) {
                Ok(t) => (t, false),
                Err(e) => (e, true),
            };
            json!({"content": [{"type": "text", "text": text}], "isError": is_error})
        }
        Some(m) => return Some(error(id, -32601, &fill(&wire.method_not_found, "m", m))),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

pub fn process_line(d: &dyn Doctor, line: &[u8]) -> Option<Value> {
    let wire = d.mcp_wire();
    if !wire.blank_line_is_parse_error && line.iter().all(u8::is_ascii_whitespace) {
        return None;
    }
    match serde_json::from_slice::<Value>(line) {
        Ok(req) => handle(d, &req),
        Err(e) => Some(error(
            Value::Null,
            -32700,
            &fill(&wire.parse_error, "e", &e.to_string()),
        )),
    }
}

/// Serve until stdin closes. Only JSON-RPC lines reach stdout.
pub fn serve(d: &dyn Doctor) -> io::Result<()> {
    serve_verbose(d, false)
}

/// [`serve`], optionally logging one line per request and response to stderr.
pub fn serve_verbose(d: &dyn Doctor, verbose: bool) -> io::Result<()> {
    let (mut out, mut inp, mut line) = (io::stdout(), io::stdin().lock(), Vec::new());
    loop {
        line.clear();
        if inp.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if verbose {
            let method = serde_json::from_slice::<Value>(&line)
                .ok()
                .and_then(|v| v.get("method").and_then(Value::as_str).map(String::from));
            warnln!("mcp <- {}", method.unwrap_or_else(|| "(unparsed)".into()));
        }
        if let Some(r) = process_line(d, &line) {
            if verbose {
                warnln!("mcp -> {} bytes", r.to_string().len());
            }
            writeln!(out, "{r}")
                .and_then(|_| out.flush())
                .map_err(|e| io::Error::new(e.kind(), format!("cannot write to stdout: {e}")))?;
        }
    }
}
