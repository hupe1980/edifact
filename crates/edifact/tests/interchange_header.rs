//! `InterchangeHeader`: parse, read, bounded read_from, envelope parity.

use edifact::{
    Charset, EdifactError, InterchangeHeader, Party, PreparationDateTime, ReaderConfig,
    SyntaxVersion, Year, from_bytes, from_reader, validate_envelope,
};
use std::io::Read;

fn parse(raw: &str) -> Result<InterchangeHeader, EdifactError> {
    let segments: Vec<_> = from_bytes(raw.as_bytes())
        .collect::<Result<_, _>>()
        .expect("parses");
    InterchangeHeader::parse(&segments[0])
}

fn code(raw: &str) -> &'static str {
    parse(raw).expect_err("refused").stable_code()
}

const FULL: &str = "UNB+UNOC:4:40101:7:01+S:14:RA:SUB+R:500+20261001:0700+1+PW:AA+APP+A+1+AGR+1'";

#[test]
fn every_position_is_read() {
    let h = parse(FULL).unwrap();
    assert_eq!(h.charset, Charset::UnoC);
    assert_eq!(h.syntax_version, SyntaxVersion::V4);
    assert_eq!(h.service_code_list_version.as_deref(), Some("40101"));
    assert_eq!(h.character_encoding.as_deref(), Some("7"));
    assert_eq!(h.syntax_release.as_deref(), Some("01"));
    assert_eq!(
        h.sender,
        Party::new("S")
            .with_qualifier("14")
            .with_internal_id("RA")
            .with_internal_sub_id("SUB")
    );
    assert_eq!(h.recipient, Party::new("R").with_qualifier("500"));
    assert_eq!(
        h.prepared,
        PreparationDateTime::new(2026, 10, 1, 7, 0).unwrap()
    );
    assert_eq!(h.control_ref, "1");
    assert_eq!(h.recipient_password.as_deref(), Some("PW"));
    assert_eq!(h.recipient_password_qualifier.as_deref(), Some("AA"));
    assert_eq!(h.application_ref.as_deref(), Some("APP"));
    assert_eq!(h.processing_priority.as_deref(), Some("A"));
    assert!(h.ack_requested());
    assert_eq!(h.communications_agreement_id.as_deref(), Some("AGR"));
    assert!(h.is_test());
}

#[test]
fn parse_then_to_segment_round_trips() {
    for raw in [
        FULL,
        "UNB+UNOA:3+S+R+260101:0900+IC1'",
        "UNB+UNOC:3+S::RA+R+260101:0900+IC1++APP'",
    ] {
        let h = parse(raw).unwrap();
        let bytes = edifact::segments_to_bytes([&h.to_segment().unwrap()]).unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap(), raw);
    }
}

#[test]
fn nothing_is_defaulted() {
    // No S001.
    assert_eq!(code("UNB++S+R+260101:0900+1'"), "E008");
    // No version.
    assert_eq!(code("UNB+UNOC+S+R+260101:0900+1'"), "E021");
}

#[test]
fn s004_and_version_are_checked() {
    assert_eq!(code("UNB+UNOC:3+S+R+261399:2599+1'"), "E060");
    assert_eq!(code("UNB+UNOC:3+S+R+260101:090000+1'"), "E049");
    assert_eq!(code("UNB+UNOC:X+S+R+260101:0900+1'"), "E059");
    assert_eq!(code("UNB+UNOC:3+S+R+260101:2400+1'"), "E060");
    let leap = parse("UNB+UNOC:3+S+R+000229:0000+1'").unwrap();
    assert_eq!(leap.prepared.year(), Year::TwoDigit(0));
}

#[test]
fn flags_other_than_1_are_refused() {
    assert_eq!(code("UNB+UNOC:3+S+R+260101:0900+1++++2'"), "E014");
    assert_eq!(code("UNB+UNOC:3+S+R+260101:0900+1++++++2'"), "E014");
}

#[test]
fn a_head_that_is_not_unb_is_refused() {
    assert_eq!(code("UIB+UNOC:3+S+R'"), "E055");
    assert_eq!(code("UNH+1+ORDERS:D:96A:UN'"), "E015");
    assert_eq!(
        InterchangeHeader::read(b"BGM+220'")
            .unwrap_err()
            .stable_code(),
        "E015"
    );
    assert_eq!(
        InterchangeHeader::read(b"").unwrap_err().stable_code(),
        "E015"
    );
}

#[test]
fn read_decodes_the_declared_repertoire() {
    let mut raw = b"\xEF\xBB\xBF\r\nUNB+UNOC:3+M".to_vec();
    raw.push(0xFC);
    raw.extend_from_slice(b"ller+R+260101:0900+1'UNH+1+X:D:96A:UN'");
    assert_eq!(InterchangeHeader::read(&raw).unwrap().sender.id, "Müller");
    let (header, _) = InterchangeHeader::read_from(&raw[..], &ReaderConfig::default()).unwrap();
    assert_eq!(header.sender.id, "Müller");
}

