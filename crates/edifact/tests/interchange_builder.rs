//! `InterchangeBuilder` and the `Writer` interchange envelope.

use edifact::{
    Charset, EdifactError, InterchangeBuilder, InterchangeHeader, MessageBuilder,
    MessageIdentifier, OwnedSegment, Party, PreparationDateTime, ServiceStringAdvice,
    SyntaxVersion, Writer, from_bytes, from_bytes_decoded, validate_envelope,
};

fn header(version: SyntaxVersion) -> InterchangeHeader {
    InterchangeHeader::new(
        Charset::UnoC,
        version,
        Party::new("9900123456789").with_qualifier("500"),
        Party::new("9900987654321").with_qualifier("500"),
        PreparationDateTime::new(2026, 8, 2, 9, 15).unwrap(),
        "REF001",
    )
}

fn owned(reference: &str) -> Vec<OwnedSegment> {
    let mut msg = MessageBuilder::new(MessageIdentifier::new(
        reference, "ORDERS", "D", "96A", "UN",
    ))
    .unwrap();
    msg.write_simple("BGM", &["220", reference, "9"]).unwrap();
    msg.write_composites("DTM", &[&["137", "20260802", "102"]])
        .unwrap();
    msg.finish().unwrap()
}

fn text(reference: &str) -> String {
    format!("UNH+{reference}+ORDERS:D:96A:UN'BGM+220+{reference}+9'UNT+3+{reference}'")
}

fn code(result: Result<Vec<u8>, EdifactError>) -> &'static str {
    result.expect_err("refused").stable_code()
}

/// Parse and validate `bytes`; return the declared `UNZ` count.
fn validated(bytes: &[u8]) -> u32 {
    let segments = from_bytes_decoded(bytes).expect("reparses");
    let result = validate_envelope(&segments).expect("no finding");
    assert_eq!(
        edifact::validate_envelope_lenient(&segments).errors.len(),
        0
    );
    result.interchange.declared_unit_count
}

#[test]
fn v3_header_and_computed_trailer() {
    let bytes = InterchangeBuilder::new(header(SyntaxVersion::V3))
        .message_segments(owned("1"))
        .message(text("2"))
        .build()
        .unwrap();
    assert!(
        bytes.starts_with(b"UNB+UNOC:3+9900123456789:500+9900987654321:500+260802:0915+REF001'")
    );
    assert!(bytes.ends_with(b"UNZ+2+REF001'"));
    assert_eq!(validated(&bytes), 2);
}

#[test]
fn v4_writes_a_full_year_and_no_una_for_default_characters() {
    let bytes = InterchangeBuilder::new(header(SyntaxVersion::V4))
        .message_segments(owned("1"))
        .build()
        .unwrap();
    assert!(
        bytes.starts_with(b"UNB+UNOC:4+9900123456789:500+9900987654321:500+20260802:0915+REF001'")
    );
    validated(&bytes);
    // Version 3 characters under version 4 differ (no repetition separator):
    // a `UNA` says so.
    let bytes = InterchangeBuilder::new(header(SyntaxVersion::V4))
        .with_service_string_advice(ServiceStringAdvice::for_syntax_version(Some(3)))
        .message_segments(owned("1"))
        .build()
        .unwrap();
    assert!(bytes.starts_with(b"UNA:+.? 'UNB+UNOC:4"));
}

#[test]
fn no_message_is_e042() {
    assert_eq!(
        code(InterchangeBuilder::new(header(SyntaxVersion::V3)).build()),
        "E042"
    );
}

#[test]
fn optional_positions_land_in_place_without_trailing_separators() {
    let mut h = header(SyntaxVersion::V3)
        .with_application_ref("APP")
        .with_processing_priority("A")
        .with_acknowledgement_requested(true)
        .with_communications_agreement_id("AGR")
        .with_test_indicator(true);
    h.sender.internal_id = Some("RA".into());
    let bytes = InterchangeBuilder::new(h.clone())
        .message(text("1"))
        .build()
        .unwrap();
    assert!(bytes.starts_with(
        b"UNB+UNOC:3+9900123456789:500:RA+9900987654321:500+260802:0915+REF001++APP+A+1+AGR+1'"
    ));
    validated(&bytes);
    h.test_indicator = false;
    h.communications_agreement_id = None;
    let bytes = InterchangeBuilder::new(h)
        .message(text("1"))
        .build()
        .unwrap();
    assert!(bytes.starts_with(
        b"UNB+UNOC:3+9900123456789:500:RA+9900987654321:500+260802:0915+REF001++APP+A+1'"
    ));
}

