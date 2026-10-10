//! K6: pluggable MCP tools and wire texts, in process (`handle` is pure).
use doctor_kit::mcp::{handle, process_line};
use doctor_kit::{Check, Doctor, McpTool, McpWire, Protocol};
use serde_json::{json, Value};

struct Own(bool);
impl Doctor for Own {
    fn name(&self) -> &str {
        "own"
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
    fn mcp_tools(&self) -> Option<Vec<McpTool>> {
        self.0.then(|| {
            vec![McpTool {
                name: "echo".into(),
                description: "Echo".into(),
                schema: json!({"type": "object"}),
                call: Box::new(|a| match a.get("fail") {
                    Some(_) => Err("failed".into()),
                    None => Ok(a.to_string()),
                }),
            }]
        })
    }
    fn mcp_wire(&self) -> McpWire {
        McpWire {
            protocol: Protocol::EchoClient,
            parse_error: "bad json".into(),
            method_not_found: "no {m}".into(),
            blank_line_is_parse_error: true,
        }
    }
}

fn req(m: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": m, "params": params})
}

#[test]
fn custom_tools_list_call_and_error() {
    let d = Own(true);
    let l = handle(&d, &req("tools/list", json!({}))).unwrap();
    assert_eq!(l["result"]["tools"][0]["name"], "echo");
    let ok = handle(
        &d,
        &req("tools/call", json!({"name": "echo", "arguments": {"a": 1}})),
    )
    .unwrap();
    assert_eq!(
        ok["result"],
        json!({"content": [{"type": "text", "text": "{\"a\":1}"}], "isError": false})
    );
    let bad = handle(
        &d,
        &req(
            "tools/call",
            json!({"name": "echo", "arguments": {"fail": 1}}),
        ),
    )
    .unwrap();
    assert_eq!(bad["result"]["isError"], true);
    assert_eq!(bad["result"]["content"][0]["text"], "failed");
    let no = handle(
        &d,
        &req("tools/call", json!({"name": "scan", "arguments": {}})),
    )
    .unwrap();
    assert_eq!(no["result"]["content"][0]["text"], "unknown tool scan");
}

#[test]
fn wire_texts_and_protocol_echo() {
    let d = Own(true);
    let i = handle(&d, &req("initialize", json!({"protocolVersion": "9999"}))).unwrap();
    assert_eq!(i["result"]["protocolVersion"], "9999");
    let i = handle(&d, &req("initialize", json!({}))).unwrap();
    assert_eq!(
        i["result"]["protocolVersion"], "2024-11-05",
        "no client version: the default"
    );
    let m = handle(&d, &req("zzz", json!({}))).unwrap();
    assert_eq!(m["error"]["message"], "no zzz");
    for line in [&b"{nope"[..], b"\n", b"   \n"] {
        let r = process_line(&d, line).unwrap();
        assert_eq!(
            (r["error"]["code"].as_i64(), r["error"]["message"].as_str()),
            (Some(-32700), Some("bad json"))
        );
    }
}

#[test]
fn defaults_are_the_0_1_behaviour() {
    struct Plain;
    impl Doctor for Plain {
        fn name(&self) -> &str {
            "p"
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
    }
    assert!(process_line(&Plain, b"\n").is_none(), "blank lines ignored");
    let r = process_line(&Plain, b"{nope").unwrap();
    assert!(r["error"]["message"]
        .as_str()
        .unwrap()
        .starts_with("parse error: "));
    let l = handle(&Plain, &req("tools/list", json!({}))).unwrap();
    let names: Vec<&str> = l["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["scan", "fix"]);
    let i = handle(
        &Plain,
        &req("initialize", json!({"protocolVersion": "9999"})),
    )
    .unwrap();
    assert_eq!(
        i["result"]["protocolVersion"], "2024-11-05",
        "fixed, not echoed"
    );
}

mod texts {
    use super::*;
    use doctor_kit::McpTexts;

    /// An older server's answers, kept byte for byte.
    struct Old;
    impl Doctor for Old {
        fn name(&self) -> &str {
            "new-name"
        }
        fn version(&self) -> &str {
            "9"
        }
        fn categories(&self) -> Vec<&'static str> {
            vec![]
        }
        fn checks(&self) -> Vec<Box<dyn Check>> {
            vec![]
        }
        fn mcp_tools(&self) -> Option<Vec<McpTool>> {
            Some(vec![McpTool {
                name: "t".into(),
                description: "T".into(),
                schema: json!({"type": "object"}),
                call: Box::new(|_| Err("boom".into())),
            }])
        }
        fn mcp_texts(&self) -> McpTexts {
            McpTexts {
                invalid_request: "Invalid Request".into(),
                invalid_request_id: "Invalid Request (id)".into(),
                invalid_request_method: "Invalid Request (method)".into(),
                unknown_tool: "no such tool: {t}".into(),
                tool_error: "{e} (failed)".into(),
                server_info: Some(("old-name".into(), "1.0".into())),
            }
        }
    }

    #[test]
    fn every_text_is_the_tools_and_defaults_are_the_kits() {
        let d = Old;
        assert_eq!(
            handle(&d, &json!([1])).unwrap()["error"]["message"],
            "Invalid Request"
        );
        assert_eq!(
            handle(&d, &json!({"id": 1.5, "method": "x"})).unwrap()["error"]["message"],
            "Invalid Request (id)"
        );
        assert_eq!(
            handle(&d, &json!({"id": 1})).unwrap()["error"]["message"],
            "Invalid Request (method)"
        );
        for code in [
            handle(&d, &json!([1])).unwrap(),
            handle(&d, &json!({"id": 1})).unwrap(),
        ] {
            assert_eq!(code["error"]["code"], -32600, "the protocol fix stays");
        }
        assert_eq!(
            handle(&d, &req("ping", json!({}))).unwrap()["result"],
            json!({}),
            "ping stays answered"
        );
        let i = handle(&d, &req("initialize", json!({}))).unwrap();
        assert_eq!(
            i["result"]["serverInfo"],
            json!({"name": "old-name", "version": "1.0"})
        );
        let u = handle(&d, &req("tools/call", json!({"name": "zz"}))).unwrap();
        assert_eq!(u["result"]["content"][0]["text"], "no such tool: zz");
        let e = handle(&d, &req("tools/call", json!({"name": "t"}))).unwrap();
        assert_eq!(
            (
                e["result"]["isError"].clone(),
                e["result"]["content"][0]["text"].clone()
            ),
            (json!(true), json!("boom (failed)"))
        );
        // defaults
        let p = Own(true);
        assert_eq!(
            handle(&p, &json!([1])).unwrap()["error"]["message"],
            "invalid request"
        );
        let u = handle(&p, &req("tools/call", json!({"name": "zz"}))).unwrap();
        assert_eq!(u["result"]["content"][0]["text"], "unknown tool zz");
        let i = handle(&p, &req("initialize", json!({}))).unwrap();
        assert_eq!(
            i["result"]["serverInfo"],
            json!({"name": "own", "version": "1"})
        );
    }
}

mod invalid_and_json {
    use super::*;
    use doctor_kit::mcp::fill;
    use doctor_kit::McpTexts;

    #[test]
    fn notification_stays_silent_but_no_id_and_no_method_is_an_invalid_request() {
        let d = Own(false);
        // method, no id: a notification, no answer
        assert!(handle(
            &d,
            &json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
        )
        .is_none());
        assert!(process_line(&d, br#"{"jsonrpc":"2.0","method":"x"}"#).is_none());
        // neither: -32600 with id null
        for bad in [
            json!({"jsonrpc": "2.0"}),
            json!({}),
            json!({"method": 7}),
            json!({"jsonrpc":"2.0","params":{}}),
        ] {
            let r = handle(&d, &bad).unwrap_or_else(|| panic!("silence for {bad}"));
            assert_eq!(
                (r["error"]["code"].as_i64(), r["id"].clone()),
                (Some(-32600), Value::Null),
                "{bad}"
            );
        }
        // an id with no method is still answered with that id (unchanged)
        let r = handle(&d, &json!({"jsonrpc": "2.0", "id": 5})).unwrap();
        assert_eq!(
            (r["error"]["code"].as_i64(), r["id"].clone()),
            (Some(-32600), json!(5))
        );
        // not an object, as before
        assert_eq!(handle(&d, &json!([1])).unwrap()["error"]["code"], -32600);
    }

    #[test]
    fn fill_gives_plain_json_literal_and_escaped_forms_in_one_pass() {
        let name = r#"a"b\c{t}"#;
        assert_eq!(fill("x {t}", "t", name), format!("x {name}"));
        assert_eq!(fill("{t_json}", "t", name), r#""a\"b\\c{t}""#);
        assert_eq!(fill("{t_esc}", "t", name), r#"a\"b\\c{t}"#);
        assert_eq!(
            fill("{e} {t}", "t", "v"),
            "{e} v",
            "other keys and stray braces are left alone"
        );
        let doc = fill(r#"{"error":"unknown tool {t_esc}"}"#, "t", name);
        let v: Value = serde_json::from_str(&doc).unwrap();
        assert_eq!(v["error"], format!("unknown tool {name}"));
    }

    #[test]
    fn unknown_tool_with_quotes_yields_valid_json_through_the_wire() {
        struct J;
        impl Doctor for J {
            fn name(&self) -> &str {
                "j"
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
            fn mcp_tools(&self) -> Option<Vec<McpTool>> {
                Some(vec![])
            }
            fn mcp_texts(&self) -> McpTexts {
                McpTexts {
                    unknown_tool: r#"{"error":{t_json}}"#.into(),
                    ..McpTexts::default()
                }
            }
        }
        let r = handle(&J, &req("tools/call", json!({"name": "ev\"il"}))).unwrap();
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(text).unwrap(),
            json!({"error": "ev\"il"})
        );
    }
}
