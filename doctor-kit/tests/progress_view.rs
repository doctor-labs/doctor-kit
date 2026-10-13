//! The status line a terminal shows while a scan runs (issue #29, second slice).
use doctor_core::Severity;
use doctor_kit::progress::ProgressView;
use doctor_kit::ProgressEvent as E;

fn feed(view: &mut ProgressView, events: &[E]) {
    for e in events {
        view.apply(e);
    }
}

fn started(id: &str, index: usize, total: usize) -> E {
    E::CheckStarted {
        id: id.into(),
        index,
        total,
    }
}
fn finding(id: &str, severity: Severity, message: &str) -> E {
    E::Finding {
        id: id.into(),
        severity,
        message: message.into(),
    }
}

#[test]
fn a_fresh_view_shows_an_empty_bar() {
    assert_eq!(ProgressView::new().line(80), "[----------] 0/0");
}

#[test]
fn the_line_shows_checks_done_a_severity_tally_and_the_latest_finding() {
    let mut v = ProgressView::new();
    feed(
        &mut v,
        &[
            started("t.a", 1, 3),
            finding("t.a", Severity::High, "boom"),
            finding("t.a", Severity::Low, "meh"),
            E::CheckFinished {
                id: "t.a".into(),
                findings: 2,
            },
            E::CheckSkipped {
                id: "t.b".into(),
                index: 2,
                total: 3,
            },
        ],
    );
    // 2 of 3 checks done (one finished, one skipped): floor(10 * 2 / 3) = 6 filled cells.
    // The tally lists non-zero severities, most severe first.
    assert_eq!(
        v.line(80),
        "[######----] 2/3 \u{b7} 1 high \u{b7} 1 low \u{b7} latest: meh"
    );
}

#[test]
fn done_fills_the_bar() {
    let mut v = ProgressView::new();
    feed(
        &mut v,
        &[
            started("t.a", 1, 1),
            E::CheckFinished {
                id: "t.a".into(),
                findings: 0,
            },
            E::Done { findings: 0 },
        ],
    );
    assert_eq!(v.line(80), "[##########] 1/1");
}

#[test]
fn a_line_longer_than_the_width_is_cut_with_an_ellipsis() {
    let mut v = ProgressView::new();
    feed(
        &mut v,
        &[
            started("t.a", 1, 3),
            finding(
                "t.a",
                Severity::High,
                "a very long finding message that cannot fit",
            ),
        ],
    );
    let line = v.line(24);
    assert_eq!(line.chars().count(), 24);
    assert!(line.ends_with('\u{2026}'), "{line}");
    assert_eq!(v.line(0), "");
}
