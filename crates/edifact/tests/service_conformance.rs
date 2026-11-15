//! The `service` definitions against a transcription of the standard.
//!
//! Every batch service-segment position — status, maximum occurrences and
//! representation for syntax versions 1–3 and 4 — transcribed by hand from the
//! JWG1 publication of ISO 9735-10 for syntax 4 release 3 (UN D.24A "Syntax
//! plus", generated 2025-05-27, <https://www.gefeg.com/jswg/>; retrieved
//! 2026-10-18):
//!
//! - segments `Current/se/se1.htm`…`se35.htm` (ISO 9735-10:2002 §5.1.5);
//! - data elements `Current/de/dl1.htm`, including "Format for Syntax 3"
//!   (ISO 9735-10:2002 §5.2.5, §5.3.5).
//!
//! Syntax versions 1 and 2 are not published separately and use the version 3
//! formats. `UNO` positions 050–080 are interactive-only (ISO 9735-10:2002 UNO
//! note 4) and not transcribed.

use edifact::{Repr, SegmentDefinition, Status, service};

const M: Status = Status::Mandatory;
const C: Status = Status::Conditional;

/// (segment, element position, component position (0 = the element itself),
/// data element, status, maximum occurrences (elements only), representation
/// in syntax versions 1–3, in syntax version 4; "" for a composite).
type Row = (
    &'static str,
    u8,
    u8,
    &'static str,
    Status,
    u8,
    &'static str,
    &'static str,
);

