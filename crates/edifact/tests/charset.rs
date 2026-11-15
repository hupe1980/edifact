//! Character-repertoire handling: `UNB` S001 DE 0001 decoding, encoding, and
//! validation.
//!
//! The case that motivates the whole feature: **UTF-8 is not a superset of
//! `UNOC`**. A German `ORDERS` carrying `Müller` is a conformant ISO 8859-1
//! interchange whose `ü` is the single byte `0xFC` — not valid UTF-8, and
//! therefore unparseable by a UTF-8-only reader.

use edifact::{
    Charset, EdifactError, ValidationContext, Writer, decode_interchange, from_bytes, sniff_charset,
};

/// `UNB+UNOC:3+…+NAD+BY+Müller` with `ü` as the raw Latin-1 byte `0xFC`.
fn unoc_interchange() -> Vec<u8> {
    let mut raw = b"UNB+UNOC:3+SENDER+RECEIVER+200101:0900+IC1'NAD+BY+M".to_vec();
    raw.push(0xFC);
    raw.extend_from_slice(b"ller'UNZ+0+IC1'");
    raw
}

#[test]
fn a_latin1_interchange_is_unparseable_until_it_is_decoded() {
    let raw = unoc_interchange();

    // Straight through the UTF-8 reader: the lone 0xFC is not valid UTF-8.
    let err = from_bytes(&raw)
        .collect::<Result<Vec<_>, _>>()
        .expect_err("0xFC is not valid UTF-8");
    assert!(matches!(err, EdifactError::InvalidText { .. }), "{err:?}");

    // Decoded first: the repertoire comes out of the interchange's own UNB.
    let utf8 = decode_interchange(&raw).expect("decode");
    let segments: Vec<_> = from_bytes(&utf8)
        .collect::<Result<Vec<_>, _>>()
        .expect("parse");
    assert_eq!(segments[1].element_str(1), Some("Müller"));
}

#[test]
fn sniffing_reads_the_syntax_identifier_without_decoding_the_body() {
    // The sender name carries a Latin-1 byte, so anything that parses the whole
    // UNB as UTF-8 before learning the repertoire would fail here.
    let mut raw = b"UNB+UNOC:3+M".to_vec();
    raw.push(0xDC); // Ü
    raw.extend_from_slice(b"LLER+R+200101:0900+IC1'UNZ+0+IC1'");

    assert_eq!(sniff_charset(&raw).unwrap(), Some(Charset::UnoC));
}

#[test]
fn sniffing_honours_a_custom_una() {
    let raw = b"UNA:+.? 'UNB+UNOD:3+S+R+200101:0900+IC1'UNZ+0+IC1'";
    assert_eq!(sniff_charset(raw).unwrap(), Some(Charset::UnoD));

    // component_sep `|`, element_sep `!`, decimal `,`, release `#`, rep `*`,
    // terminator `~` — the composite separators must follow the UNA, not the
    // defaults.
    let exotic = b"UNA|!,#*~UNB!UNOE|3!S!R!200101|0900!IC1~UNZ!0!IC1~";
    assert_eq!(sniff_charset(exotic).unwrap(), Some(Charset::UnoE));
}

#[test]
fn an_input_without_a_unb_is_returned_untouched() {
    let bare = b"UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+3+1'";
    assert_eq!(sniff_charset(bare).unwrap(), None);
    assert!(matches!(
        decode_interchange(bare).unwrap(),
        std::borrow::Cow::Borrowed(_)
    ));
}

#[test]
fn an_ascii_interchange_is_never_copied() {
    let ascii = b"UNB+UNOC:3+S+R+200101:0900+IC1'BGM+220'UNZ+0+IC1'";
    // UNOC, but every byte is ASCII — the zero-copy path must survive.
    assert!(
        matches!(
            decode_interchange(ascii).unwrap(),
            std::borrow::Cow::Borrowed(_)
        ),
        "an all-ASCII payload must not be reallocated"
    );
}

#[test]
fn stateful_and_multibyte_repertoires_are_refused_rather_than_mis_decoded() {
    for identifier in ["UNOX", "KECA"] {
        let err = Charset::from_syntax_identifier(identifier).unwrap_err();
        assert!(
            matches!(err, EdifactError::UnsupportedCharset { .. }),
            "{identifier}: {err:?}"
        );
        assert_eq!(err.stable_code(), "E039");
    }
    let err = Charset::from_syntax_identifier("NOPE").unwrap_err();
    assert!(matches!(err, EdifactError::UnrecognisedSyntaxIdentifier(_)));
}

