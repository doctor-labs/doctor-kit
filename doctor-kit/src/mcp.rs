//! `mcp`: a Model Context Protocol server, JSON-RPC 2.0 over stdio, no extra crates.
//! Tools run in-process through the same code as the CLI, so `scan` returns the doctor/1
//! envelope byte-identical to `scan --json` stdout.
use crate::cli::{ctx_from, Opts};
use crate::doctor::Doctor;
use crate::fix;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

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
        o => Err(format!("unknown tool {o}")),
    }
}

fn error(id: Value, code: i64, msg: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": msg}})
}

/// Handle one request; `None` for a notification.
pub fn handle(d: &dyn Doctor, req: &Value) -> Option<Value> {
    let Some(obj) = req.as_object() else {
        return Some(error(Value::Null, -32600, "invalid request"));
    };
    let id = match obj.get("id") {
        None => return None,
        Some(v) if v.is_string() || v.is_i64() || v.is_u64() => v.clone(),
        Some(_) => return Some(error(Value::Null, -32600, "invalid request: bad id")),
    };
    let result = match obj.get("method").and_then(Value::as_str) {
        None => return Some(error(id, -32600, "invalid request: no method")),
        Some("initialize") => json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": d.name(), "version": d.version()},
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
        Some(m) => return Some(error(id, -32601, &format!("unknown method {m}"))),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

pub fn process_line(d: &dyn Doctor, line: &[u8]) -> Option<Value> {
    if line.iter().all(u8::is_ascii_whitespace) {
        return None;
    }
    match serde_json::from_slice::<Value>(line) {
        Ok(req) => handle(d, &req),
        Err(e) => Some(error(Value::Null, -32700, &format!("parse error: {e}"))),
    }
}

/// Serve until stdin closes. Only JSON-RPC lines reach stdout.
pub fn serve(d: &dyn Doctor) -> io::Result<()> {
    let (mut out, mut inp, mut line) = (io::stdout(), io::stdin().lock(), Vec::new());
    loop {
        line.clear();
        if inp.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if let Some(r) = process_line(d, &line) {
            writeln!(out, "{r}")?;
            out.flush()?;
        }
    }
}
