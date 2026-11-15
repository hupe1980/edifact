//! Service segments are validated by default: representations per syntax
//! version, S004 calendar, syntax version, group counts, refused and reserved
//! `U` tags, and data after the interchange trailer.
//!
//! Every input runs through `validate_envelope_lenient` on the slice and the
//! reader path; the strict entry point and the `Envelope` layer of a
//! `ValidationContext` are checked to agree.

use edifact::contrl::Contrl;
use edifact::recover::Scanned;
use edifact::{
    EdifactError, OwnedSegment, RecoveryConfig, Span, ValidationContext, ValidationIssue,
    ValidationLayer, from_bytes, from_reader, scan_recovering, validate_envelope,
    validate_envelope_lenient,
};

const BODY: &str = "UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+3+1'";

fn parse(input: &[u8]) -> Vec<OwnedSegment> {
    from_bytes(input)
        .map(|r| r.map(|s| s.into_owned()))
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(input)))
}

fn parse_reader(input: &[u8]) -> Vec<OwnedSegment> {
    from_reader(std::io::Cursor::new(input))
        .collect::<Result<Vec<_>, _>>()
        .expect("reader parses")
}

/// The lenient findings of `input`, after checking that the reader path, the
/// strict entry point and the `Envelope` layer agree with them.
fn errors(input: &str) -> Vec<EdifactError> {
    let segments = parse(input.as_bytes());
    let lenient = validate_envelope_lenient(&segments);
    assert_eq!(
        validate_envelope_lenient(&parse_reader(input.as_bytes())).errors,
        lenient.errors,
        "reader path diverged on {input}"
    );
    match validate_envelope(&segments) {
        Ok(_) => assert!(lenient.errors.is_empty(), "{input}"),
        Err(first) => assert_eq!(Some(&first), lenient.errors.first(), "{input}"),
    }
    let layer: Vec<_> = envelope_layer(&segments)
        .iter()
        .filter_map(|i| i.error_code().map(str::to_owned))
        .collect();
    let codes: Vec<_> = lenient.errors.iter().map(|e| e.stable_code()).collect();
    assert_eq!(layer, codes, "Envelope layer diverged on {input}");
    lenient.errors
}

fn codes(input: &str) -> Vec<&'static str> {
    errors(input)
        .iter()
        .map(EdifactError::stable_code)
        .collect()
}

fn envelope_layer(segments: &[OwnedSegment]) -> Vec<ValidationIssue> {
    ValidationContext::builder()
        .with_envelope_validation()
        .structure(false)
        .code_list(false)
        .profile(false)
        .build()
        .validate(segments)
        .errors()
        .to_vec()
}

fn interchange(unb: &str) -> String {
    format!("{unb}'{BODY}UNZ+1+1'")
}

/// The byte range of the first occurrence of `needle` in `haystack`.
fn span_of(haystack: &str, needle: &str) -> Span {
    let start = haystack.find(needle).expect("needle");
    Span::new(start, start + needle.len())
}

/// The span of the segment `text` (without its terminator) in `haystack`; a
/// segment's span includes the terminator.
fn segment_span(haystack: &str, text: &str) -> Span {
    let value = span_of(haystack, text);
    Span::new(value.start, value.end + 1)
}

// ── default representation checks ────────────────────────────────────────────

#[test]
fn a_v3_date_of_eight_digits_is_too_long() {
    let input = interchange("UNB+UNOA:3+S+R+20260101:0900+1");
    assert_eq!(
        errors(&input),
        [EdifactError::DataElementTooLong {
            tag: "UNB".into(),
            element_index: 3,
            component_index: 0,
            repr: "n6".into(),
            actual: 8,
            span: span_of(&input, "20260101"),
        }]
    );
}

#[test]
fn a_v4_date_of_six_digits_is_too_short() {
    let input = interchange("UNB+UNOA:4+S+R+260101:0900+1");
    assert_eq!(codes(&input), ["E050"]);
}

