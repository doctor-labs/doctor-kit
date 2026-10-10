//! 0.2.x must stay source-compatible with code written against 0.2.0: public structs that tools
//! may build literally keep exactly their 0.2.0 fields. New knobs live in new types, hooks and
//! functions. This file is written with 0.2.0 shapes only; if it stops compiling, a patch release
//! broke a user.
use doctor_core::Severity;
use doctor_kit::{Face, OutputArgs};

#[test]
fn output_args_literal_of_0_2_0_still_compiles() {
    let o = OutputArgs {
        json: false,
        score: false,
        headless: true,
        face: Face::Plain,
        theme: None,
        theme_file: None,
        color: doctor_kit::output::Color::Never,
        sarif: None,
        baseline: None,
        fail_on: Severity::High,
        theme_root: None,
    };
    assert!(o.headless);
}

#[test]
fn mcp_wire_and_exec_opts_literals_of_0_2_0_still_compile() {
    let w = doctor_kit::McpWire {
        protocol: doctor_kit::Protocol::EchoClient,
        parse_error: "p".into(),
        method_not_found: "m {m}".into(),
        blank_line_is_parse_error: true,
    };
    assert!(w.blank_line_is_parse_error);
    let o = doctor_kit::mcp::ExecOpts {
        ok_codes: vec![0, 1],
        timeout: None,
        max_bytes: 10,
    };
    assert_eq!(o.max_bytes, 10);
}