#[test]
fn every_byte_of_every_repertoire_round_trips_exactly() {
    // Exhaustive rather than one representative character per repertoire: a
    // single transposed table entry is a silent mojibake bug, and it only shows
    // up on the one counterparty message that happens to use that character.
    //
    // Two directions, both required for a value to survive a decode/encode pass:
    //   byte -> char -> byte   for every byte the repertoire defines
    //   char -> byte -> char   for every character it permits
    let repertoires = [
        Charset::UnoA,
        Charset::UnoB,
        Charset::UnoC,
        Charset::UnoD,
        Charset::UnoE,
        Charset::UnoF,
        Charset::UnoG,
        Charset::UnoH,
        Charset::UnoI,
        Charset::UnoJ,
        Charset::UnoK,
    ];

    for charset in repertoires {
        let mut defined = 0usize;
        for byte in 0u8..=0xFF {
            let single = [byte];
            let Ok(decoded) = charset.decode(&single) else {
                continue; // the repertoire leaves this slot undefined
            };
            defined += 1;
            let mut chars = decoded.chars();
            let ch = chars.next().expect("one byte decodes to one character");
            assert!(
                chars.next().is_none(),
                "{charset}: 0x{byte:02X} decoded to >1 char"
            );

            let reencoded = charset
                .encode(&decoded)
                .unwrap_or_else(|e| panic!("{charset}: decoded 0x{byte:02X} to {ch:?}, which it then refused to encode: {e}"));
            assert_eq!(
                reencoded.as_ref(),
                &single[..],
                "{charset}: 0x{byte:02X} -> {ch:?} -> {reencoded:02X?}, not a round trip",
            );
        }

        // ASCII is common to every repertoire; a single-byte one adds at most 128 more.
        assert!(
            defined >= 128,
            "{charset}: only {defined} byte values decode, expected at least the ASCII range",
        );

        // The other direction, over every character the repertoire admits.
        for code in 0u32..=0xFFFF {
            let Some(ch) = char::from_u32(code) else {
                continue;
            };
            if !charset.permits(ch) {
                continue;
            }
            let text = ch.to_string();
            let encoded = charset.encode(&text).unwrap_or_else(|e| {
                panic!("{charset} permits {ch:?} but refuses to encode it: {e}")
            });
            let decoded = charset.decode(&encoded).unwrap_or_else(|e| {
                panic!("{charset}: encoded {ch:?} to bytes it cannot decode: {e}")
            });
            assert_eq!(
                decoded, text,
                "{charset}: {ch:?} did not survive encode/decode"
            );
        }
    }
}

#[test]
fn utf8_is_the_identity_transform() {
    // `UNOY` must neither transcode nor reject: it is already UTF-8.
    let text = "Grüße — Ω 漢字 🦀";
    let encoded = Charset::UnoY.encode(text).expect("encode");
    assert_eq!(encoded.as_ref(), text.as_bytes());
    assert_eq!(Charset::UnoY.decode(&encoded).expect("decode"), text);
    assert!(text.chars().all(|c| Charset::UnoY.permits(c)));
}

#[test]
fn level_a_and_level_b_repertoires_match_iso_9735() {
    // Level A: upper case, digits, space, and a fixed punctuation set.
    assert!(Charset::UnoA.permits('A'));
    assert!(Charset::UnoA.permits('7'));
    assert!(Charset::UnoA.permits(' '));
    for punctuation in ".,-()/='+:?!\"%&*;<>".chars() {
        assert!(Charset::UnoA.permits(punctuation), "{punctuation:?}");
    }
    assert!(!Charset::UnoA.permits('a'));
    assert!(!Charset::UnoA.permits('#'));
    assert!(!Charset::UnoA.permits('ü'));

    // Level B adds lower case and nothing else.
    assert!(Charset::UnoB.permits('a'));
    assert!(!Charset::UnoB.permits('ü'));
    assert!(!Charset::UnoB.permits('#'));

    // UNOY permits everything.
    assert!(Charset::UnoY.permits('€'));
    assert!(
        Charset::UnoY
            .first_violation("anything at all — 漢字")
            .is_none()
    );
}

#[test]
fn a_character_outside_the_repertoire_reports_its_offset() {
    assert_eq!(Charset::UnoA.first_violation("ORDER"), None);
    assert_eq!(Charset::UnoA.first_violation("ORDer"), Some((3, 'e')));
    // Offsets are byte offsets, so a multi-byte prefix shifts them.
    assert_eq!(Charset::UnoC.first_violation("Mü€"), Some((3, '€')));
}

