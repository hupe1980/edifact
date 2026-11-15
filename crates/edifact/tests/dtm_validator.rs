//! `DtmValidator`: values that contradict their format code.

use edifact::{
    DtmValidator, FindingKind, Segment, SyntaxError, ValidationContext, ValidationLayer,
    ValidationReport, ValidationSeverity,
};

fn validate(input: &[u8]) -> ValidationReport {
    let segments: Vec<Segment<'_>> = edifact::from_bytes(input)
        .collect::<Result<_, _>>()
        .expect("parse");
    ValidationContext::builder()
        .with_validator(ValidationLayer::CodeList, DtmValidator)
        .build()
        .validate(&segments)
}

#[test]
fn value_contradicting_its_format_is_an_e027_error() {
    let input = b"UNH+1+UTILMD:D:11A:UN'DTM+137:20260701:303'UNT+3+1'";
    let report = validate(input);
    assert_eq!(report.errors().len(), 1);
    assert!(report.warnings().is_empty());
    let issue = &report.errors()[0];
    assert_eq!(issue.severity, ValidationSeverity::Error);
    assert_eq!(issue.error_code(), Some("E027"));
    assert_eq!(issue.kind(), Some(FindingKind::Representation));
    assert_eq!(issue.layer(), Some(ValidationLayer::CodeList));
    assert_eq!(issue.rule_id.as_deref(), Some("DTM-2379"));
    assert_eq!(issue.element_index, Some(0));
    assert_eq!(issue.component_index, Some(1));
    assert_eq!(issue.segment_position, Some(2));
    let span = issue.span.expect("component span");
    assert_eq!(&input[span.start..span.end], b"20260701");
    assert!(issue.message.contains("20260701"), "{}", issue.message);
    assert!(issue.message.contains("303"), "{}", issue.message);
    assert_eq!(SyntaxError::for_issue(issue), SyntaxError::InvalidValue);
    assert_eq!(SyntaxError::for_issue(issue).code(), "12");
}

#[test]
fn unsupported_code_is_a_warning_on_the_code() {
    let input = b"DTM+137:anything:999'";
    let report = validate(input);
    assert!(report.errors().is_empty());
    assert_eq!(report.warnings().len(), 1);
    let issue = &report.warnings()[0];
    assert_eq!(issue.component_index, Some(2));
    assert_eq!(issue.kind(), Some(FindingKind::Representation));
    assert!(issue.message.contains("999"), "{}", issue.message);
    let span = issue.span.expect("component span");
    assert_eq!(&input[span.start..span.end], b"999");
}

#[test]
fn missing_value_or_code_gives_no_finding() {
    for input in [
        &b"DTM+137:20260701'"[..],
        b"DTM+137'",
        b"DTM+137::102'",
        b"DTM+:20260701:102'",
    ] {
        let report = validate(input);
        assert_eq!(
            report.total_issues(),
            0,
            "{}",
            String::from_utf8_lossy(input)
        );
    }
}

#[test]
fn repository_fixtures_give_no_finding() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
    let mut seen = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "edi") {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let Ok(segments) = edifact::from_bytes(&bytes).collect::<Result<Vec<_>, _>>() else {
            continue;
        };
        seen += segments.iter().filter(|s| s.tag == "DTM").count();
        let report = ValidationContext::builder()
            .with_validator(ValidationLayer::CodeList, DtmValidator)
            .build()
            .validate(&segments);
        assert_eq!(report.total_issues(), 0, "{}", path.display());
    }
    assert!(seen > 0);
}

#[test]
fn mako_values_give_no_finding() {
    let input: &[u8] = b"DTM+137:202612312300?+00:303'\
DTM+163:202701010000?+01:303'\
DTM+Z01:201801010500201801020500:719'\
DTM+Z05:0:805'\
DTM+Z05:1:805'\
DTM+137:201801011200:203'\
DTM+137:202606010000?+00:303'\
DTM+157:20261001:102'\
DTM+Z34:3:802'\
DTM+Z35:2:803'\
DTM+Z36:14:804'";
    assert_eq!(validate(input).total_issues(), 0);
}

#[test]
fn one_finding_per_bad_dtm() {
    let report =
        validate(b"DTM+137:20260701:303'DTM+137:2026070112:102'DTM+137:20260701:102'DTM+2:x:101'");
    assert_eq!(report.errors().len(), 2);
    assert_eq!(report.warnings().len(), 1);
}

#[test]
fn code_104_is_decoded() {
    let report = validate(b"DTM+752:05040504:104'");
    assert_eq!(report.total_issues(), 0, "{report:?}");
    let report = validate(b"DTM+752:1001:104'");
    assert!(report.warnings().is_empty());
    assert_eq!(report.errors().len(), 1);
    assert_eq!(report.errors()[0].error_code(), Some("E027"));
}
