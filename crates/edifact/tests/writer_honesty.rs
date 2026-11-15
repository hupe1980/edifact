//! The writer stack never turns an error path into a well-formed artefact.
//!
//! A failed write, an abandoned message, or an emitter finished with a segment
//! open must leave output that is visibly incomplete, and the producer must
//! learn about it at its next call. Successful sessions keep their exact bytes,
//! pinned by the golden matrix below.

#![cfg(feature = "derive")]

use edifact::{
    Charset, DataElement, EdifactError, EdifactEvent, EdifactSerialize, Element, EventEmitter,
    Segment, ServiceStringAdvice, Writer, WriterEmitter, elements, from_bytes, validate_envelope,
};
use std::io::{self, Write};

// ── helpers ───────────────────────────────────────────────────────────────────

/// A sink that fails its `fail_at`-th `write` call (zero-based), and every
/// `flush` when `fail_flush` is set.
#[derive(Debug, Default)]
struct FailingSink {
    buf: Vec<u8>,
    writes: usize,
    fail_at: Option<usize>,
    fail_flush: bool,
}

impl FailingSink {
    fn failing_at(k: usize) -> Self {
        Self {
            fail_at: Some(k),
            ..Self::default()
        }
    }
}

impl Write for FailingSink {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let index = self.writes;
        self.writes += 1;
        if self.fail_at == Some(index) {
            return Err(io::Error::other("injected sink failure"));
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.fail_flush {
            return Err(io::Error::other("injected flush failure"));
        }
        Ok(())
    }
}

/// Whether `bytes` parse and pass envelope validation.
fn envelope_ok(bytes: &[u8]) -> bool {
    match from_bytes(bytes).collect::<Result<Vec<_>, _>>() {
        Ok(segments) => validate_envelope(&segments).is_ok(),
        Err(_) => false,
    }
}

fn unt_count(bytes: &[u8]) -> usize {
    bytes.windows(4).filter(|w| *w == b"UNT+").count()
}

fn exotic_ssa() -> ServiceStringAdvice {
    ServiceStringAdvice {
        component_sep: b'|',
        element_sep: b'!',
        decimal_mark: b',',
        release_char: b'#',
        repetition_sep: b'*',
        segment_term: b'~',
    }
}

/// The two-message interchange every failure sweep replays, written by a
/// "log and continue" caller: every error is recorded and the script goes on.
/// Returns the result of every call in order.
fn script_two_messages<W: Write>(w: &mut Writer<W>) -> Vec<Result<(), EdifactError>> {
    let mut results = Vec::new();
    results.push(w.begin_interchange(&header(
        "UNOA", "1", "SENDER", "RECEIVER", "200101", "0900", "IC1",
    )));
    for message_ref in ["1", "2"] {
        match w.begin_message(edifact::MessageIdentifier::new(
            message_ref,
            "ORDERS",
            "D",
            "96A",
            "UN",
        )) {
            Ok(mut msg) => {
                results.push(msg.write_simple("BGM", &["220", "PO", "9"]));
                results.push(msg.write_composites("DTM", &[&["137", "20260101", "102"]]));
                results.push(msg.finish());
            }
            Err(e) => results.push(Err(e)),
        }
    }
    results.push(w.end_interchange());
    results
}

// ── golden matrix ──────────────────────

#[derive(EdifactSerialize)]
#[edifact(segment = "BGM")]
struct Bgm {
    #[edifact(element = 0)]
    code: String,
    #[edifact(element = 1)]
    number: String,
    #[edifact(element = 2)]
    function: Option<String>,
}

