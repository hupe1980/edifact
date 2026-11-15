#![cfg(feature = "derive")]

//! No write path emits trailing empty data elements or components
//! (ISO 9735-1:2002 §8.7.1, §8.7.2); interior empties keep their separators.

use edifact::{
    DataElement, EdifactEvent, Element, EventEmitter, Segment, ValidationContext, Writer,
    WriterEmitter, from_bytes,
};

#[derive(edifact::EdifactSerialize)]
#[edifact(segment = "BGM")]
struct Bgm {
    code: String,
    number: String,
    function: Option<String>,
}

/// Syntax findings of `bytes` that concern trailing separators.
fn trailing_findings(bytes: &[u8]) -> usize {
    let segments: Vec<_> = from_bytes(bytes).collect::<Result<_, _>>().unwrap();
    let report = ValidationContext::builder()
        .with_syntax_validation()
        .build()
        .validate(&segments);
    report
        .errors()
        .iter()
        .chain(report.warnings())
        .filter(|i| i.error_code.as_deref() == Some("E051"))
        .count()
}

#[test]
fn ser_drops_an_absent_last_field() {
    let bgm = Bgm {
        code: "220".into(),
        number: "PO".into(),
        function: None,
    };
    let bytes = edifact::to_bytes(&bgm).unwrap();
    assert_eq!(bytes, b"BGM+220+PO'");
    assert_eq!(trailing_findings(&bytes), 0);
    // A trailing separator is a finding.
    assert_eq!(trailing_findings(b"BGM+220+PO+'"), 1);
}

#[test]
fn an_all_empty_trailing_composite_is_dropped() {
    let mut w = Writer::new(Vec::new());
    w.write_composites("NAD", &[&["BY"][..], &["", "", ""][..]])
        .unwrap();
    w.write_elements(
        "DTM",
        &[DataElement::Composite(&["137", "20260101", "", ""])],
    )
    .unwrap();
    assert_eq!(w.finish().unwrap(), b"NAD+BY'DTM+137:20260101'");
}

#[test]
fn interior_empties_keep_their_separators() {
    let mut w = Writer::new(Vec::new());
    w.write_simple("BGM", &["220", "", "9"]).unwrap();
    w.write_composites("NAD", &[&["MS", ""][..], &["", "", "293"][..]])
        .unwrap();
    w.write_segment(&Segment::new(
        "FTX",
        vec![Element::of(&[""]), Element::of(&["", "X"])],
    ))
    .unwrap();
    assert_eq!(w.finish().unwrap(), b"BGM+220++9'NAD+MS+::293'FTX++:X'");
}

#[test]
fn the_event_emitter_truncates_too() {
    let mut e = WriterEmitter::new(Vec::new());
    e.emit(EdifactEvent::start("NAD")).unwrap();
    e.emit(EdifactEvent::element("MS")).unwrap();
    e.emit(EdifactEvent::element("ID")).unwrap();
    e.emit(EdifactEvent::component("")).unwrap();
    e.emit(EdifactEvent::element("")).unwrap();
    e.emit(EdifactEvent::EndSegment).unwrap();
    assert_eq!(e.finish().unwrap(), b"NAD+MS+ID'");
}