#[test]
fn the_decoding_reader_matches_whole_slice_decoding() {
    let raw = unoc_interchange();
    let expected = decode_interchange(&raw).expect("decode");

    // Exercise every buffer size, including ones that split a two-byte UTF-8
    // sequence across two `read` calls.
    for capacity in 1..=16 {
        let reader = std::io::BufReader::with_capacity(
            capacity,
            Charset::UnoC.decoding_reader(std::io::Cursor::new(raw.clone())),
        );
        let mut got = Vec::new();
        std::io::Read::read_to_end(&mut { reader }, &mut got).expect("read");
        assert_eq!(got, expected.as_ref(), "capacity {capacity}");
    }
}

#[test]
fn the_decoding_reader_feeds_the_streaming_parser() {
    let raw = unoc_interchange();
    let segments = edifact::from_reader(Charset::UnoC.decoding_reader(std::io::Cursor::new(raw)))
        .collect::<Result<Vec<_>, _>>()
        .expect("parse");
    assert_eq!(segments[1].element_str(1), Some("Müller"));
}

#[test]
fn an_undefined_byte_in_a_repertoire_is_an_error_not_a_replacement_character() {
    // 0xA1 is an undefined slot in ISO 8859-6 (UNOI).
    let err = Charset::UnoI
        .decode(b"AB\xA1CD")
        .expect_err("undefined slot");
    assert!(
        matches!(err, EdifactError::InvalidText { offset: 2 }),
        "{err:?}"
    );
}

// ── writer side ───────────────────────────────────────────────────────────────

#[test]
fn a_charset_bound_writer_emits_single_byte_values() {
    let mut writer = Writer::new(Vec::new()).with_charset(Charset::UnoC);
    writer
        .write_composites("NAD", &[&["BY"], &["Müller"]])
        .expect("write");
    let wire = writer.finish().expect("finish");

    assert_eq!(wire, b"NAD+BY+M\xFCller'".to_vec());
    // And it reads back through the matching decoder.
    let decoded = Charset::UnoC.transcode_to_utf8(&wire).expect("decode");
    let segments: Vec<_> = from_bytes(&decoded)
        .collect::<Result<Vec<_>, _>>()
        .expect("parse");
    assert_eq!(segments[0].element_str(1), Some("Müller"));
}

#[test]
fn a_value_outside_the_repertoire_is_refused() {
    let mut writer = Writer::new(Vec::new()).with_charset(Charset::UnoA);
    let err = writer
        .write_composites("NAD", &[&["BY"], &["Müller"]])
        .expect_err("level A cannot carry ü");
    assert!(
        matches!(
            err,
            EdifactError::CharacterNotInRepertoire {
                character: 'ü', ..
            }
        ),
        "{err:?}"
    );
    assert_eq!(err.stable_code(), "E038");
}

#[test]
fn escaping_survives_encoding() {
    // The escaped byte is a service character (ASCII) but the surrounding text
    // is not — both halves must come out right.
    let mut writer = Writer::new(Vec::new()).with_charset(Charset::UnoC);
    writer
        .write_composites("FTX", &[&["Müller+Söhne"]])
        .expect("write");
    let wire = writer.finish().expect("finish");
    assert_eq!(wire, b"FTX+M\xFCller?+S\xF6hne'".to_vec());

    let decoded = Charset::UnoC.transcode_to_utf8(&wire).expect("decode");
    let segments: Vec<_> = from_bytes(&decoded)
        .collect::<Result<Vec<_>, _>>()
        .expect("parse");
    assert_eq!(segments[0].element_str(0), Some("Müller+Söhne"));
}

#[test]
fn a_header_that_lies_about_the_body_is_refused() {
    let mut writer = Writer::new(Vec::new()).with_charset(Charset::UnoC);
    let err = writer
        .begin_interchange(&header("UNOA", "3", "S", "R", "200101", "0900", "IC1"))
        .expect_err("UNB must not declare a repertoire the writer does not encode");
    assert!(
        matches!(err, EdifactError::CharacterRepertoireMismatch { .. }),
        "{err:?}"
    );
    assert_eq!(err.stable_code(), "E041");

    // The writer's own identifier is always accepted.
    let mut writer = Writer::new(Vec::new()).with_charset(Charset::UnoC);
    writer
        .begin_interchange(&header("UNOC", "3", "S", "R", "200101", "0900", "IC1"))
        .expect("matching identifier");
}