#[test]
fn read_honours_una_and_released_terminators() {
    let raw = b"UNA:+.? ~UNB+UNOC:3+S?~X+R+260101:0900+1~UNH";
    assert_eq!(InterchangeHeader::read(raw).unwrap().sender.id, "S~X");
}

#[test]
fn validate_envelope_carries_the_parsed_header() {
    let raw = format!("{}UNH+1+X:D:96A:UN'BGM+1'UNT+3+1'UNZ+1+1'", FULL);
    let segments: Vec<_> = from_bytes(raw.as_bytes())
        .collect::<Result<_, _>>()
        .unwrap();
    let result = validate_envelope(&segments).unwrap();
    assert_eq!(
        result.interchange.header,
        InterchangeHeader::parse(&segments[0]).unwrap()
    );
}

// ── bounded read_from ──────────────────────────────────────────────────────────

/// A reader over `data` that counts the bytes handed out, in reads of at most
/// `chunk` bytes.
struct Counting<'a> {
    data: &'a [u8],
    pos: usize,
    chunk: usize,
}

impl Read for Counting<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = buf.len().min(self.chunk).min(self.data.len() - self.pos);
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

fn bound() -> usize {
    4096 + 9 + ReaderConfig::default().max_segment_bytes
}

#[test]
fn read_from_consumes_little_of_a_large_interchange() {
    let mut raw = b"UNB+UNOC:3+S+R+260101:0900+1'".to_vec();
    let message = b"UNH+1+X:D:96A:UN'FTX+AAA+0123456789012345678901234567890123456789'UNT+3+1'";
    while raw.len() < 50 << 20 {
        raw.extend_from_slice(message);
    }
    let mut reader = Counting {
        data: &raw,
        pos: 0,
        chunk: usize::MAX,
    };
    let (header, _) = InterchangeHeader::read_from(&mut reader, &ReaderConfig::default()).unwrap();
    assert_eq!(header.control_ref, "1");
    assert!(reader.pos <= bound(), "consumed {}", reader.pos);
}

#[test]
fn the_returned_reader_replays_everything() {
    let raw = b"UNB+UNOC:3+S+R+260101:0900+1'UNH+1+X:D:96A:UN'BGM+1'UNT+3+1'UNZ+1+1'";
    for chunk in [1, 2, 7, 4096] {
        let reader = Counting {
            data: raw,
            pos: 0,
            chunk,
        };
        let (_, mut rest) = InterchangeHeader::read_from(reader, &ReaderConfig::default()).unwrap();
        let mut replay = Vec::new();
        rest.read_to_end(&mut replay).unwrap();
        assert_eq!(replay, raw, "chunk {chunk}");
    }
    let (_, rest) = InterchangeHeader::read_from(&raw[..], &ReaderConfig::default()).unwrap();
    let streamed: Vec<_> = from_reader(rest).collect::<Result<_, _>>().unwrap();
    let whole: Vec<_> = from_bytes(raw).collect::<Result<_, _>>().unwrap();
    assert_eq!(streamed.len(), whole.len());
    for (a, b) in streamed.iter().zip(&whole) {
        assert_eq!(a, b);
    }
}

#[test]
fn hostile_heads_fail_within_the_bound() {
    let endless_prologue = vec![b' '; 1 << 20];
    let mut unterminated = b"UNB+UNOC:3+S+R+260101:0900+".to_vec();
    unterminated.resize(1 << 20, b'X');
    let mut all_released = b"UNB+UNOC:3+S+R+260101:0900+".to_vec();
    while all_released.len() < 1 << 20 {
        all_released.extend_from_slice(b"?'");
    }
    for input in [&endless_prologue, &unterminated, &all_released] {
        let mut reader = Counting {
            data: input,
            pos: 0,
            chunk: 4096,
        };
        let Err(err) = InterchangeHeader::read_from(&mut reader, &ReaderConfig::default()) else {
            panic!("refused")
        };
        assert_eq!(err.stable_code(), "E036");
        assert!(reader.pos <= bound(), "consumed {}", reader.pos);
        assert_eq!(
            InterchangeHeader::read(input).unwrap_err().stable_code(),
            "E036"
        );
    }
}

#[test]
fn a_smaller_segment_budget_shrinks_the_bound() {
    let raw = b"UNB+UNOC:3+S+R+260101:0900+1'";
    let config = ReaderConfig::default().max_segment_bytes(8);
    let mut long = raw.to_vec();
    long.splice(11..11, std::iter::repeat_n(b'S', 5000));
    let Err(err) = InterchangeHeader::read_from(&long[..], &config) else {
        panic!("refused")
    };
    assert_eq!(err.stable_code(), "E036");
}
