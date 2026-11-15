//! Diagnostic text built from hostile input stays one line.
//!
//! Error `Display`, issue `Display` and the deterministic report rendering
//! write control characters, U+2028/U+2029 and bidi controls from values as
//! escapes; the stored fields keep the raw values.

use edifact::{
    EdifactError, GroupPath, GroupStep, Span, ValidationIssue, ValidationReport,
    ValidationSeverity, from_bytes, validate_envelope,
};

/// Every character the escape neutralises, plus text around it.
const HOSTILE: &str = "x\r\n\x1b[31m\u{85}\u{2028}y";

/// Whether `text` holds a character the escape must have removed.
fn has_control(text: &str) -> bool {
    text.chars()
        .any(|c| c.is_control() || c == '\u{2028}' || c == '\u{2029}')
}

/// Two `UNH` references carrying CR LF: the sender tries to forge a log line.
#[test]
fn forged_reference_prints_on_one_line() {
    let input = b"UNB+UNOC:3+S+R+260101:0900+IC1'\
UNH+A\r\nFAKE LINE+ORDERS:D:96A:UN'BGM+220'UNT+3+A\r\nFAKE LINE'\
UNH+A\r\nFAKE LINE+ORDERS:D:96A:UN'UNT+2+A\r\nFAKE LINE'UNZ+2+IC1'";
    let segments: Vec<_> = from_bytes(input).collect::<Result<_, _>>().expect("parses");
    let error = validate_envelope(&segments).expect_err("duplicate reference");
    assert_eq!(
        error.to_string(),
        r"duplicate UNH reference 'A\r\nFAKE LINE' at bytes 91..124"
    );
    let EdifactError::DuplicateReference { reference, .. } = &error else {
        panic!("{error:?}");
    };
    assert_eq!(reference, "A\r\nFAKE LINE", "the field keeps the raw value");
}

#[test]
fn clean_values_print_unchanged() {
    let error = EdifactError::QualifierMismatch {
        tag: "NAD".to_owned(),
        actual: "BY".to_owned(),
        expected: "SU".to_owned(),
        span: Span::new(0, 12),
    };
    assert_eq!(
        error.to_string(),
        "segment NAD has qualifier 'BY', expected 'SU'"
    );
}

fn hostile_issue(severity: ValidationSeverity) -> ValidationIssue {
    let path = GroupPath::new([GroupStep::new(HOSTILE, 0).expect("step")]).expect("path");
    ValidationIssue::new(severity, HOSTILE)
        .with_segment(HOSTILE)
        .with_rule_id(HOSTILE)
        .with_suggestion(HOSTILE)
        .with_pack(HOSTILE)
        .with_group_path(path)
        .with_span(Span::new(0, 3))
}

#[test]
fn report_renders_one_line_per_issue() {
    let mut report = ValidationReport::default();
    report.add_error(hostile_issue(ValidationSeverity::Error));
    report.add_warning(hostile_issue(ValidationSeverity::Warning));
    let text = report.render_deterministic();
    // "Validation Report:", two bucket headers, two issues.
    assert_eq!(text.lines().count(), 5, "{text}");
    assert!(!has_control(&text.replace('\n', "")), "{text:?}");
    assert_eq!(report.to_string(), text);

    let issue = hostile_issue(ValidationSeverity::Error);
    let line = issue.to_string();
    assert!(!has_control(&line), "{line:?}");
    assert_eq!(issue.message, HOSTILE, "the field keeps the raw value");
}

#[test]
fn escaping_is_idempotent() {
    // An issue built from an error's text renders that text unchanged.
    let error = EdifactError::EmptyMessage {
        message_ref: HOSTILE.to_owned(),
        span: Span::new(0, 1),
    };
    let text = error.to_string();
    let issue = ValidationIssue::new(ValidationSeverity::Error, text.clone());
    assert_eq!(issue.to_string(), format!("[ERROR] {text}"));
}

#[cfg(feature = "diagnostics")]
#[test]
fn miette_help_and_labels_are_escaped() {
    use miette::Diagnostic;
    let issue = hostile_issue(ValidationSeverity::Error);
    let help = issue.help().expect("suggestion").to_string();
    assert!(!has_control(&help), "{help:?}");
    for label in issue.labels().expect("one label") {
        let text = label.label().expect("text");
        assert!(!has_control(text), "{text:?}");
    }
}

/// A right-to-left override in a reference must not reorder the message.
#[test]
fn bidi_overrides_are_escaped() {
    use edifact::ValidationContext;

    const BIDI: [char; 12] = [
        '\u{061C}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}',
        '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
    ];
    for bidi in BIDI {
        let input = format!(
            "UNB+UNOY:4+S+R+20260101:0900+IC1'UNH+A{bidi}gnp.exe+ORDERS:D:96A:UN'BGM+220'UNT+3+B'UNZ+1+IC1'"
        );
        let segments: Vec<_> = from_bytes(input.as_bytes())
            .collect::<Result<_, _>>()
            .expect("parses");
        let escape = format!("\\u{{{:x}}}", u32::from(bidi));

        let error = validate_envelope(&segments).expect_err("reference mismatch");
        let text = error.to_string();
        assert!(text.contains(&format!("A{escape}gnp.exe")), "{text}");
        assert!(!text.contains(bidi), "{text:?}");

        let rendered = ValidationContext::builder()
            .with_envelope_validation()
            .build()
            .validate(&segments)
            .render_deterministic();
        assert!(rendered.contains(&escape), "{rendered}");
        assert!(!rendered.contains(bidi), "{rendered:?}");
    }
}