#[test]
fn a_control_reference_of_15_characters_is_too_long_in_unb_and_unz() {
    let input = format!("UNB+UNOA:3+S+R+260101:0900+ABCDEFGHIJKLMNO'{BODY}UNZ+1+ABCDEFGHIJKLMNO'");
    let found: Vec<_> = errors(&input)
        .into_iter()
        .map(|e| match e {
            EdifactError::DataElementTooLong {
                tag, element_index, ..
            } => (tag, element_index),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(found, [("UNB".into(), 4), ("UNZ".into(), 1)]);
}

#[test]
fn de_0008_is_an_14_in_v3_and_an_35_in_v4() {
    let long = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    assert_eq!(
        codes(&interchange(&format!(
            "UNB+UNOA:3+S::{long}+R+260101:0900+1"
        ))),
        ["E049"]
    );
    assert_eq!(
        codes(&interchange(&format!(
            "UNB+UNOA:4+S::{long}+R+20260101:0900+1"
        ))),
        [""; 0]
    );
}

#[test]
fn every_time_before_ten_passes_both_paths() {
    for (version, date) in [("3", "260101"), ("4", "20260101")] {
        for time in ["0005", "0900", "2359"] {
            let input = interchange(&format!("UNB+UNOC:{version}+S+R+{date}:{time}+1"));
            assert_eq!(codes(&input), [""; 0], "{input}");

            let segments = parse(input.as_bytes());
            let report = ValidationContext::builder()
                .with_validator(ValidationLayer::Structure, service_directory())
                .build()
                .validate(&segments[..1]);
            assert!(
                report.errors().is_empty() && report.warnings().is_empty(),
                "{input}: {:?} {:?}",
                report.errors(),
                report.warnings()
            );
        }
    }
}

#[test]
fn a_non_numeric_count_is_e048_only() {
    for (input, tag) in [
        (format!("UNB+UNOA:3+S+R+260101:0900+1'{BODY}UNZ+abc+1'"), "UNZ"),
        (
            "UNB+UNOA:3+S+R+260101:0900+1'UNG+++++G1'UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+3+1'UNE+abc+G1'UNZ+1+1'"
                .to_owned(),
            "UNE",
        ),
        (
            "UNB+UNOA:3+S+R+260101:0900+1'UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+abc+1'UNZ+1+1'"
                .to_owned(),
            "UNT",
        ),
    ] {
        let found = errors(&input);
        assert!(
            matches!(
                found.as_slice(),
                [EdifactError::InvalidCharacterType { tag: t, element_index: 0, span, .. }]
                    if t == tag && *span == span_of(&input, "abc")
            ),
            "{input}: {found:?}"
        );
        assert!(validate_envelope(&parse(input.as_bytes())).is_err());
    }
}

#[test]
fn a_signed_or_decimal_count_is_e048_as_well() {
    for count in ["-1", "1.0", "1E0"] {
        let input = format!("UNB+UNOA:3+S+R+260101:0900+1'{BODY}UNZ+{count}+1'");
        assert_eq!(codes(&input), ["E048"], "{input}");
    }
}

#[test]
fn a_missing_mandatory_component_is_e021() {
    let input = "UNB+UNOA:3+S+R+260101:0900+1'UNH+1+ORDERS:D'BGM+220'UNT+3+1'UNZ+1+1'";
    let found: Vec<_> = errors(input)
        .into_iter()
        .map(|e| match e {
            EdifactError::MissingRequiredComponent {
                tag,
                element_index,
                component_index,
            } => (tag, element_index, component_index),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(found, [("UNH".into(), 1, 2), ("UNH".into(), 1, 3)]);
}

#[test]
fn a_released_value_is_measured_after_unescaping() {
    // `AB?+CDEFGHIJKLM` is 14 characters: the release character is not counted.
    let input = "UNB+UNOA:3+S+R+260101:0900+AB?+CDEFGHIJKLM'UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+3+1'UNZ+1+AB?+CDEFGHIJKLM'";
    assert_eq!(codes(input), [""; 0]);
}

#[test]
fn length_is_counted_in_characters() {
    let name = "Müller Möbel Großhandel Jürgen Ölmü"; // 35 characters, more bytes
    assert_eq!(name.chars().count(), 35);
    let input = interchange(&format!("UNB+UNOC:3+{name}+R+260101:0900+1"));
    assert_eq!(codes(&input), [""; 0]);
}

#[test]
fn optional_and_mandatory_empty_composites() {
    assert_eq!(
        codes(&interchange("UNB+UNOA:3+S+R+260101:0900+1++APPREF")),
        [""; 0]
    );
    let found = errors(&interchange("UNB+UNOA:3+S+R++1"));
    assert_eq!(
        found,
        [EdifactError::MissingRequiredElement {
            tag: "UNB".into(),
            element_index: 3
        }]
    );
}

#[test]
fn a_ung_with_only_its_reference_is_valid() {
    let input = "UNB+UNOA:3+S+R+260101:0900+1'UNG+++++G1'UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+3+1'UNE+1+G1'UNZ+1+1'";
    assert_eq!(codes(input), [""; 0]);
}

#[test]
fn leading_zeroes_in_a_count_are_no_envelope_finding() {
    let input = "UNB+UNOA:3+S+R+260101:0900+1'UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+0003+1'UNZ+1+1'";
    assert_eq!(codes(input), [""; 0]);
}

#[test]
fn a_huge_count_is_too_long_and_not_compared() {
    let input = format!("UNB+UNOA:3+S+R+260101:0900+1'{BODY}UNZ+99999999999+1'");
    assert_eq!(codes(&input), ["E049"]);
}

#[test]
fn a_ten_digit_unt_count_is_compared_exactly() {
    for declared in [4_294_967_296_u64, 9_999_999_999] {
        let input = format!(
            "UNB+UNOA:4+S+R+20260101:0900+1'UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+{declared}+1'UNZ+1+1'"
        );
        let segments: Vec<_> = from_bytes(input.as_bytes())
            .collect::<Result<_, _>>()
            .expect("parses");
        let error = validate_envelope(&segments).expect_err("count mismatch");
        assert!(
            matches!(
                error,
                EdifactError::SegmentCountMismatch { expected, actual: 3, .. } if expected == declared
            ),
            "{error:?}"
        );
        assert!(error.to_string().contains(&declared.to_string()), "{error}");
        let lenient = validate_envelope_lenient(&segments);
        let message = &lenient.interchange.expect("interpretable").messages[0];
        assert_eq!(message.declared_segment_count, declared);
        assert_eq!(message.actual_segment_count, 3);
    }
}

#[test]
fn unt_0074_is_n_6_in_v3_and_n_10_in_v4() {
    let v3 = "UNB+UNOA:3+S+R+260101:0900+1'UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+0000003+1'UNZ+1+1'";
    assert_eq!(codes(v3), ["E049"]);
    let v4 = "UNB+UNOA:4+S+R+20260101:0900+1'UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+0000003+1'UNZ+1+1'";
    assert_eq!(codes(v4), [""; 0]);
}

// ── S004 calendar ────────────────────────────────────────────────────────────

#[test]
fn an_impossible_date_and_time_are_e060() {
    let input = interchange("UNB+UNOC:3+S+R+261399:2599+1");
    let found = errors(&input);
    assert_eq!(
        found,
        [
            EdifactError::InvalidDateTime {
                tag: "UNB".into(),
                element_index: 3,
                component_index: 0,
                value: "261399".into(),
                span: span_of(&input, "261399"),
            },
            EdifactError::InvalidDateTime {
                tag: "UNB".into(),
                element_index: 3,
                component_index: 1,
                value: "2599".into(),
                span: span_of(&input, "2599"),
            },
        ]
    );
}

#[test]
fn leap_days_follow_the_version() {
    // v3 YYMMDD: the century is not transferred, so 00 is a leap year.
    assert_eq!(codes(&interchange("UNB+UNOC:3+S+R+000229:0900+1")), [""; 0]);
    assert_eq!(
        codes(&interchange("UNB+UNOC:3+S+R+010229:0900+1")),
        ["E060"]
    );
    // v4 CCYYMMDD: Gregorian.
    assert_eq!(
        codes(&interchange("UNB+UNOC:4+S+R+20000229:0900+1")),
        [""; 0]
    );
    assert_eq!(
        codes(&interchange("UNB+UNOC:4+S+R+19000229:0900+1")),
        ["E060"]
    );
    assert_eq!(
        codes(&interchange("UNB+UNOC:4+S+R+20260431:2400+1")),
        ["E060", "E060"]
    );
}

#[test]
fn a_malformed_date_is_not_also_a_calendar_finding() {
    assert_eq!(
        codes(&interchange("UNB+UNOC:3+S+R+2613991:0900+1")),
        ["E049"]
    );
    assert_eq!(
        codes(&interchange("UNB+UNOC:3+S+R+26AB99:99+1")),
        ["E048", "E050"]
    );
}

#[test]
fn a_group_date_is_checked_too() {
    let input = "UNB+UNOC:3+S+R+260101:0900+1'UNG+ORDERS+S+R+261301:0900+G1'UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+3+1'UNE+1+G1'UNZ+1+1'";
    let found = errors(input);
    assert!(
        matches!(found.as_slice(), [EdifactError::InvalidDateTime { tag, .. }] if tag == "UNG"),
        "{found:?}"
    );
}

// ── syntax version ───────────────────────────────────────────────────────────

#[test]
fn an_unsupported_syntax_version_is_e059() {
    for version in ["0", "5", "9", "x", "X"] {
        let input = interchange(&format!("UNB+UNOC:{version}+S+R+260101:0900+1"));
        assert_eq!(
            errors(&input),
            [EdifactError::UnsupportedSyntaxVersion {
                value: version.into(),
                span: Span::new(9, 10),
            }],
            "{input}"
        );
    }
}

#[test]
fn versions_one_to_four_are_supported() {
    for (version, date) in [
        ("1", "260101"),
        ("2", "260101"),
        ("3", "260101"),
        ("4", "20260101"),
    ] {
        let input = interchange(&format!("UNB+UNOA:{version}+S+R+{date}:0900+1"));
        assert_eq!(codes(&input), [""; 0], "{input}");
    }
}

#[test]
fn a_missing_syntax_version_is_e021_not_e059() {
    assert_eq!(codes(&interchange("UNB+UNOA+S+R+260101:0900+1")), ["E021"]);
}

#[test]
fn an_unknown_version_accepts_either_representation() {
    for date in ["260101", "20260101"] {
        let input = interchange(&format!("UNB+UNOC:9+S+R+{date}:0900+1"));
        assert_eq!(codes(&input), ["E059"], "{input}");
    }
}

// ── group counts ─────────────────────────────────────────────────────────────

const GROUP: &str = "UNG+ORDERS+S+R+260101:0900+G1+UN+D:96A'";

#[test]
fn a_wrong_group_count_is_e054_on_the_une() {
    let input = format!("UNB+UNOA:3+S+R+260101:0900+1'{GROUP}{BODY}UNE+5+G1'UNZ+1+1'");
    let found = errors(&input);
    assert_eq!(
        found,
        [EdifactError::GroupCountMismatch {
            group_ref: "G1".into(),
            expected: 5,
            actual: 1,
            span: segment_span(&input, "UNE+5+G1"),
        }]
    );
    assert!(found[0].to_string().contains("UNE"), "{}", found[0]);
}

#[test]
fn a_wrong_interchange_count_is_e004_on_the_unz() {
    let input = format!("UNB+UNOA:3+S+R+260101:0900+1'{BODY}UNZ+5+1'");
    assert_eq!(
        errors(&input),
        [EdifactError::MessageCountMismatch {
            expected: 5,
            actual: 1,
            span: segment_span(&input, "UNZ+5+1"),
        }]
    );
}

#[test]
fn an_empty_group_is_e061_on_the_ung() {
    let input = format!("UNB+UNOA:3+S+R+260101:0900+1'{GROUP}UNE+0+G1'UNZ+1+1'");
    assert_eq!(
        errors(&input),
        [EdifactError::EmptyGroup {
            group_ref: "G1".into(),
            span: segment_span(&input, GROUP.trim_end_matches('\'')),
        }]
    );
}

#[test]
fn a_contrl_reports_the_group_count_on_the_ucf() {
    let input = format!("UNB+UNOA:3+S+R+260101:0900+1'{GROUP}{BODY}UNE+5+G1'UNZ+1+1'");
    let segments = parse(input.as_bytes());
    let validated = validate_envelope_lenient(&segments)
        .interchange
        .expect("interpretable");
    let report = ValidationContext::builder()
        .with_envelope_validation()
        .build()
        .validate(&segments);
    let contrl = Contrl::from_report(&validated, &segments, &report)
        .with_message_reference("ACK1")
        .to_edifact_string()
        .expect("renders");
    assert!(contrl.contains("UCF+G1+S+R+4+29+UNE"), "{contrl}");
}

// ── refused and reserved tags ────────────────────────────────────────────────

#[test]
fn interactive_segments_inside_an_interchange_are_e055() {
    let input = "UNB+UNOA:3+S+R+260101:0900+1'UIH+1+X'UIT+1'UNZ+0+1'";
    let found = errors(input);
    assert_eq!(
        found[0],
        EdifactError::InteractiveSegmentNotSupported {
            tag: "UIH".into(),
            span: segment_span(input, "UIH+1+X"),
        }
    );
    assert!(
        !found.iter().any(|e| e.stable_code() == "E011"),
        "{found:?}"
    );
}

#[test]
fn an_interactive_interchange_is_e055_not_a_missing_unb() {
    let input = "UIB+UNOA:4+1+S+R+20260101:0900'UIZ++1'";
    assert_eq!(
        errors(input),
        [EdifactError::InteractiveSegmentNotSupported {
            tag: "UIB".into(),
            span: segment_span(input, "UIB+UNOA:4+1+S+R+20260101:0900"),
        }]
    );
}

#[test]
fn a_security_segment_is_e056_inside_a_message_and_before_it() {
    let inside =
        "UNB+UNOA:3+S+R+260101:0900+1'UNH+1+ORDERS:D:96A:UN'USH+7+1'BGM+220'UNT+4+1'UNZ+1+1'";
    assert_eq!(
        errors(inside),
        [EdifactError::SecuritySegmentNotSupported {
            tag: "USH".into(),
            span: segment_span(inside, "USH+7+1"),
        }]
    );
    let before = format!("UNB+UNOA:3+S+R+260101:0900+1'USH+7+1'{BODY}UNZ+1+1'");
    assert_eq!(codes(&before), ["E056"]);
}

#[test]
fn a_reserved_u_tag_is_e057() {
    let input = "UNB+UNOA:3+S+R+260101:0900+1'UNH+1+ORDERS:D:96A:UN'UXX+1'BGM+220'UNT+4+1'UNZ+1+1'";
    assert_eq!(
        errors(input),
        [EdifactError::ReservedServiceSegmentTag {
            tag: "UXX".into(),
            span: segment_span(input, "UXX+1"),
        }]
    );
}

#[test]
fn batch_service_segments_inside_a_message_are_not_refused() {
    for tag in [
        "UNS+D",
        "UGH+1",
        "UGT+1",
        "UCI+1+S+R+7",
        "UCM+1+X:D:96A:UN+7",
    ] {
        let input = format!(
            "UNB+UNOA:3+S+R+260101:0900+1'UNH+1+ORDERS:D:96A:UN'BGM+220'{tag}'UNT+4+1'UNZ+1+1'"
        );
        let found = codes(&input);
        assert!(
            !found.iter().any(|c| matches!(*c, "E055" | "E056" | "E057")),
            "{input}: {found:?}"
        );
    }
}

#[test]
fn every_refused_tag_fails_validate_envelope() {
    for tag in [
        "UIB", "UIH", "UIR", "UIT", "UIZ", "USA", "USB", "USC", "USD", "USE", "USF", "USH", "USL",
        "USR", "UST", "USU", "USX", "USY", "UAA", "UZZ",
    ] {
        let input = format!(
            "UNB+UNOA:3+S+R+260101:0900+1'UNH+1+ORDERS:D:96A:UN'BGM+220'{tag}+1'UNT+4+1'UNZ+1+1'"
        );
        let expected = match &tag[..2] {
            "UI" => "E055",
            "US" => "E056",
            _ => "E057",
        };
        let err = validate_envelope(&parse(input.as_bytes())).expect_err(tag);
        assert_eq!(err.stable_code(), expected, "{tag}");
        assert_eq!(
            edifact::severity_for_error(&err),
            edifact::ValidationSeverity::Error
        );
    }
}

// ── data after the interchange trailer ───────────────────────────────────────

const FIRST: &str =
    "UNB+UNOA:3+S+R+260101:0900+IC1'UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+3+1'UNZ+1+IC1'";

#[test]
fn a_second_interchange_is_e058_only() {
    for second_ref in ["IC2", "IC1"] {
        let second = format!(
            "UNB+UNOA:3+S+R+260101:0900+{second_ref}'UNH+9+ORDERS:D:96A:UN'BGM+220'UNT+3+9'UNZ+1+{second_ref}'"
        );
        let input = format!("{FIRST}{second}");
        assert_eq!(
            errors(&input),
            [EdifactError::DataAfterInterchangeTrailer {
                tag: "UNB".into(),
                span: Span::new(FIRST.len(), FIRST.len() + second.find('\'').unwrap() + 1),
            }]
        );
        let lenient = validate_envelope_lenient(&parse(input.as_bytes()));
        let only = lenient.interchange.expect("the first interchange");
        assert_eq!(only.interchange.header.control_ref, "IC1");
        assert_eq!(
            only.messages
                .iter()
                .map(|m| m.message_ref.as_str())
                .collect::<Vec<_>>(),
            ["1"]
        );
    }
}

#[test]
fn a_stray_segment_after_unz_is_e058() {
    let input = format!("{FIRST}BGM+1'");
    assert_eq!(codes(&input), ["E058"]);
}

#[test]
fn trailing_whitespace_is_not_e058() {
    let input = format!("{FIRST}\r\n \n");
    assert_eq!(codes(&input), [""; 0]);
}

#[test]
fn a_unz_inside_a_message_is_still_e011() {
    let input =
        "UNB+UNOA:3+S+R+260101:0900+1'UNH+1+ORDERS:D:96A:UN'UNZ+1+1'BGM+220'UNT+4+1'UNZ+1+1'";
    let found = codes(input);
    assert!(found.contains(&"E011"), "{found:?}");
    assert!(!found.contains(&"E058"), "{found:?}");
}

#[test]
fn a_una_starting_the_second_interchange_is_e058_on_every_path() {
    let input = format!("{FIRST}UNA:+.? 'UNB+UNOA:3+S+R+260101:0900+IC2'");
    let expected = EdifactError::DataAfterInterchangeTrailer {
        tag: "UNA".into(),
        span: Span::new(FIRST.len(), FIRST.len() + 3),
    };
    let bytes = input.as_bytes();
    assert_eq!(
        from_bytes(bytes)
            .collect::<Result<Vec<_>, _>>()
            .unwrap_err(),
        expected
    );
    for chunk in [1, 2, 3, 7, 64] {
        let reader = Chunked { data: bytes, chunk };
        assert_eq!(
            from_reader(reader)
                .collect::<Result<Vec<_>, _>>()
                .unwrap_err(),
            expected,
            "chunk {chunk}"
        );
    }
    let faulted = scan_recovering(bytes, &RecoveryConfig::default())
        .find_map(|item| match item {
            Scanned::Fault(fault) => Some(fault.error() == &expected),
            _ => None,
        })
        .expect("a fault");
    assert!(faulted, "the recovering scan reports another fault");
}

/// A reader that hands out at most `chunk` bytes per call.
struct Chunked<'a> {
    data: &'a [u8],
    chunk: usize,
}

impl std::io::Read for Chunked<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.chunk.min(buf.len()).min(self.data.len());
        buf[..n].copy_from_slice(&self.data[..n]);
        self.data = &self.data[n..];
        Ok(n)
    }
}

// ── envelope and DirectoryValidator(service::lookup) agree ───────────────────

#[path = "common/service_findings.rs"]
mod service_findings;
use service_findings::{assert_same_service_findings, service_directory};

#[test]
fn envelope_and_directory_validator_report_the_same_service_findings() {
    let versions = ["1", "2", "3", "4", "9", "X", ""];
    let dates = ["260101", "20260101", "2601", "26010A"];
    let refs = ["IC1", "AB?+CDEFGHIJKLMN", "ABCDEFGHIJKLMNO", ""];
    let senders = [
        "S",
        "Müller Möbel Großhandel Jürgen Ölmü",
        "S::ABCDEFGHIJKLMNO",
    ];
    for version in versions {
        for date in dates {
            for reference in refs {
                for sender in senders {
                    let input = format!(
                        "UNB+UNOC:{version}+{sender}+R+{date}:0900+{reference}'\
UNG+ORDERS+S+R+{date}:0900+G1+UNX+D:96A'\
UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+3+1'\
UNE+1+G1'UNZ+1+{reference}'"
                    );
                    assert_same_service_findings(input.as_bytes());
                }
            }
        }
    }
}