// ── validation layer ──────────────────────────────────────────────────────────

#[test]
fn the_validator_reports_values_outside_the_declared_repertoire() {
    // UNOA is upper-case only, but the party name is mixed case.
    let segments: Vec<_> = from_bytes(b"UNB+UNOA:3+S+R+200101:0900+IC1'NAD+BY+Acme Ltd'UNZ+0+IC1'")
        .collect::<Result<_, _>>()
        .expect("parse");

    let report = ValidationContext::builder()
        .with_charset_validation()
        .build()
        .validate(&segments);

    let issue = report
        .errors()
        .iter()
        .find(|i| i.error_code() == Some("E038"))
        .expect("expected a repertoire finding");
    assert_eq!(issue.segment_tag.as_deref(), Some("NAD"));
    assert_eq!(issue.element_index, Some(1));
    // The span points at the offending value, not the whole segment.
    let span = issue.span.expect("span");
    assert_eq!(
        &b"UNB+UNOA:3+S+R+200101:0900+IC1'NAD+BY+Acme Ltd'UNZ+0+IC1'"[span.start..span.end],
        b"Acme Ltd"
    );
}

#[test]
fn a_conformant_interchange_produces_no_repertoire_findings() {
    let segments: Vec<_> = from_bytes(b"UNB+UNOA:3+S+R+200101:0900+IC1'NAD+BY+ACME LTD'UNZ+0+IC1'")
        .collect::<Result<_, _>>()
        .expect("parse");

    let report = ValidationContext::builder()
        .with_charset_validation()
        .build()
        .validate(&segments);

    assert!(
        !report
            .errors()
            .iter()
            .any(|i| i.error_code() == Some("E038")),
        "{:#?}",
        report.errors()
    );
}

#[test]
fn a_pinned_repertoire_applies_to_message_level_slices() {
    // No UNB, so nothing declares a repertoire — the pinned one still applies.
    let segments: Vec<_> = from_bytes(b"UNH+1+ORDERS:D:96A:UN'NAD+BY+Acme'UNT+3+1'")
        .collect::<Result<_, _>>()
        .expect("parse");

    let unchecked = ValidationContext::builder()
        .with_charset_validation()
        .build()
        .validate(&segments);
    assert!(
        !unchecked
            .errors()
            .iter()
            .any(|i| i.error_code() == Some("E038"))
    );

    let pinned = ValidationContext::builder()
        .with_charset_validation_for(Charset::UnoA)
        .build()
        .validate(&segments);
    assert!(
        pinned
            .errors()
            .iter()
            .any(|i| i.error_code() == Some("E038"))
    );
}

#[test]
fn repetitions_are_checked_too() {
    // The second occurrence is the one that violates level A.
    let segments: Vec<_> =
        from_bytes(b"UNA:+.?*'UNB+UNOA:4+S+R+200101:0900+IC1'RFF+ON:AAA*ON:bbb'UNZ+0+IC1'")
            .collect::<Result<_, _>>()
            .expect("parse");

    let report = ValidationContext::builder()
        .with_charset_validation()
        .build()
        .validate(&segments);

    assert!(
        report
            .errors()
            .iter()
            .any(|i| i.error_code() == Some("E038") && i.segment_tag.as_deref() == Some("RFF")),
        "a violation in a further repetition must still be reported: {:#?}",
        report.errors()
    );
}

// ── non-finite numbers ────────────────────────────────────────────────────────

#[test]
fn non_finite_floats_are_refused() {
    use edifact::{EdifactSerialize, VecEmitter, ser::DecimalFloat};

    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut emitter = VecEmitter::default();
        let err = DecimalFloat(value)
            .edifact_serialize(&mut emitter)
            .expect_err("NaN and infinity have no EDIFACT representation");
        assert!(
            matches!(err, EdifactError::NonFiniteNumber { .. }),
            "{err:?}"
        );
        assert_eq!(err.stable_code(), "E040");
    }

    // f32 is checked on the same terms.
    let mut emitter = VecEmitter::default();
    assert!(
        DecimalFloat(f32::NAN)
            .edifact_serialize(&mut emitter)
            .is_err()
    );

    // Finite values are untouched.
    let mut emitter = VecEmitter::default();
    DecimalFloat(12.5_f64)
        .edifact_serialize(&mut emitter)
        .expect("finite values still serialize");
}