const TRANSCRIPTION: &[Row] = &[
    ("UNB", 1, 0, "S001", M, 1, "", ""),
    ("UNB", 1, 1, "0001", M, 0, "a4", "a4"),
    ("UNB", 1, 2, "0002", M, 0, "an1", "an1"),
    ("UNB", 1, 3, "0080", C, 0, "an..6", "an..6"),
    ("UNB", 1, 4, "0133", C, 0, "an..3", "an..3"),
    ("UNB", 1, 5, "0076", C, 0, "an2", "an2"),
    ("UNB", 2, 0, "S002", M, 1, "", ""),
    ("UNB", 2, 1, "0004", M, 0, "an..35", "an..35"),
    ("UNB", 2, 2, "0007", C, 0, "an..4", "an..4"),
    ("UNB", 2, 3, "0008", C, 0, "an..14", "an..35"),
    ("UNB", 2, 4, "0042", C, 0, "an..35", "an..35"),
    ("UNB", 3, 0, "S003", M, 1, "", ""),
    ("UNB", 3, 1, "0010", M, 0, "an..35", "an..35"),
    ("UNB", 3, 2, "0007", C, 0, "an..4", "an..4"),
    ("UNB", 3, 3, "0014", C, 0, "an..14", "an..35"),
    ("UNB", 3, 4, "0046", C, 0, "an..35", "an..35"),
    ("UNB", 4, 0, "S004", M, 1, "", ""),
    ("UNB", 4, 1, "0017", M, 0, "n6", "n8"),
    ("UNB", 4, 2, "0019", M, 0, "n4", "n4"),
    ("UNB", 5, 0, "0020", M, 1, "an..14", "an..14"),
    ("UNB", 6, 0, "S005", C, 1, "", ""),
    ("UNB", 6, 1, "0022", M, 0, "an..14", "an..14"),
    ("UNB", 6, 2, "0025", C, 0, "an2", "an2"),
    ("UNB", 7, 0, "0026", C, 1, "an..14", "an..14"),
    ("UNB", 8, 0, "0029", C, 1, "a1", "a1"),
    ("UNB", 9, 0, "0031", C, 1, "n1", "n1"),
    ("UNB", 10, 0, "0032", C, 1, "an..35", "an..35"),
    ("UNB", 11, 0, "0035", C, 1, "n1", "n1"),
    ("UNZ", 1, 0, "0036", M, 1, "n..6", "n..6"),
    ("UNZ", 2, 0, "0020", M, 1, "an..14", "an..14"),
    ("UNG", 1, 0, "0038", C, 1, "an..6", "an..6"),
    ("UNG", 2, 0, "S006", C, 1, "", ""),
    ("UNG", 2, 1, "0040", M, 0, "an..35", "an..35"),
    ("UNG", 2, 2, "0007", C, 0, "an..4", "an..4"),
    ("UNG", 3, 0, "S007", C, 1, "", ""),
    ("UNG", 3, 1, "0044", M, 0, "an..35", "an..35"),
    ("UNG", 3, 2, "0007", C, 0, "an..4", "an..4"),
    ("UNG", 4, 0, "S004", C, 1, "", ""),
    ("UNG", 4, 1, "0017", M, 0, "n6", "n8"),
    ("UNG", 4, 2, "0019", M, 0, "n4", "n4"),
    ("UNG", 5, 0, "0048", M, 1, "an..14", "an..14"),
    ("UNG", 6, 0, "0051", C, 1, "an..2", "an..3"),
    ("UNG", 7, 0, "S008", C, 1, "", ""),
    ("UNG", 7, 1, "0052", M, 0, "an..3", "an..3"),
    ("UNG", 7, 2, "0054", M, 0, "an..3", "an..3"),
    ("UNG", 7, 3, "0057", C, 0, "an..6", "an..6"),
    ("UNG", 8, 0, "0058", C, 1, "an..14", "an..14"),
    ("UNE", 1, 0, "0060", M, 1, "n..6", "n..6"),
    ("UNE", 2, 0, "0048", M, 1, "an..14", "an..14"),
    ("UNH", 1, 0, "0062", M, 1, "an..14", "an..14"),
    ("UNH", 2, 0, "S009", M, 1, "", ""),
    ("UNH", 2, 1, "0065", M, 0, "an..6", "an..6"),
    ("UNH", 2, 2, "0052", M, 0, "an..3", "an..3"),
    ("UNH", 2, 3, "0054", M, 0, "an..3", "an..3"),
    ("UNH", 2, 4, "0051", M, 0, "an..2", "an..3"),
    ("UNH", 2, 5, "0057", C, 0, "an..6", "an..6"),
    ("UNH", 2, 6, "0110", C, 0, "an..6", "an..6"),
    ("UNH", 2, 7, "0113", C, 0, "an..6", "an..6"),
    ("UNH", 3, 0, "0068", C, 1, "an..35", "an..35"),
    ("UNH", 4, 0, "S010", C, 1, "", ""),
    ("UNH", 4, 1, "0070", M, 0, "n..2", "n..2"),
    ("UNH", 4, 2, "0073", C, 0, "a1", "a1"),
    ("UNH", 5, 0, "S016", C, 1, "", ""),
    ("UNH", 5, 1, "0115", M, 0, "an..14", "an..14"),
    ("UNH", 5, 2, "0116", C, 0, "an..3", "an..3"),
    ("UNH", 5, 3, "0118", C, 0, "an..3", "an..3"),
    ("UNH", 5, 4, "0051", C, 0, "an..2", "an..3"),
    ("UNH", 6, 0, "S017", C, 1, "", ""),
    ("UNH", 6, 1, "0121", M, 0, "an..14", "an..14"),
    ("UNH", 6, 2, "0122", C, 0, "an..3", "an..3"),
    ("UNH", 6, 3, "0124", C, 0, "an..3", "an..3"),
    ("UNH", 6, 4, "0051", C, 0, "an..2", "an..3"),
    ("UNH", 7, 0, "S018", C, 1, "", ""),
    ("UNH", 7, 1, "0127", M, 0, "an..14", "an..14"),
    ("UNH", 7, 2, "0128", C, 0, "an..3", "an..3"),
    ("UNH", 7, 3, "0130", C, 0, "an..3", "an..3"),
    ("UNH", 7, 4, "0051", C, 0, "an..2", "an..3"),
    ("UNT", 1, 0, "0074", M, 1, "n..6", "n..10"),
    ("UNT", 2, 0, "0062", M, 1, "an..14", "an..14"),
    ("UNS", 1, 0, "0081", M, 1, "a1", "a1"),
    ("UNO", 1, 0, "0800", M, 1, "an..35", "an..35"),
    ("UNO", 2, 0, "S020", M, 99, "", ""),
    ("UNO", 2, 1, "0813", M, 0, "an..3", "an..3"),
    ("UNO", 2, 2, "0802", M, 0, "an..35", "an..35"),
    ("UNO", 3, 0, "S021", M, 99, "", ""),
    ("UNO", 3, 1, "0805", M, 0, "an..3", "an..3"),
    ("UNO", 3, 2, "0809", C, 0, "an..256", "an..256"),
    ("UNO", 3, 3, "0808", C, 0, "an..256", "an..256"),
    ("UNO", 3, 4, "0051", C, 0, "an..2", "an..3"),
    ("UNO", 4, 0, "S022", M, 1, "", ""),
    ("UNO", 4, 1, "0810", M, 0, "n..18", "n..18"),
    ("UNO", 4, 2, "0814", C, 0, "n..3", "n..3"),
    ("UNO", 4, 3, "0070", C, 0, "n..2", "n..2"),
    ("UNO", 4, 4, "0073", C, 0, "a1", "a1"),
    ("UNP", 1, 0, "0810", M, 1, "n..18", "n..18"),
    ("UNP", 2, 0, "0800", M, 1, "an..35", "an..35"),
    ("UGH", 1, 0, "0087", M, 1, "an..4", "an..4"),
    ("UGT", 1, 0, "0087", M, 1, "an..4", "an..4"),
    ("UCI", 1, 0, "0020", M, 1, "an..14", "an..14"),
    ("UCI", 2, 0, "S002", M, 1, "", ""),
    ("UCI", 2, 1, "0004", M, 0, "an..35", "an..35"),
    ("UCI", 2, 2, "0007", C, 0, "an..4", "an..4"),
    ("UCI", 2, 3, "0008", C, 0, "an..14", "an..35"),
    ("UCI", 2, 4, "0042", C, 0, "an..35", "an..35"),
    ("UCI", 3, 0, "S003", M, 1, "", ""),
    ("UCI", 3, 1, "0010", M, 0, "an..35", "an..35"),
    ("UCI", 3, 2, "0007", C, 0, "an..4", "an..4"),
    ("UCI", 3, 3, "0014", C, 0, "an..14", "an..35"),
    ("UCI", 3, 4, "0046", C, 0, "an..35", "an..35"),
    ("UCI", 4, 0, "0083", M, 1, "an..3", "an..3"),
    ("UCI", 5, 0, "0085", C, 1, "an..3", "an..3"),
    ("UCI", 6, 0, "0135", C, 1, "a3", "an..3"),
    ("UCI", 7, 0, "S011", C, 1, "", ""),
    ("UCI", 7, 1, "0098", M, 0, "n..3", "n..3"),
    ("UCI", 7, 2, "0104", C, 0, "n..3", "n..3"),
    ("UCI", 7, 3, "0136", C, 0, "n..6", "n..6"),
    ("UCI", 8, 0, "0534", C, 1, "an..14", "an..14"),
    ("UCI", 9, 0, "0138", C, 1, "n..6", "n..6"),
    ("UCF", 1, 0, "0048", M, 1, "an..14", "an..14"),
    ("UCF", 2, 0, "S006", C, 1, "", ""),
    ("UCF", 2, 1, "0040", M, 0, "an..35", "an..35"),
    ("UCF", 2, 2, "0007", C, 0, "an..4", "an..4"),
    ("UCF", 3, 0, "S007", C, 1, "", ""),
    ("UCF", 3, 1, "0044", M, 0, "an..35", "an..35"),
    ("UCF", 3, 2, "0007", C, 0, "an..4", "an..4"),
    ("UCF", 4, 0, "0083", M, 1, "an..3", "an..3"),
    ("UCF", 5, 0, "0085", C, 1, "an..3", "an..3"),
    ("UCF", 6, 0, "0135", C, 1, "a3", "an..3"),
    ("UCF", 7, 0, "S011", C, 1, "", ""),
    ("UCF", 7, 1, "0098", M, 0, "n..3", "n..3"),
    ("UCF", 7, 2, "0104", C, 0, "n..3", "n..3"),
    ("UCF", 7, 3, "0136", C, 0, "n..6", "n..6"),
    ("UCF", 8, 0, "0534", C, 1, "an..14", "an..14"),
    ("UCF", 9, 0, "0138", C, 1, "n..6", "n..6"),
    ("UCM", 1, 0, "0062", C, 1, "an..14", "an..14"),
    ("UCM", 2, 0, "S009", C, 1, "", ""),
    ("UCM", 2, 1, "0065", M, 0, "an..6", "an..6"),
    ("UCM", 2, 2, "0052", M, 0, "an..3", "an..3"),
    ("UCM", 2, 3, "0054", M, 0, "an..3", "an..3"),
    ("UCM", 2, 4, "0051", M, 0, "an..2", "an..3"),
    ("UCM", 2, 5, "0057", C, 0, "an..6", "an..6"),
    ("UCM", 2, 6, "0110", C, 0, "an..6", "an..6"),
    ("UCM", 2, 7, "0113", C, 0, "an..6", "an..6"),
    ("UCM", 3, 0, "0083", M, 1, "an..3", "an..3"),
    ("UCM", 4, 0, "0085", C, 1, "an..3", "an..3"),
    ("UCM", 5, 0, "0135", C, 1, "a3", "an..3"),
    ("UCM", 6, 0, "S011", C, 1, "", ""),
    ("UCM", 6, 1, "0098", M, 0, "n..3", "n..3"),
    ("UCM", 6, 2, "0104", C, 0, "n..3", "n..3"),
    ("UCM", 6, 3, "0136", C, 0, "n..6", "n..6"),
    ("UCM", 7, 0, "0800", C, 1, "an..35", "an..35"),
    ("UCM", 8, 0, "S020", C, 99, "", ""),
    ("UCM", 8, 1, "0813", M, 0, "an..3", "an..3"),
    ("UCM", 8, 2, "0802", M, 0, "an..35", "an..35"),
    ("UCM", 9, 0, "0534", C, 1, "an..14", "an..14"),
    ("UCM", 10, 0, "0138", C, 1, "n..6", "n..6"),
    ("UCS", 1, 0, "0096", M, 1, "n..6", "n..6"),
    ("UCS", 2, 0, "0085", C, 1, "an..3", "an..3"),
    ("UCD", 1, 0, "0085", M, 1, "an..3", "an..3"),
    ("UCD", 2, 0, "S011", M, 1, "", ""),
    ("UCD", 2, 1, "0098", M, 0, "n..3", "n..3"),
    ("UCD", 2, 2, "0104", C, 0, "n..3", "n..3"),
    ("UCD", 2, 3, "0136", C, 0, "n..6", "n..6"),
];