fn session_default_delimiters() -> Vec<u8> {
    let mut w = Writer::new(Vec::new());
    w.write_segment(&Segment::new(
        "NAD",
        vec![Element::of(&["BY"]), Element::of(&["ACME+1", "", "9"])],
    ))
    .unwrap();
    w.write_simple("BGM", &["220", "PO:1'?", "9"]).unwrap();
    w.write_composites("DTM", &[&["137", "20260101", "102"]])
        .unwrap();
    w.write_elements("NAD", elements!["MS", ["9900112233445", "", "293"]])
        .unwrap();
    w.write_elements("UNS", &[] as &[DataElement<'_>]).unwrap();
    w.finish().unwrap()
}

fn session_with_una_exotic() -> Vec<u8> {
    let mut w = Writer::with_una(Vec::new(), exotic_ssa()).unwrap();
    w.write_segment(&Segment::new(
        "RFF",
        vec![Element::of(&["ON", "1"]).and_repeat(&["ON", "2*3"])],
    ))
    .unwrap();
    w.write_simple("FTX", &["AAA", "a!b|c#d~e"]).unwrap();
    w.write_composites("DTM", &[&["137", "20260101"]]).unwrap();
    w.write_elements("NAD", elements!["MS", ["x", "y"]])
        .unwrap();
    w.finish().unwrap()
}

fn session_service_string_advice() -> Vec<u8> {
    let ssa = ServiceStringAdvice::for_syntax_version(Some(4));
    let mut w = Writer::with_service_string_advice(Vec::new(), ssa).unwrap();
    w.write_segment(&Segment::new(
        "RFF",
        vec![Element::of(&["ON", "1"]).and_repeat(&["ON", "2"])],
    ))
    .unwrap();
    w.write_simple("FTX", &["a*b"]).unwrap();
    w.finish().unwrap()
}

fn session_charset() -> Vec<u8> {
    let mut w = Writer::new(Vec::new()).with_charset(Charset::UnoC);
    w.begin_interchange(&header("UNOC", "3", "S", "R", "200101", "0900", "IC1"))
        .unwrap();
    w.write_composites("NAD", &[&["BY"][..], &["Müller+Söhne"][..]])
        .unwrap();
    w.finish().unwrap()
}

fn session_envelope() -> Vec<u8> {
    let mut w = Writer::new(Vec::new());
    w.begin_interchange(&header(
        "UNOA", "1", "SENDER", "RECEIVER", "200101", "0900", "IC1",
    ))
    .unwrap();
    {
        let mut msg = w
            .begin_message(edifact::MessageIdentifier::new(
                "1", "ORDERS", "D", "96A", "UN",
            ))
            .unwrap();
        msg.write_simple("BGM", &["220", "PO", "9"]).unwrap();
        msg.write_composites("DTM", &[&["137", "20260101", "102"]])
            .unwrap();
        msg.write_elements("NAD", elements!["BY", ["1", "", "9"]])
            .unwrap();
        msg.write_segment(&Segment::new("UNS", vec![Element::of(&["S"])]))
            .unwrap();
        msg.finish().unwrap();
    }
    {
        let mut msg = w
            .begin_message(edifact::MessageIdentifier::new(
                "2", "ORDERS", "D", "96A", "UN",
            ))
            .unwrap();
        msg.write_simple("BGM", &["220", "PO2"]).unwrap();
        msg.finish().unwrap();
    }
    w.end_interchange().unwrap();
    w.finish().unwrap()
}

fn session_envelope_exotic() -> Vec<u8> {
    let mut w = Writer::with_una(Vec::new(), exotic_ssa()).unwrap();
    // An active repetition separator needs syntax version 4.
    w.begin_interchange(&header("UNOA", "4", "S", "R", "20200101", "0900", "IC1"))
        .unwrap();
    let mut msg = w
        .begin_message(edifact::MessageIdentifier::new(
            "1", "ORDERS", "D", "96A", "UN",
        ))
        .unwrap();
    msg.write_simple("BGM", &["220"]).unwrap();
    msg.finish().unwrap();
    w.end_interchange().unwrap();
    w.finish().unwrap()
}

fn session_finish_unt_after_unh() -> Vec<u8> {
    let mut w = Writer::new(Vec::new());
    w.begin_interchange(&header("UNOA", "1", "S", "R", "200101", "0900", "IC1"))
        .unwrap();
    w.write_composites("UNH", &[&["1"][..], &["ORDERS", "D", "96A", "UN"][..]])
        .unwrap();
    w.write_simple("BGM", &["220"]).unwrap();
    w.finish_unt("1").unwrap()
}

fn session_finish_unt_without_unh() -> Vec<u8> {
    let mut w = Writer::new(Vec::new());
    w.write_simple("BGM", &["220"]).unwrap();
    w.write_simple("DTM", &["137"]).unwrap();
    w.finish_unt("7").unwrap()
}

fn session_emitter_default() -> Vec<u8> {
    let mut e = WriterEmitter::new(Vec::new());
    for event in [
        EdifactEvent::start("NAD"),
        EdifactEvent::element("MS"),
        EdifactEvent::element("99+00"),
        EdifactEvent::component(""),
        EdifactEvent::component("293"),
        EdifactEvent::EndSegment,
        EdifactEvent::start("UNH"),
        EdifactEvent::element("1"),
        EdifactEvent::EndSegment,
    ] {
        e.emit(event).unwrap();
    }
    e.finish().unwrap()
}

fn session_emitter_with_una() -> Vec<u8> {
    let ssa = ServiceStringAdvice::from_bytes(b"UNA:+.?*'").unwrap();
    let mut e = WriterEmitter::with_una(Vec::new(), ssa).unwrap();
    for event in [
        EdifactEvent::start("RFF"),
        EdifactEvent::element("ON"),
        EdifactEvent::component("1"),
        EdifactEvent::repeat("ON"),
        EdifactEvent::component("2*"),
        EdifactEvent::EndSegment,
    ] {
        e.emit(event).unwrap();
    }
    e.finish().unwrap()
}

fn session_emitter_charset() -> Vec<u8> {
    let mut e = WriterEmitter::new(Vec::new()).with_charset(Charset::UnoC);
    e.emit(EdifactEvent::start("NAD")).unwrap();
    e.emit(EdifactEvent::element("Müller")).unwrap();
    e.emit(EdifactEvent::EndSegment).unwrap();
    e.finish().unwrap()
}

fn session_lib_to_writer() -> Vec<u8> {
    let segments = vec![
        Segment::new("BGM", vec![Element::of(&["220"]), Element::of(&["P'O"])]),
        Segment::new("DTM", vec![Element::of(&["137", "20260101", "102"])]),
    ];
    let mut out = Vec::new();
    edifact::to_writer(&mut out, &segments).unwrap();
    out
}

fn session_segments_to_bytes() -> Vec<u8> {
    let segments = vec![Segment::new(
        "FTX",
        vec![Element::of(&["AAA"]), Element::of(&["x:y", "z"])],
    )];
    edifact::segments_to_bytes(&segments).unwrap()
}

fn session_ser_to_bytes() -> Vec<u8> {
    let bgm = Bgm {
        code: "220".into(),
        number: "PO+1".into(),
        function: Some("9".into()),
    };
    let mut out = edifact::to_bytes(&bgm).unwrap();
    let partial = Bgm {
        code: "220".into(),
        number: "PO".into(),
        function: None,
    };
    edifact::ser::to_writer(&mut out, &partial).unwrap();
    out
}

type Session = fn() -> Vec<u8>;

const SESSIONS: &[(&str, Session)] = &[
    ("default_delimiters", session_default_delimiters),
    ("with_una_exotic", session_with_una_exotic),
    ("service_string_advice", session_service_string_advice),
    ("charset", session_charset),
    ("envelope", session_envelope),
    ("envelope_exotic", session_envelope_exotic),
    ("finish_unt_after_unh", session_finish_unt_after_unh),
    ("finish_unt_without_unh", session_finish_unt_without_unh),
    ("emitter_default", session_emitter_default),
    ("emitter_with_una", session_emitter_with_una),
    ("emitter_charset", session_emitter_charset),
    ("lib_to_writer", session_lib_to_writer),
    ("segments_to_bytes", session_segments_to_bytes),
    ("ser_to_bytes", session_ser_to_bytes),
];

const GOLDEN: &[(&str, &[u8])] = &[
    ("default_delimiters", b"NAD+BY+ACME?+1::9'BGM+220+PO?:1?'??+9'DTM+137:20260101:102'NAD+MS+9900112233445::293'UNS'"),
    ("with_una_exotic", b"UNA|!,#*~RFF!ON|1*ON|2#*3~FTX!AAA!a#!b#|c##d#~e~DTM!137|20260101~NAD!MS!x|y~"),
    ("service_string_advice", b"RFF+ON:1*ON:2'FTX+a?*b'"),
    ("charset", b"UNB+UNOC:3+S+R+200101:0900+IC1'NAD+BY+M\xfcller?+S\xf6hne'"),
    ("envelope", b"UNB+UNOA:1+SENDER+RECEIVER+200101:0900+IC1'UNH+1+ORDERS:D:96A:UN'BGM+220+PO+9'DTM+137:20260101:102'NAD+BY+1::9'UNS+S'UNT+6+1'UNH+2+ORDERS:D:96A:UN'BGM+220+PO2'UNT+3+2'UNZ+2+IC1'"),
    ("envelope_exotic", b"UNA|!,#*~UNB!UNOA|4!S!R!20200101|0900!IC1~UNH!1!ORDERS|D|96A|UN~BGM!220~UNT!3!1~UNZ!1!IC1~"),
    ("finish_unt_after_unh", b"UNB+UNOA:1+S+R+200101:0900+IC1'UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+3+1'"),
    ("finish_unt_without_unh", b"BGM+220'DTM+137'UNT+3+7'"),
    ("emitter_default", b"NAD+MS+99?+00::293'UNH+1'"),
    ("emitter_with_una", b"UNA:+.?*'RFF+ON:1*ON:2?*'"),
    ("emitter_charset", b"NAD+M\xfcller'"),
    ("lib_to_writer", b"BGM+220+P?'O'DTM+137:20260101:102'"),
    ("segments_to_bytes", b"FTX+AAA+x?:y:z'"),
    ("ser_to_bytes", b"BGM+220+PO?+1+9'BGM+220+PO'"),
];

#[test]
fn golden_matrix_is_byte_identical() {
    assert_eq!(SESSIONS.len(), GOLDEN.len());
    for ((name, session), (golden_name, expected)) in SESSIONS.iter().zip(GOLDEN) {
        assert_eq!(name, golden_name);
        let actual = session();
        assert_eq!(
            actual.escape_ascii().to_string(),
            expected.escape_ascii().to_string(),
            "session {name}"
        );
    }
}

// ── aborted messages never look complete ──────────────────────────────────────

fn is_poisoned_err<T>(result: &Result<T, EdifactError>) -> bool {
    matches!(result, Err(EdifactError::WriterPoisoned { .. }))
}

#[derive(Debug)]
struct MappingError;

/// Writes message `message_ref` and fails with the caller's own error after
/// `BGM`, the way an early `?` in mapping code does.
fn write_order<W: Write>(
    w: &mut Writer<W>,
    message_ref: &str,
    fail: bool,
) -> Result<(), MappingError> {
    let mut msg = w
        .begin_message(edifact::MessageIdentifier::new(
            message_ref,
            "ORDERS",
            "D",
            "96A",
            "UN",
        ))
        .map_err(|_| MappingError)?;
    msg.write_simple("BGM", &["220", "PO", "9"])
        .map_err(|_| MappingError)?;
    if fail {
        return Err(MappingError);
    }
    msg.write_composites("DTM", &[&["137", "20260101", "102"]])
        .map_err(|_| MappingError)?;
    msg.finish().map_err(|_| MappingError)
}

#[test]
fn abort_on_caller_error() {
    let mut w = Writer::new(Vec::new());
    w.begin_interchange(&header(
        "UNOA", "1", "SENDER", "RECEIVER", "200101", "0900", "IC1",
    ))
    .unwrap();
    write_order(&mut w, "1", false).unwrap();
    assert!(write_order(&mut w, "2", true).is_err());
    // Log and carry on: the interchange cannot be closed.
    let closed = w.end_interchange();
    assert!(is_poisoned_err(&closed), "{closed:?}");
    assert!(w.is_poisoned());

    let bytes = w.into_inner();
    assert!(
        bytes.ends_with(b"UNH+2+ORDERS:D:96A:UN'BGM+220+PO+9'"),
        "{}",
        bytes.escape_ascii()
    );
    assert_eq!(unt_count(&bytes), 1, "only message 1 is closed");
    assert!(!envelope_ok(&bytes));
}

/// Index of the next sink write after running `prefix` on a fresh writer.
fn writes_after(prefix: impl FnOnce(&mut Writer<FailingSink>)) -> usize {
    let mut w = Writer::new(FailingSink::default());
    prefix(&mut w);
    w.into_inner().writes
}

#[test]
fn abort_on_failing_sink() {
    // The DTM tag write is the first write after BGM; count it without UNT.
    let dtm_tag = writes_after(|w| {
        w.begin_interchange(&header("UNOA", "1", "S", "R", "200101", "0900", "IC1"))
            .unwrap();
        let mut msg = w
            .begin_message(edifact::MessageIdentifier::new(
                "1", "ORDERS", "D", "96A", "UN",
            ))
            .unwrap();
        msg.write_simple("BGM", &["220", "PO", "9"]).unwrap();
        std::mem::forget(msg);
    });

    let mut w = Writer::new(FailingSink::failing_at(dtm_tag));
    w.begin_interchange(&header("UNOA", "1", "S", "R", "200101", "0900", "IC1"))
        .unwrap();
    {
        let mut msg = w
            .begin_message(edifact::MessageIdentifier::new(
                "1", "ORDERS", "D", "96A", "UN",
            ))
            .unwrap();
        msg.write_simple("BGM", &["220", "PO", "9"]).unwrap();
        let dtm = msg.write_composites("DTM", &[&["137", "20260101", "102"]]);
        assert!(matches!(dtm, Err(EdifactError::Io(_))), "{dtm:?}");
        // The guard is dropped here, as an early `?` would.
    }
    let closed = w.end_interchange();
    assert!(is_poisoned_err(&closed), "{closed:?}");

    let bytes = w.into_inner().buf;
    assert!(
        bytes.ends_with(b"BGM+220+PO+9'"),
        "{}",
        bytes.escape_ascii()
    );
    assert_eq!(unt_count(&bytes), 0);
    assert!(!envelope_ok(&bytes));
}

#[test]
fn abort_on_charset_error_mid_value() {
    let mut w = Writer::new(Vec::new()).with_charset(Charset::UnoC);
    w.begin_interchange(&header("UNOC", "3", "S", "R", "200101", "0900", "IC1"))
        .unwrap();
    {
        let mut msg = w
            .begin_message(edifact::MessageIdentifier::new(
                "1", "ORDERS", "D", "96A", "UN",
            ))
            .unwrap();
        let nad = msg.write_composites("NAD", &[&["BY"][..], &["M€ller"][..]]);
        assert!(
            matches!(nad, Err(EdifactError::CharacterNotInRepertoire { .. })),
            "{nad:?}"
        );
    }
    assert!(w.is_poisoned());
    let bytes = w.into_inner();
    assert!(bytes.ends_with(b"NAD+BY+"), "{}", bytes.escape_ascii());
    assert_eq!(unt_count(&bytes), 0);
    assert!(!envelope_ok(&bytes));
}

#[test]
fn abort_on_panic() {
    let mut w = Writer::new(Vec::new());
    w.begin_interchange(&header("UNOA", "1", "S", "R", "200101", "0900", "IC1"))
        .unwrap();
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut msg = w
            .begin_message(edifact::MessageIdentifier::new(
                "1", "ORDERS", "D", "96A", "UN",
            ))
            .unwrap();
        msg.write_simple("BGM", &["220"]).unwrap();
        panic!("mapping bug");
    }));
    assert!(caught.is_err());
    assert!(w.is_poisoned());
    let bytes = w.into_inner();
    assert_eq!(unt_count(&bytes), 0, "{}", bytes.escape_ascii());
    assert!(!envelope_ok(&bytes));
}