#[test]
fn an_over_long_control_reference_is_e049() {
    let mut h = header(SyntaxVersion::V3);
    h.control_ref = "123456789012345".into();
    assert_eq!(
        code(InterchangeBuilder::new(h).message(text("1")).build()),
        "E049"
    );
}

#[test]
fn writer_and_builder_write_the_same_bytes() {
    let h = header(SyntaxVersion::V3);
    let built = InterchangeBuilder::new(h.clone())
        .message_segments(owned("1"))
        .message_segments(owned("2"))
        .build()
        .unwrap();
    let mut w = Writer::new(Vec::new());
    w.begin_interchange(&h).unwrap();
    for reference in ["1", "2"] {
        let mut msg = w
            .begin_message(MessageIdentifier::new(
                reference, "ORDERS", "D", "96A", "UN",
            ))
            .unwrap();
        msg.write_simple("BGM", &["220", reference, "9"]).unwrap();
        msg.write_composites("DTM", &[&["137", "20260802", "102"]])
            .unwrap();
        msg.finish().unwrap();
    }
    w.end_interchange().unwrap();
    assert_eq!(w.finish().unwrap(), built);
}

#[test]
fn matrix_validates_with_computed_counts() {
    let custom = ServiceStringAdvice {
        component_sep: b'|',
        element_sep: b'!',
        decimal_mark: b',',
        release_char: b'#',
        repetition_sep: b'*',
        segment_term: b'~',
    };
    let mut cases = 0;
    for version in [SyntaxVersion::V3, SyntaxVersion::V4] {
        for una in [false, true] {
            for count in [1usize, 2, 1000] {
                for as_text in [false, true] {
                    let mut builder = InterchangeBuilder::new(header(version));
                    if una {
                        let mut ssa = custom;
                        if version != SyntaxVersion::V4 {
                            ssa.repetition_sep = b' ';
                        }
                        builder = builder.with_service_string_advice(ssa);
                    }
                    for i in 0..count {
                        let reference = (i + 1).to_string();
                        builder = if as_text && !una {
                            builder.message(text(&reference))
                        } else if as_text {
                            builder.message(
                                text(&reference)
                                    .replace('+', "!")
                                    .replace(':', "|")
                                    .replace('\'', "~"),
                            )
                        } else {
                            builder.message_segments(owned(&reference))
                        };
                    }
                    let bytes = builder.build().unwrap();
                    assert_eq!(bytes.starts_with(b"UNA"), una);
                    assert_eq!(validated(&bytes) as usize, count);
                    cases += 1;
                }
            }
        }
    }
    assert!(cases >= 12);
}

#[test]
fn reproduces_a_known_envelope_byte_for_byte() {
    let h = InterchangeHeader::new(
        Charset::UnoC,
        SyntaxVersion::V3,
        Party::new("9900123456789").with_qualifier("500"),
        Party::new("9900987654321").with_qualifier("500"),
        PreparationDateTime::from_two_digit_year(26, 8, 2, 9, 15).unwrap(),
        "REF001",
    );
    let bytes = InterchangeBuilder::new(h.clone())
        .message("UNH+1+UTILMD:D:11A:UN:S2.1'BGM+E01'UNT+3+1'")
        .build()
        .unwrap();
    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        "UNB+UNOC:3+9900123456789:500+9900987654321:500+260802:0915+REF001'\
         UNH+1+UTILMD:D:11A:UN:S2.1'BGM+E01'UNT+3+1'UNZ+1+REF001'"
    );
    let bodyless = InterchangeBuilder::new(h).message("UNH+1+UTILMD:D:11A:UN:S2.1'UNT+2+1'");
    assert_eq!(code(bodyless.build()), "E043");
}

