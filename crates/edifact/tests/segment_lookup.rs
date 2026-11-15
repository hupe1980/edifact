//! Segment lookup: `Segment::is` and `SegmentSliceExt`, on borrowed and owned
//! segments alike.

use edifact::{OwnedSegment, Segment, SegmentSliceExt, from_bytes, from_reader};

const INPUT: &[u8] = b"UNH+1+ORDERS:D:11A:UN'BGM+220+PO-4711+9'NAD+BY+4000001::9'\
NAD+SU+4000002::9'RFF+Z13:17009'UNT+6+1'";

fn borrowed(input: &[u8]) -> Vec<Segment<'_>> {
    from_bytes(input).collect::<Result<_, _>>().expect("parse")
}

fn owned(input: &[u8]) -> Vec<OwnedSegment> {
    from_reader(input).collect::<Result<_, _>>().expect("parse")
}

/// The answers every scenario checks, as plain values.
fn answers(segs: &[Segment<'_>]) -> Vec<Option<String>> {
    let text = |s: Option<&str>| s.map(str::to_owned);
    vec![
        text(segs.first_tag("BGM").and_then(|s| s.element_str(1))),
        text(segs.first_tag("DTM").and_then(|s| s.element_str(0))),
        text(segs.qualified("NAD", "SU").and_then(|s| s.element_str(1))),
        text(segs.qualified("NAD", "DP").and_then(|s| s.element_str(1))),
        text(segs.qualified_value("RFF", "Z13", 0, 1)),
        text(segs.qualified_value("RFF", "Z13", 0, 5)),
        text(segs.qualified_value("RFF", "Z13", 3, 0)),
        text(segs.qualified_value("RFF", "ACW", 0, 1)),
        Some(segs.tagged("NAD").count().to_string()),
        text(segs.tagged("NAD").next().and_then(|s| s.element_str(0))),
        segs.iter()
            .position(|s| s.is("NAD", "SU"))
            .map(|i| i.to_string()),
    ]
}

#[test]
fn lookups_answer_the_quickstart_scenarios() {
    let segs = borrowed(INPUT);
    let expected = [
        Some("PO-4711"),
        None,
        Some("4000002"),
        None,
        Some("17009"),
        None,
        None,
        None,
        Some("2"),
        Some("BY"),
        Some("3"),
    ];
    assert_eq!(
        answers(&segs),
        expected.map(|v| v.map(str::to_owned)).to_vec()
    );
}

#[test]
fn owned_segments_answer_like_borrowed_ones() {
    assert_eq!(answers(&owned(INPUT)), answers(&borrowed(INPUT)));
}

#[test]
fn the_first_of_several_matches_wins() {
    let input = b"NAD+BY+1'NAD+SU+2'NAD+SU+3'";
    let borrowed = borrowed(input);
    let owned = owned(input);
    for segs in [&borrowed[..], &owned[..]] {
        assert_eq!(
            segs.qualified("NAD", "SU").unwrap().element_str(1),
            Some("2")
        );
        assert_eq!(segs.first_tag("NAD").unwrap().element_str(0), Some("BY"));
        let all: Vec<_> = segs.tagged("NAD").map(|s| s.element_str(1)).collect();
        assert_eq!(all, [Some("1"), Some("2"), Some("3")]);
    }
}

#[test]
fn an_empty_slice_finds_nothing() {
    let segs: Vec<Segment<'_>> = Vec::new();
    assert!(segs.first_tag("BGM").is_none());
    assert_eq!(segs.tagged("BGM").count(), 0);
    assert!(segs.qualified("NAD", "BY").is_none());
    assert!(segs.qualified_value("RFF", "Z13", 0, 1).is_none());
}

#[test]
fn is_needs_both_tag_and_qualifier() {
    let segs = borrowed(b"RFF+Z13:1'");
    let rff = &segs[0];
    assert!(rff.is("RFF", "Z13"));
    assert!(!rff.is("RFF", "Z14"));
    assert!(!rff.is("NAD", "Z13"));
    assert!(!rff.is("rff", "Z13"), "tags compare exactly");
    assert!(segs.first_tag("rff").is_none());
}

#[test]
fn an_absent_qualifier_compares_as_empty() {
    let uns = Segment::new("UNS", vec![]);
    assert!(uns.is("UNS", ""));
    assert!(!uns.is("UNS", "D"));
    let segs = [uns];
    assert!(segs.qualified("UNS", "").is_some());
}

#[test]
fn only_the_first_component_of_the_first_repetition_is_the_qualifier() {
    let segs = borrowed(b"UNA:+.?*'RFF+ON:1*Z13:2'");
    assert!(segs[0].is("RFF", "ON"));
    assert!(!segs[0].is("RFF", "Z13"));
    assert!(!segs[0].is("RFF", "1"));
}