#[test]
fn a_failed_finish_is_not_retried_on_drop() {
    let unt = writes_after(|w| {
        let msg = w
            .begin_message(edifact::MessageIdentifier::new(
                "1", "ORDERS", "D", "96A", "UN",
            ))
            .unwrap();
        std::mem::forget(msg);
    });
    let mut w = Writer::new(FailingSink::failing_at(unt));
    let msg = w
        .begin_message(edifact::MessageIdentifier::new(
            "1", "ORDERS", "D", "96A", "UN",
        ))
        .unwrap();
    assert!(matches!(msg.finish(), Err(EdifactError::Io(_))));
    assert!(w.is_poisoned());
    let sink = w.into_inner();
    // Exactly one (failed) attempt at UNT, and no retry on drop.
    assert_eq!(sink.writes, unt + 1);
    assert_eq!(unt_count(&sink.buf), 0);
}

// ── failures poison, refusals do not ──────────────────────────────────────────

/// A writer whose sink failed on `BGM`'s second write: `BGM` reached the sink,
/// the element separator did not.
fn poisoned_writer() -> Writer<FailingSink> {
    let mut w = Writer::new(FailingSink::failing_at(1));
    let failed = w.write_simple("BGM", &["220"]);
    assert!(matches!(failed, Err(EdifactError::Io(_))), "{failed:?}");
    assert!(w.is_poisoned());
    w
}