fn repr(base: Option<Repr>) -> String {
    base.map(|r| r.to_string()).unwrap_or_default()
}

fn rows_of(definition: &SegmentDefinition) -> Vec<Row> {
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    let mut rows = Vec::new();
    for element in definition.elements {
        let base = element.repr();
        rows.push((
            definition.tag,
            element.position(),
            0,
            element.data_element(),
            element.status(),
            element.max_repeat(),
            leak(repr(base)),
            leak(repr(element.repr_from_v4().or(base))),
        ));
        for component in element.components() {
            let base = component.repr();
            rows.push((
                definition.tag,
                element.position(),
                component.position(),
                component.data_element(),
                component.status(),
                0,
                leak(repr(base)),
                leak(repr(component.repr_from_v4().or(base))),
            ));
        }
    }
    rows
}

#[test]
fn every_batch_service_position_matches_the_transcription() {
    let shipped: Vec<Row> = service::ALL.iter().flat_map(|d| rows_of(d)).collect();
    let mismatches: Vec<String> = TRANSCRIPTION
        .iter()
        .filter(|row| !shipped.contains(row))
        .map(|row| format!("standard {row:?}"))
        .chain(
            shipped
                .iter()
                .filter(|row| !TRANSCRIPTION.contains(row))
                .map(|row| format!("shipped  {row:?}")),
        )
        .collect();
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    assert_eq!(TRANSCRIPTION.len(), 163);
    assert_eq!(shipped.len(), 163);
}

#[test]
fn a_syntax_release_number_in_s001_is_accepted() {
    use edifact::{DirectoryValidator, ValidationContext, ValidationLayer, from_bytes};
    let validator = DirectoryValidator::new(
        "service",
        service::lookup,
        |_, _| true,
        |_, _| None,
        |_, _| None,
        None,
    );
    let context = ValidationContext::builder()
        .with_validator(ValidationLayer::Structure, validator)
        .build();
    for input in [
        &b"UNB+UNOA:4:40101::01+S+R+20260101:0900+1'"[..],
        // `UCM` 080 S020 occurs up to 99 times.
        b"UNA:+.?*'UCM+1+ORDERS:D:96A:UN+7+++++AAA:1*AAA:2'",
    ] {
        let segments: Vec<_> = from_bytes(input).collect::<Result<_, _>>().expect("parses");
        let report = context.validate(&segments);
        assert!(report.errors().is_empty(), "{:?}", report.errors());
    }
}