#[test]
fn a_utf8_payload_survives_the_decoding_reader() {
    // `UNOY` and the ASCII-subset repertoires have an empty high half, so mapping
    // a byte through them has no answer.  The reader must pass them through
    // rather than reject them — `decode_reader` wraps every stream, including the
    // ones that need no decoding at all.
    let raw = "UNB+UNOY:4+S+R+260101:0900+IC1'NAD+BY+Müller'UNZ+0+IC1'".as_bytes();
    for charset in [Charset::UnoY, Charset::UnoA, Charset::UnoB] {
        let segments = edifact::from_reader(charset.decoding_reader(std::io::Cursor::new(raw)))
            .collect::<Result<Vec<_>, _>>()
            .expect("parse");
        assert_eq!(segments[1].element_str(1), Some("Müller"), "{charset}");
    }
}

#[test]
fn decode_reader_discovers_the_repertoire_from_the_stream() {
    use edifact::decode_reader;

    let raw = unoc_interchange();
    let segments = edifact::from_reader(decode_reader(std::io::Cursor::new(raw)).expect("sniff"))
        .collect::<Result<Vec<_>, _>>()
        .expect("parse");
    assert_eq!(segments[1].element_str(1), Some("Müller"));

    // A UTF-8 interchange goes through untouched …
    let utf8 = "UNB+UNOY:4+S+R+260101:0900+IC1'NAD+BY+Müller'UNZ+0+IC1'".as_bytes();
    let segments = edifact::from_reader(decode_reader(std::io::Cursor::new(utf8)).expect("sniff"))
        .collect::<Result<Vec<_>, _>>()
        .expect("parse");
    assert_eq!(segments[1].element_str(1), Some("Müller"));

    // … and so does a bare message with no UNB to sniff.
    let bare = b"UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+3+1'";
    let segments =
        edifact::from_reader(decode_reader(std::io::Cursor::new(&bare[..])).expect("sniff"))
            .collect::<Result<Vec<_>, _>>()
            .expect("parse");
    assert_eq!(segments.len(), 3);
}

#[test]
fn decode_reader_handles_a_stream_that_returns_one_byte_at_a_time() {
    use edifact::decode_reader;

    /// A reader that never returns more than one byte per call, which is what
    /// the probe loop has to tolerate — a short read must not truncate the UNB.
    struct Trickle(std::io::Cursor<Vec<u8>>);
    impl std::io::Read for Trickle {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            if out.is_empty() {
                return Ok(0);
            }
            std::io::Read::read(&mut self.0, &mut out[..1])
        }
    }

    let segments = edifact::from_reader(
        decode_reader(Trickle(std::io::Cursor::new(unoc_interchange()))).expect("sniff"),
    )
    .collect::<Result<Vec<_>, _>>()
    .expect("parse");
    assert_eq!(segments[1].element_str(1), Some("Müller"));
}

// ── decoding parse entry points ───────────────────────────────────────────────

#[test]
fn from_bytes_decoded_reads_a_conformant_unoc_interchange() {
    let raw = unoc_interchange();
    // The undecoded bytes are not UTF-8 and never claimed to be.
    assert!(
        edifact::from_bytes(&raw)
            .collect::<Result<Vec<_>, _>>()
            .is_err()
    );

    let segments = edifact::from_bytes_decoded(&raw).expect("decoded parse");
    assert_eq!(segments[1].element_str(1), Some("Müller"));
}

#[test]
fn from_reader_decoded_is_lazy_and_needs_no_question_mark() {
    // The whole point: a plain `Iterator`, so a streaming pipeline does not have
    // to be boxed or made eager just to sniff the repertoire.
    let segments: Vec<_> = edifact::from_reader_decoded(std::io::Cursor::new(unoc_interchange()))
        .collect::<Result<Vec<_>, _>>()
        .expect("decoded stream");
    assert_eq!(segments[1].element_str(1), Some("Müller"));
}

#[test]
fn a_decode_failure_arrives_as_the_first_item_not_at_construction() {
    // `UNOX` is a real syntax identifier this crate cannot decode.  Constructing
    // the iterator must still succeed; the error surfaces where parse errors do.
    let raw = b"UNB+UNOX:3+S+R+260101:0900+IC1'UNZ+0+IC1'";
    let mut stream = edifact::from_reader_decoded(std::io::Cursor::new(&raw[..]));

    let first = stream.next().expect("one item");
    assert!(
        matches!(first, Err(edifact::EdifactError::UnsupportedCharset { .. })),
        "got {first:?}"
    );
    // And it terminates rather than repeating the error forever.
    assert!(stream.next().is_none());
}