#[test]
fn poison_after_sink_failure_blocks_every_method() {
    let mut w = poisoned_writer();
    assert!(is_poisoned_err(
        &w.write_segment(&Segment::new("DTM", vec![Element::of(&["1"])]))
    ));
    assert!(is_poisoned_err(&w.write_simple("DTM", &["1"])));
    assert!(is_poisoned_err(&w.write_composites("DTM", &[&["1"]])));
    assert!(is_poisoned_err(&w.write_elements("DTM", elements!["1"])));
    assert!(is_poisoned_err(&w.begin_interchange(&header(
        "UNOA", "1", "S", "R", "200101", "0900", "IC1"
    ))));
    assert!(is_poisoned_err(&w.begin_message(
        edifact::MessageIdentifier::new("1", "ORDERS", "D", "96A", "UN")
    )));
    assert!(is_poisoned_err(&w.end_interchange()));
    // A poisoned writer refuses even calls that would otherwise be refusals.
    assert!(is_poisoned_err(&w.write_simple("bad", &["1"])));
    assert_eq!(w.into_inner().buf, b"BGM");

    assert!(is_poisoned_err(&poisoned_writer().finish_unt("1")));
    assert!(is_poisoned_err(&poisoned_writer().finish()));
}

#[test]
fn poison_blocks_a_message_writer_obtained_before_the_failure() {
    // The first write after UNH is BGM's tag; the next one its separator.
    let bgm_sep = writes_after(|w| {
        let msg = w
            .begin_message(edifact::MessageIdentifier::new(
                "1", "ORDERS", "D", "96A", "UN",
            ))
            .unwrap();
        std::mem::forget(msg);
    }) + 1;
    let mut w = Writer::new(FailingSink::failing_at(bgm_sep));
    {
        let mut msg = w
            .begin_message(edifact::MessageIdentifier::new(
                "1", "ORDERS", "D", "96A", "UN",
            ))
            .unwrap();
        assert!(matches!(
            msg.write_simple("BGM", &["220"]),
            Err(EdifactError::Io(_))
        ));
        assert!(is_poisoned_err(&msg.write_simple("DTM", &["1"])));
        assert!(is_poisoned_err(&msg.write_composites("DTM", &[&["1"]])));
        assert!(is_poisoned_err(&msg.write_elements("DTM", elements!["1"])));
        assert!(is_poisoned_err(
            &msg.write_segment(&Segment::new("DTM", vec![Element::of(&["1"])]))
        ));
        assert!(is_poisoned_err(&msg.finish()));
    }
    let bytes = w.into_inner().buf;
    assert_eq!(bytes, b"UNH+1+ORDERS:D:96A:UN'BGM");
}

