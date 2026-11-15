//! `MessageBuilder`: owned segments equal to what `MessageWriter` writes.

use edifact::{
    Charset, DataElement, EdifactError, Element, MessageBuilder, MessageIdentifier, OwnedSegment,
    Segment, ServiceStringAdvice, Writer, from_bytes_with_config,
};

const ID: MessageIdentifier<'static> = MessageIdentifier::new("1", "ORDERS", "D", "96A", "UN");

fn v4() -> ServiceStringAdvice {
    ServiceStringAdvice::for_syntax_version(Some(4))
}

fn repeating() -> Segment<'static> {
    Segment::new(
        "RFF",
        vec![
            Element::of(&["ON", "1"])
                .and_repeat(&["ON", ""])
                .and_repeat(&["", ""]),
        ],
    )
}

/// The same calls through `MessageWriter` and `MessageBuilder`.
fn both() -> (Vec<u8>, Vec<OwnedSegment>) {
    let mut w = Writer::with_service_string_advice(Vec::new(), v4()).unwrap();
    let mut b = MessageBuilder::new(ID)
        .unwrap()
        .with_service_string_advice(v4())
        .unwrap();
    {
        let mut m = w.begin_message(ID).unwrap();
        m.write_simple("BGM", &["220", "", "9", ""]).unwrap();
        b.write_simple("BGM", &["220", "", "9", ""]).unwrap();
        m.write_composites("DTM", &[&["137", "20260101", ""][..], &[""][..]])
            .unwrap();
        b.write_composites("DTM", &[&["137", "20260101", ""][..], &[""][..]])
            .unwrap();
        let mixed = [
            DataElement::Simple("MS"),
            DataElement::Composite(&["99?+00", "", "293"]),
            DataElement::Composite(&["", ""]),
        ];
        m.write_elements("NAD", &mixed).unwrap();
        b.write_elements("NAD", &mixed).unwrap();
        m.write_segment(&repeating()).unwrap();
        b.write_segment(&repeating()).unwrap();
        m.finish().unwrap();
    }
    (w.finish().unwrap(), b.finish().unwrap())
}

fn without_spans(seg: &Segment<'_>) -> Vec<Vec<Vec<String>>> {
    seg.elements
        .iter()
        .map(|e| {
            e.repetitions()
                .map(|occ| occ.iter().map(|(c, _)| c.to_string()).collect())
                .collect()
        })
        .collect()
}

#[test]
fn owned_segments_render_to_the_message_writer_bytes() {
    let (bytes, owned) = both();
    let mut w = Writer::with_service_string_advice(Vec::new(), v4()).unwrap();
    for seg in &owned {
        w.write_segment(seg).unwrap();
    }
    assert_eq!(w.finish().unwrap(), bytes);

    let config = edifact::ReaderConfig::default().with_service_string_advice(v4());
    let parsed: Vec<_> = from_bytes_with_config(&bytes, config)
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(parsed.len(), owned.len());
    for (p, o) in parsed.iter().zip(&owned) {
        assert_eq!(p.tag, o.tag);
        assert_eq!(without_spans(p), without_spans(o));
    }
}

#[test]
fn unh_carries_the_association_code_and_unt_is_computed() {
    let id = MessageIdentifier::new("1", "UTILMD", "D", "11A", "UN").with_association_code("S2.1");
    let mut b = MessageBuilder::new(id).unwrap();
    b.write_simple("BGM", &["E01"]).unwrap();
    b.write_simple("DTM", &["137"]).unwrap();
    let segments = b.finish().unwrap();
    assert_eq!(
        edifact::segments_to_bytes(&segments).unwrap(),
        b"UNH+1+UTILMD:D:11A:UN:S2.1'BGM+E01'DTM+137'UNT+4+1'".to_vec()
    );
}

#[test]
fn refusals_change_nothing() {
    let mut b = MessageBuilder::new(ID).unwrap().with_charset(Charset::UnoC);
    b.write_simple("BGM", &["220"]).unwrap();
    let before = b.segment_count();
    let refused: [Result<(), EdifactError>; 3] = [
        b.write_simple("bgm", &["220"]),
        b.write_segment(&repeating()),
        b.write_simple("FTX", &["AAA", "5 €"]),
    ];
    let codes: Vec<_> = refused
        .into_iter()
        .map(|r| r.unwrap_err().stable_code())
        .collect();
    assert_eq!(codes, ["E006", "E037", "E038"]);
    assert_eq!(b.segment_count(), before);

    // The writer raises the same codes.
    let mut w = Writer::new(Vec::new()).with_charset(Charset::UnoC);
    assert_eq!(
        w.write_simple("bgm", &["220"]).unwrap_err().stable_code(),
        "E006"
    );
    assert_eq!(
        w.write_segment(&repeating()).unwrap_err().stable_code(),
        "E037"
    );
    assert_eq!(
        w.write_simple("FTX", &["AAA", "5 €"])
            .unwrap_err()
            .stable_code(),
        "E038"
    );
}

#[test]
fn an_empty_body_is_e043() {
    let b = MessageBuilder::new(ID).unwrap();
    assert_eq!(b.finish().unwrap_err().stable_code(), "E043");
}

#[test]
fn an_invalid_service_string_advice_is_e007() {
    let bad = ServiceStringAdvice {
        element_sep: b':',
        ..ServiceStringAdvice::default()
    };
    let err = MessageBuilder::new(ID)
        .unwrap()
        .with_service_string_advice(bad)
        .unwrap_err();
    assert_eq!(err.stable_code(), "E007");
}