#[test]
fn decoding_entry_points_pass_reader_config_through() {
    let config = edifact::ReaderConfig::default().max_segments(1);
    let raw = unoc_interchange();

    let err =
        edifact::from_bytes_decoded_with_config(&raw, config).expect_err("budget must be reported");
    assert!(matches!(err, edifact::EdifactError::LimitExceeded { .. }));

    let err = edifact::from_reader_decoded_with_config(std::io::Cursor::new(raw), config)
        .collect::<Result<Vec<_>, _>>()
        .expect_err("budget must be reported");
    assert!(matches!(err, edifact::EdifactError::LimitExceeded { .. }));
}

#[test]
fn an_ascii_interchange_decodes_to_the_same_segments() {
    let raw = b"UNB+UNOA:3+S+R+260101:0900+IC1'BGM+220'UNZ+0+IC1'";
    let plain: Vec<_> = from_bytes(raw)
        .map(|r| r.map(|s| s.into_owned()))
        .collect::<Result<Vec<_>, _>>()
        .expect("plain");
    let decoded = edifact::from_bytes_decoded(raw).expect("decoded");
    assert_eq!(plain, decoded);
}

// ── prologue: byte order mark and whitespace ahead of the header ──────────────

type Parse = Result<Vec<edifact::OwnedSegment>, EdifactError>;

/// A named decoding entry point.
type EntryPoint = (&'static str, fn(&[u8]) -> Parse);

const BOM: &[u8] = b"\xEF\xBB\xBF";
const UNA: &[u8] = b"UNA:+.? '";

/// `prefix`, an optional default `UNA`, then [`unoc_interchange`].
fn unoc_with_prologue(prefix: &[u8], una: bool) -> Vec<u8> {
    let mut raw = prefix.to_vec();
    if una {
        raw.extend_from_slice(UNA);
    }
    raw.extend_from_slice(&unoc_interchange());
    raw
}

/// A reader that returns one byte per call.
struct OneByte(std::io::Cursor<Vec<u8>>);

impl std::io::Read for OneByte {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let n = out.len().min(1);
        std::io::Read::read(&mut self.0, &mut out[..n])
    }
}

/// The six entry points that read the repertoire from the interchange and parse.
fn entry_points() -> [EntryPoint; 6] {
    [
        ("decode_interchange + from_bytes", |raw| {
            let decoded = decode_interchange(raw)?;
            from_bytes(&decoded)
                .map(|r| r.map(edifact::Segment::into_owned))
                .collect()
        }),
        ("from_bytes_decoded", edifact::from_bytes_decoded),
        ("from_bytes_decoded_with_config", |raw| {
            edifact::from_bytes_decoded_with_config(raw, edifact::ReaderConfig::default())
        }),
        ("decode_reader + from_reader", |raw| {
            let reader = edifact::decode_reader(OneByte(std::io::Cursor::new(raw.to_vec())))?;
            edifact::from_reader(reader).collect()
        }),
        ("from_reader_decoded", |raw| {
            edifact::from_reader_decoded(std::io::Cursor::new(raw.to_vec())).collect()
        }),
        ("from_reader_decoded_with_config", |raw| {
            edifact::from_reader_decoded_with_config(
                std::io::Cursor::new(raw.to_vec()),
                edifact::ReaderConfig::default(),
            )
            .collect()
        }),
    ]
}

/// Assert `result` is the sniff-window `E036` at `offset`.
#[track_caller]
fn assert_sniff_window<T: std::fmt::Debug>(
    result: Result<T, EdifactError>,
    offset: usize,
    name: &str,
) {
    assert!(
        matches!(
            result,
            Err(EdifactError::LimitExceeded { limit: "sniff_window", max: 4096, offset: o, .. }) if o == offset
        ),
        "{name}: expected the sniff window refused at {offset}, got {result:?}"
    );
}

/// The segments re-serialised: tags and values, without spans.
fn text_of(result: &Parse) -> Vec<u8> {
    let segments = result.as_ref().expect("parse");
    edifact::segments_to_bytes(segments).expect("write")
}