#[test]
fn into_inner_returns_partial_bytes() {
    assert_eq!(poisoned_writer().into_inner().buf, b"BGM");
    // Healthy writers hand their sink back unflushed as well.
    let mut w = Writer::new(FailingSink {
        fail_flush: true,
        ..FailingSink::default()
    });
    w.write_simple("BGM", &["220"]).unwrap();
    assert!(!w.is_poisoned());
    assert_eq!(w.into_inner().buf, b"BGM+220'");
}

#[test]
fn a_flush_failure_is_reported_by_finish() {
    let mut w = Writer::new(FailingSink {
        fail_flush: true,
        ..FailingSink::default()
    });
    w.write_simple("BGM", &["220"]).unwrap();
    assert!(matches!(w.finish(), Err(EdifactError::Io(_))));
}

#[test]
fn refusal_does_not_poison() {
    let refused_session = || {
        let mut w = Writer::new(Vec::new()).with_charset(Charset::UnoC);
        assert!(matches!(
            w.write_simple("bgm", &["220"]),
            Err(EdifactError::InvalidSegmentTag(_))
        ));
        assert!(matches!(
            w.write_segment(&Segment::new(
                "RFF",
                vec![Element::of(&["ON", "1"]).and_repeat(&["ON", "2"])]
            )),
            Err(EdifactError::RepetitionSeparatorNotDeclared)
        ));
        assert!(matches!(
            w.begin_interchange(&header("UNOA", "1", "S", "R", "200101", "0900", "IC1")),
            Err(EdifactError::CharacterRepertoireMismatch { .. })
        ));
        assert!(!w.is_poisoned());
        w.begin_interchange(&header("UNOC", "3", "S", "R", "200101", "0900", "IC1"))
            .unwrap();
        let mut msg = w
            .begin_message(edifact::MessageIdentifier::new(
                "1", "ORDERS", "D", "96A", "UN",
            ))
            .unwrap();
        assert!(matches!(
            msg.write_elements("bad", elements!["1"]),
            Err(EdifactError::InvalidSegmentTag(_))
        ));
        msg.write_simple("BGM", &["220"]).unwrap();
        msg.finish().unwrap();
        w.end_interchange().unwrap();
        w.finish().unwrap()
    };
    let clean_session = || {
        let mut w = Writer::new(Vec::new()).with_charset(Charset::UnoC);
        w.begin_interchange(&header("UNOC", "3", "S", "R", "200101", "0900", "IC1"))
            .unwrap();
        let mut msg = w
            .begin_message(edifact::MessageIdentifier::new(
                "1", "ORDERS", "D", "96A", "UN",
            ))
            .unwrap();
        msg.write_simple("BGM", &["220"]).unwrap();
        msg.finish().unwrap();
        w.end_interchange().unwrap();
        w.finish().unwrap()
    };
    let bytes = refused_session();
    assert_eq!(bytes, clean_session());
    assert!(envelope_ok(&bytes));
}

#[test]
fn every_failure_index_is_loud() {
    let total = {
        let mut w = Writer::new(FailingSink::default());
        let results = script_two_messages(&mut w);
        assert!(results.iter().all(Result::is_ok));
        let sink = w.into_inner();
        assert!(envelope_ok(&sink.buf));
        sink.writes
    };
    assert!(
        total > 20,
        "the script must exercise many writes, got {total}"
    );

    for k in 0..total {
        let mut w = Writer::new(FailingSink::failing_at(k));
        let results = script_two_messages(&mut w);
        let first = results
            .iter()
            .position(Result::is_err)
            .unwrap_or_else(|| panic!("k={k}: no call reported the failure"));
        assert!(
            matches!(results[first], Err(EdifactError::Io(_))),
            "k={k}: {:?}",
            results[first]
        );
        for later in &results[first + 1..] {
            assert!(is_poisoned_err(later), "k={k}: {later:?}");
        }
        assert!(results.last().unwrap().is_err(), "k={k}: UNZ written");
        assert!(w.is_poisoned());
        let bytes = w.into_inner().buf;
        assert!(!envelope_ok(&bytes), "k={k}: {}", bytes.escape_ascii());
    }
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