// ── wrapped text ───────────────────────────────────────────────────────────────

#[test]
fn text_is_encoded_into_the_repertoire() {
    let bytes = InterchangeBuilder::new(header(SyntaxVersion::V3))
        .message("UNH+1+ORDERS:D:96A:UN'FTX+AAA+Müller'UNT+3+1'")
        .build()
        .unwrap();
    assert!(bytes.windows(6).any(|w| w == b"M\xFCller"));
    let segments = from_bytes_decoded(&bytes).unwrap();
    assert_eq!(segments[2].element_str(1), Some("Müller"));
}

#[test]
fn an_unencodable_character_names_itself_and_its_offset() {
    let message = "UNH+1+ORDERS:D:96A:UN'FTX+AAA+a—b'UNT+3+1'";
    let err = InterchangeBuilder::new(header(SyntaxVersion::V3))
        .message(message)
        .build()
        .unwrap_err();
    match err {
        EdifactError::CharacterNotInRepertoire {
            character, offset, ..
        } => {
            assert_eq!(character, '—');
            assert_eq!(offset, message.find('—').unwrap());
        }
        other => panic!("{other:?}"),
    }
    // Offsets index the text as given, leading whitespace included.
    let padded = format!("\n\n{message}");
    let err = InterchangeBuilder::new(header(SyntaxVersion::V3))
        .message(padded.as_str())
        .build()
        .unwrap_err();
    assert!(
        matches!(err, EdifactError::CharacterNotInRepertoire { offset, .. } if offset == padded.find('—').unwrap()),
        "{err:?}"
    );
}

#[test]
fn surrounding_whitespace_is_accepted_and_not_written() {
    let build = |text: &str| {
        InterchangeBuilder::new(header(SyntaxVersion::V3))
            .message(text)
            .build()
    };
    let message = "UNH+1+ORDERS:D:96A:UN'BGM+1'UNT+3+1'";
    let expected = build(message).expect("builds");
    for text in [
        format!("{message}\n"),
        format!("\r\n{message}\r\n"),
        format!(" \t{message}"),
    ] {
        assert_eq!(build(&text).expect("builds"), expected, "{text:?}");
    }
}

#[test]
fn text_that_is_not_one_clean_message_is_refused() {
    let build = |messages: &[&str]| {
        let mut b = InterchangeBuilder::new(header(SyntaxVersion::V3));
        for m in messages {
            b = b.message(*m);
        }
        b.build()
    };
    let cases: &[(&[&str], &str)] = &[
        (&["UNH+1+ORDERS:D:96A:UN'BGM+1'UNZ+1+X'UNT+4+1'"], "E011"),
        (&["UNH+1+ORDERS:D:96A:UN'BGM+1'UNT+3+1'UNZ+1+X'"], "E011"),
        (&["UNA:+.? 'UNH+1+ORDERS:D:96A:UN'BGM+1'UNT+3+1'"], "E011"),
        (&["UNH+1+ORDERS:D:96A:UN'BGM+1'UNT+3+1'trailing"], "E006"),
        (&["\u{FEFF}UNH+1+ORDERS:D:96A:UN'BGM+1'UNT+3+1'"], "E011"),
        (&["\nUNA:+.? 'UNH+1+ORDERS:D:96A:UN'BGM+1'UNT+3+1'"], "E011"),
        (&["UNH+1+ORDERS:D:96A:UN'BGM+1'UNT+3+1'X"], "E001"),
        (&["UNH+1+ORDERS:D:96A:UN'BGM+1'UNT+3+1'\n\u{FEFF}"], "E006"),
        (
            &["UNH+1+ORDERS:D:96A:UN'BGM+1'UNT+3+1'UNH+2+ORDERS:D:96A:UN'BGM+1'UNT+3+2'"],
            "E011",
        ),
        (&["UNH+1+ORDERS:D:96A:UN'BGM+1'UNT+5+1'"], "E005"),
        (&["UNH+1+ORDERS:D:96A:UN'UNT+2+1'"], "E043"),
        (&["UNH+1+ORDERS:D:96A:UN'BGM+1'UNT+3+2'"], "E016"),
        (&["UNH+1+ORDERS:D:96A:UN'BGM+1'UNT+1234567+1'"], "E049"),
        (&[&text("1"), &text("1")], "E032"),
        (&["BGM+1'UNT+2+1'"], "E011"),
    ];
    for (messages, expected) in cases {
        assert_eq!(code(build(messages)), *expected, "{messages:?}");
    }
    // Written with `~` under the default characters: no terminator at all.
    let err = build(&["UNH+1+ORDERS:D:96A:UN~BGM+1~UNT+3+1~"]).unwrap_err();
    assert_ne!(err.stable_code(), "");
}