#[test]
fn prologue_variants_decode_like_the_bare_input() {
    let bare = unoc_interchange();
    let prologues: [(&[u8], bool); 5] = [
        (BOM, true),
        (BOM, false),
        (b"\xEF\xBB\xBF\n", false),
        (b"\r\n  ", true),
        (b"\r\n  ", false),
    ];
    for (name, parse) in entry_points() {
        let expected = text_of(&parse(&bare));
        for &(prefix, una) in &prologues {
            let raw = unoc_with_prologue(prefix, una);
            let result = parse(&raw);
            assert_eq!(text_of(&result), expected, "{name} on {prefix:?} una={una}");
            let segments = result.unwrap();
            assert_eq!(segments[1].element_str(1), Some("Müller"), "{name}");
        }
    }
    for &(prefix, una) in &prologues {
        let raw = unoc_with_prologue(prefix, una);
        assert_eq!(sniff_charset(&raw), Ok(Some(Charset::UnoC)), "{prefix:?}");
    }

    // UTF-8 and all-ASCII level A behind a byte order mark: nothing to map.
    for body in [
        "UNB+UNOY:4+S+R+260101:0900+IC1'NAD+BY+Müller'UNZ+0+IC1'".as_bytes(),
        b"UNB+UNOA:3+S+R+260101:0900+IC1'NAD+BY+MUELLER'UNZ+0+IC1'",
    ] {
        let raw = [BOM, body].concat();
        for (name, parse) in entry_points() {
            assert_eq!(text_of(&parse(&raw)), text_of(&parse(body)), "{name}");
        }
    }
}

#[test]
fn prefixed_unsupported_repertoires_are_refused() {
    for (identifier, code) in [("UNOX", "E039"), ("KECA", "E039"), ("UNOZ", "E031")] {
        for prefix in [BOM, b"\r\n  "] {
            let raw = [
                prefix,
                format!("UNB+{identifier}:3+S+R+260101:0900+IC1'UNZ+0+IC1'").as_bytes(),
            ]
            .concat();
            let err = sniff_charset(&raw).expect_err("sniff");
            assert_eq!(err.stable_code(), code, "sniff_charset {identifier}");
            for (name, parse) in entry_points() {
                let err = parse(&raw).expect_err(name);
                assert_eq!(err.stable_code(), code, "{name} {identifier}");
            }
        }
    }
}

#[test]
fn sniff_window_is_measured_after_the_prologue() {
    // A prologue of exactly the window decodes on every path.
    let raw = unoc_with_prologue(&[b'\n'; 4096], false);
    assert_eq!(sniff_charset(&raw), Ok(Some(Charset::UnoC)));
    for (name, parse) in entry_points() {
        assert_eq!(parse(&raw).expect(name)[1].element_str(1), Some("Müller"));
    }

    // One byte more and the header lies outside the window: refused, not
    // passed through undecoded.
    let raw = unoc_with_prologue(&[b'\n'; 4097], false);
    assert_sniff_window(sniff_charset(&raw), 4097, "sniff_charset");
    for (name, parse) in entry_points() {
        assert_sniff_window(parse(&raw), 4097, name);
    }
}

#[test]
fn decode_reader_reads_at_most_8_kib_before_returning() {
    /// Hands out `limit` bytes of `inner`, then panics on any further read.
    struct Fused {
        inner: std::io::Cursor<Vec<u8>>,
        left: usize,
    }
    impl std::io::Read for Fused {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            assert!(self.left > 0, "read past the sniff head");
            let n = out.len().min(self.left);
            let n = std::io::Read::read(&mut self.inner, &mut out[..n])?;
            self.left -= n;
            Ok(n)
        }
    }

    for prologue in [0, 4096, 20_000] {
        let raw = unoc_with_prologue(&vec![b' '; prologue], true);
        let reader = Fused {
            inner: std::io::Cursor::new(raw),
            left: 8192,
        };
        edifact::decode_reader(reader).expect("construction");
    }
}

#[test]
fn degenerate_prologues_pass_through() {
    let inputs: [Vec<u8>; 6] = [
        Vec::new(),
        BOM.to_vec(),
        b"\r\n\r\n".to_vec(),
        vec![b' '; 10_000],
        [BOM, &[b'\n'; 9000]].concat(),
        [UNA, &[b' '; 10_000][..]].concat(),
    ];
    for raw in &inputs {
        assert_eq!(sniff_charset(raw), Ok(None), "{} bytes", raw.len());
        for (name, parse) in entry_points() {
            assert_eq!(parse(raw), Ok(Vec::new()), "{name}, {} bytes", raw.len());
        }
    }
}