#[test]
fn owned_segments_are_checked_the_same_way() {
    let mut two = owned("1");
    two.extend(owned("2"));
    assert_eq!(
        code(
            InterchangeBuilder::new(header(SyntaxVersion::V3))
                .message_segments(two)
                .build()
        ),
        "E011"
    );
    let mut wrong_count = owned("1");
    let last = wrong_count.len() - 1;
    wrong_count[last] = from_bytes(b"UNT+9+1'")
        .next()
        .unwrap()
        .unwrap()
        .into_owned();
    assert_eq!(
        code(
            InterchangeBuilder::new(header(SyntaxVersion::V3))
                .message_segments(wrong_count)
                .build()
        ),
        "E005"
    );
}

#[test]
fn release_characters_in_owned_values_are_escaped_once() {
    let mut msg =
        MessageBuilder::new(MessageIdentifier::new("1", "ORDERS", "D", "96A", "UN")).unwrap();
    msg.write_simple("FTX", &["AAA", "it's 50?"]).unwrap();
    let bytes = InterchangeBuilder::new(header(SyntaxVersion::V3))
        .message_segments(msg.finish().unwrap())
        .build()
        .unwrap();
    assert!(bytes.windows(14).any(|w| w == b"FTX+AAA+it?'s ".as_slice()));
    let segments = from_bytes_decoded(&bytes).unwrap();
    assert_eq!(segments[2].element_str(1), Some("it's 50?"));
    validated(&bytes);
}

// ── Writer envelope ────────────────────────────────────────────────────────────

#[test]
fn raw_unb_and_unz_are_refused_inside_an_open_interchange() {
    use edifact::DataElement;
    let h = header(SyntaxVersion::V3);
    for tag in ["UNB", "UNZ"] {
        let mut w = Writer::new(Vec::new());
        w.begin_interchange(&h).unwrap();
        let before = w.segment_count();
        let calls: [Result<(), EdifactError>; 4] = [
            w.write_simple(tag, &["1", "REF001"]),
            w.write_composites(tag, &[&["1"], &["REF001"]]),
            w.write_elements(tag, &[DataElement::Simple("1")]),
            w.write_segment(&edifact::Segment::new(
                tag,
                vec![edifact::Element::of(&["1"])],
            )),
        ];
        for call in calls {
            assert_eq!(call.unwrap_err().stable_code(), "E024");
        }
        assert_eq!(w.segment_count(), before);
        assert!(!w.is_poisoned());
    }
}

#[test]
fn unz_counts_raw_groups() {
    let mut w = Writer::new(Vec::new());
    w.begin_interchange(&header(SyntaxVersion::V3)).unwrap();
    w.write_simple(
        "UNG",
        &["ORDERS", "S", "R", "260802:0915", "G1", "UN", "D:96A"],
    )
    .unwrap();
    let mut msg = w
        .begin_message(MessageIdentifier::new("1", "ORDERS", "D", "96A", "UN"))
        .unwrap();
    msg.write_simple("BGM", &["220"]).unwrap();
    msg.finish().unwrap();
    w.write_simple("UNE", &["1", "G1"]).unwrap();
    w.end_interchange().unwrap();
    assert!(w.finish().unwrap().ends_with(b"UNZ+1+REF001'"));
}