#[test]
fn an_over_long_prologue_is_refused_not_passed_through() {
    let unox = b"UNB+UNOX:3+S+R+260101:0900+IC1'UNZ+0+IC1'";
    let cases: [(Vec<u8>, usize); 4] = [
        // Prologue beyond the window, inside and beyond the 8 KiB reader head.
        ([&[b'\n'; 5000][..], unox].concat(), 5000),
        ([&[b'\n'; 10_000][..], unox].concat(), 10_000),
        // `UNA` followed by a gap that uses up the window.
        ([BOM, UNA, &[b' '; 4096][..], unox].concat(), 4108),
        ([BOM, UNA, &[b' '; 9000][..], unox].concat(), 9012),
    ];
    for (raw, offset) in &cases {
        assert_sniff_window(sniff_charset(raw), *offset, "sniff_charset");
        for (name, parse) in entry_points() {
            assert_sniff_window(parse(raw), *offset, name);
        }
    }

    // A header cut by the window's end is refused the same way.
    let raw = [UNA, &[b' '; 4085][..], unox].concat();
    for (name, parse) in entry_points() {
        let err = parse(&raw).expect_err(name);
        assert_eq!(err.stable_code(), "E036", "{name}");
    }
}

#[test]
fn a_prefixed_ascii_interchange_borrows_and_keeps_spans() {
    let raw = [BOM, b"UNB+UNOC:3+S+R+200101:0900+IC1'BGM+220'UNZ+0+IC1'"].concat();
    let decoded = decode_interchange(&raw).expect("decode");
    assert!(matches!(decoded, std::borrow::Cow::Borrowed(_)));
    let direct: Vec<_> = from_bytes(&raw).collect::<Result<_, _>>().expect("raw");
    let via: Vec<_> = from_bytes(&decoded)
        .collect::<Result<_, _>>()
        .expect("decoded");
    assert_eq!(direct, via);
}

#[test]
fn a_decode_error_offset_counts_the_prologue() {
    let mut raw = [BOM, b"UNB+UNOA:3+S+R+200101:0900+IC1'NAD+BY+M"].concat();
    let at = raw.len();
    raw.push(0xFC);
    raw.extend_from_slice(b"ller'UNZ+0+IC1'");
    assert_eq!(
        decode_interchange(&raw),
        Err(EdifactError::InvalidText { offset: at })
    );
}

#[test]
fn decode_reader_honours_the_prologue_one_byte_at_a_time() {
    use std::io::Read;

    let raw = unoc_with_prologue(BOM, true);
    let mut decoded = Vec::new();
    edifact::decode_reader(OneByte(std::io::Cursor::new(raw)))
        .expect("sniff")
        .read_to_end(&mut decoded)
        .expect("read");
    let bare = decode_interchange(&unoc_interchange())
        .unwrap()
        .into_owned();
    assert_eq!(decoded, [BOM, UNA, &bare].concat());
}

/// A `UNB` header from its strings, as `InterchangeHeader::parse` reads them.
fn header(
    syntax: &str,
    version: &str,
    sender: &str,
    recipient: &str,
    date: &str,
    time: &str,
    reference: &str,
) -> edifact::InterchangeHeader {
    let raw = format!("UNB+{syntax}:{version}+{sender}+{recipient}+{date}:{time}+{reference}'");
    let segments: Vec<_> = edifact::from_bytes(raw.as_bytes())
        .collect::<Result<_, _>>()
        .expect("header parses");
    edifact::InterchangeHeader::parse(&segments[0]).expect("header is valid")
}

#[test]
fn the_decoding_reader_reports_an_undefined_byte_as_e003() {
    use std::io::Read;

    // Latin-3 letters decode to two UTF-8 bytes each; 0xA5 is undefined in UNOG.
    let mut raw = Charset::UnoG.encode("ĦĤß").unwrap().into_owned();
    raw.push(0xA5);
    let expected = EdifactError::InvalidText { offset: 3 };

    for size in [1, 2, 3] {
        let mut reader = Charset::UnoG.decoding_reader(std::io::Cursor::new(raw.clone()));
        let mut buf = vec![0u8; size];
        let error = loop {
            match reader.read(&mut buf) {
                Ok(0) => panic!("buffer {size}: ended without an error"),
                Ok(_) => {}
                Err(error) => break error,
            }
        };
        assert_eq!(EdifactError::from(error), expected, "buffer {size}");
        let again = reader.read(&mut buf).expect_err("the error repeats");
        assert_eq!(EdifactError::from(again), expected, "buffer {size}");
    }

    let mut out = Vec::new();
    let error = Charset::UnoG
        .decoding_reader(std::io::Cursor::new(raw))
        .read_to_end(&mut out)
        .expect_err("undefined byte");
    assert_eq!(EdifactError::from(error), expected);
    assert_eq!(out, "ĦĤß".as_bytes());
}
