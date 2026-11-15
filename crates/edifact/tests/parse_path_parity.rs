//! The slice and reader parse paths give one answer.
//!
//! For every input, `from_bytes` and `from_bufread`/`from_reader` return the
//! same `Result` — the same segments with the same spans, or the same error
//! with the same code, offset and payload — whatever chunks the reader hands
//! out. One regression test per divergence class the differential probe found,
//! plus an exhaustive sweep over short inputs. The random-mode counterpart is
//! `fuzz_parity_raw` / `fuzz_parity_structured` in `bolero_harness.rs`.

#[path = "common/parity.rs"]
mod parity;

use edifact::{EdifactError, ReaderConfig, ServiceStringAdvice};
use parity::{ParseResult, assert_decoded_parity, assert_full_parity, parse_both};

/// Assert the slice path and every chunking of the reader path return
/// `expected` exactly.
#[track_caller]
fn assert_parity_with(input: &[u8], config: ReaderConfig, expected: &ParseResult) {
    let (slice, readers) = parse_both(input, config);
    let shown = String::from_utf8_lossy(input);
    assert_eq!(&slice, expected, "slice path on {shown:?}");
    for (label, reader) in &readers {
        assert_eq!(
            reader, expected,
            "reader ({label}) on {shown:?}; slice gave {slice:?}"
        );
    }
}

#[track_caller]
fn assert_parity(input: &[u8], expected: &ParseResult) {
    assert_parity_with(input, ReaderConfig::default(), expected);
}

#[track_caller]
fn assert_error(input: &[u8], error: EdifactError) {
    assert_parity(input, &Err(error));
}

#[track_caller]
fn assert_error_with(input: &[u8], config: ReaderConfig, error: EdifactError) {
    assert_parity_with(input, config, &Err(error));
}

/// The common result, after asserting every path agrees and succeeds.
#[track_caller]
fn parse_ok(input: &[u8], config: ReaderConfig) -> Vec<edifact::OwnedSegment> {
    assert_full_parity(input, config);
    parity::slice_result(input, config).expect("both paths must accept the input")
}

fn e002(byte: u8, offset: usize) -> EdifactError {
    EdifactError::InvalidDelimiter { byte, offset }
}

fn e001(offset: usize) -> EdifactError {
    EdifactError::UnexpectedEof { offset }
}

fn e006(tag: &str) -> EdifactError {
    EdifactError::InvalidSegmentTag(tag.to_owned())
}

// ── D1: a terminator where a tag is required ─────────────────────────────────

#[test]
fn d1_doubled_terminator_is_e002_on_both_paths() {
    assert_error(b"UNH+1+X:D:96A:UN''BGM+1'UNT+3+1'", e002(b'\'', 17));
}

#[test]
fn d1_terminator_at_head_is_e002() {
    assert_error(b"'", e002(b'\'', 0));
    assert_error(b"\r\n'", e002(b'\'', 2));
}

#[test]
fn d1_whitespace_only_slot_is_e002() {
    assert_error(b"UUU' '", e002(b'\'', 5));
}

// ── D2: error offsets are absolute ───────────────────────────────────────────

#[test]
fn d2_reader_error_offsets_are_absolute() {
    assert_error(b" +'", e002(b'+', 1));
    assert_error(b"BGM+1'+'", e002(b'+', 6));
    assert_error(b"BGM+1'FTX+\xFF'", EdifactError::InvalidText { offset: 10 });
    assert_error(
        b"BGM+1'FTX+a?",
        EdifactError::InvalidReleaseSequence { offset: 11 },
    );
}

// ── D3: end of input inside a segment ────────────────────────────────────────

#[test]
fn d3_first_fault_wins_at_end_of_input() {
    assert_error(b"+", e002(b'+', 0));
    assert_error(b"+?", e002(b'+', 0));
    assert_error(b"UNH+\xFC", EdifactError::InvalidText { offset: 4 });
    assert_error(b"UNT+3+1'B?", e006("B?"));
    assert_error(
        b"FTX+AAA++x?",
        EdifactError::InvalidReleaseSequence { offset: 10 },
    );
    // Nothing wrong among the bytes present: the input length.
    assert_error(b"BGM+220", e001(7));
    assert_error(b"BGM+220\n", e001(8));
}

#[test]
fn d3a_tag_cut_by_eof_is_e001_on_both_paths() {
    assert_error(b"UNT+3+1'U", e001(9));
    assert_error(b"UN", e001(2));
    assert_error(b"U", e001(1));
    assert_error(b"BGM", e001(3));
    // A non-tag byte is a fault before the end of input is.
    assert_error(b"U'", e006("U"));
    assert_error(b"Ux", e006("Ux"));
}

// ── D4: a truncated head `UNA` ───────────────────────────────────────────────

#[test]
fn d4_truncated_una_is_e001_on_both_paths() {
    assert_error(b"UNA'", e001(4));
    assert_error(b"UNA+?", e001(5));
    assert_error(b"UNA:+.?", e001(7));
    assert_error(b"UNA", e001(3));
    assert_error(b"UNA+X'", e001(6));
    assert_error(b"\n UNA:+.? ", e001(10));
}

#[test]
fn d4_service_string_advice_from_bytes_rejects_truncated_una() {
    assert_eq!(ServiceStringAdvice::from_bytes(b"UNA:+"), Err(e001(5)));
    // A complete one, and input without a `UNA`, are unaffected.
    assert!(ServiceStringAdvice::from_bytes(b"UNA:+.? '").is_ok());
    assert!(ServiceStringAdvice::from_bytes(b"UN").is_ok());
}

// ── D5: the prologue under any chunking ──────────────────────────────────────

#[test]
fn d5_bom_split_across_fills_is_still_prologue() {
    for (input, first_segment) in [
        ("\u{FEFF}UNB+UNOA:1+S+R+200101:0900+1'UNZ+0+1'", 3),
        ("\u{FEFF}UNA:+.? 'BGM+1'", 12),
    ] {
        let segments = parse_ok(input.as_bytes(), ReaderConfig::default());
        assert_eq!(segments[0].span.start, first_segment, "{input:?}");
    }
    // A partial mark is not a prologue: it is a (malformed) tag on both paths.
    assert_error(b"\xEF\xBBUNB'", e006("[239, 187, 85, 78, 66]"));
}

#[test]
fn long_whitespace_run_is_not_retained() {
    // 8 MiB of inter-segment whitespace between two segments: the reader
    // consumes it as it scans, never as part of a segment. A segment budget
    // far below the run proves no whitespace is counted into (or buffered
    // with) the following segment, and the read buffer keeps its capacity.
    let mut input = b"BGM+1'".to_vec();
    input.resize(input.len() + 8 * 1024 * 1024, b'\n');
    input.extend_from_slice(b"BGM+22'");
    let config = ReaderConfig::default().max_segment_bytes(16);

    let slice = parity::slice_result(&input, config);
    let mut reader = std::io::BufReader::with_capacity(8192, &input[..]);
    let streamed: ParseResult = edifact::from_bufread_with_config(&mut reader, config).collect();
    assert_eq!(streamed, slice);
    assert_eq!(reader.capacity(), 8192);
    let segments = slice.expect("both segments parse");
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[1].span.start, input.len() - 7);
}

// ── D6: only the head `UNB` sets the repetition separator ────────────────────

#[test]
fn d6_only_the_head_unb_sets_the_repetition_separator() {
    let segments = parse_ok(
        b"BGM+1'UNB+UNOC:4+S+R+20260101:0900+1'FTX+A*B'",
        ReaderConfig::default(),
    );
    assert_eq!(segments[2].tag, "FTX");
    assert_eq!(segments[2].elements[0].repeat_count(), 1);
}

#[test]
fn d6_head_unb_is_read_under_its_own_advice() {
    let segments = parse_ok(
        b"UNB+UNOC:4+S+R+20260101:0900*X+1'UNZ+0+1'",
        ReaderConfig::default(),
    );
    assert_eq!(segments[0].elements[3].repeat_count(), 2);
}

#[test]
fn d6_the_syntax_version_is_read_within_the_segment_budget() {
    // The budget ends inside S001, before its version: neither path may use
    // the version it cannot see, so `*` stays data and the bad byte is
    // reported where its value starts.
    assert_error_with(
        b"UNB+a*\xFF:4+S'",
        ReaderConfig::default().max_segment_bytes(7),
        EdifactError::InvalidText { offset: 4 },
    );
}

// ── D7: `max_segment_bytes` counts tag to terminator ─────────────────────────

#[test]
fn d7_segment_budget_excludes_inter_segment_whitespace() {
    // `UNB` and `UNH` are 28 bytes each, tag to terminator; the `UNH` carries
    // embedded whitespace (counted) and is preceded by `\r\n` (not counted).
    let input = b"UNB+UNOA:1+S+R+200101:0900+1'\r\nUNH+1+INVOIC:D:96A \n  \n\n :UN'BGM+380+1'UNT+3+1'UNZ+1+1'";
    let segments = parse_ok(input, ReaderConfig::default().max_segment_bytes(28));
    assert_eq!(segments.len(), 5);

    let input = b"BGM+1'\n\n  BGM+22'";
    let segments = parse_ok(input, ReaderConfig::default().max_segment_bytes(6));
    assert_eq!(segments.len(), 2);
    assert_error_with(
        input,
        ReaderConfig::default().max_segment_bytes(5),
        EdifactError::SegmentTooLong {
            offset: 10,
            limit: 5,
        },
    );
}

#[test]
fn d7_an_earlier_fault_wins_over_the_segment_budget() {
    // The segment is too long, but its second byte is already wrong.
    assert_error_with(
        b"BGM+\xFF+aaaaaaaaaaaaaaaa'",
        ReaderConfig::default().max_segment_bytes(10),
        EdifactError::InvalidText { offset: 4 },
    );
    // A release character in the budget's last byte escapes a byte beyond it.
    assert_error_with(
        b"BGM+aaaaa?aaaa'",
        ReaderConfig::default().max_segment_bytes(9),
        EdifactError::SegmentTooLong {
            offset: 0,
            limit: 9,
        },
    );
}

// ── D8: the release character does not escape a tag ─────────────────────────

#[test]
fn d8_release_char_in_tag_is_e006() {
    assert_error(b"B?", e006("B?"));
    assert_error(b"B?'X'", e006("B?"));
    // A `UNA` declaring `'` as release character and `!` as terminator.
    assert_error(b"UNA:+.' !B'!", e006("B'"));
}

// ── D9: chunking cannot change the result ────────────────────────────────────

#[test]
fn d9_chunking_does_not_change_the_result() {
    assert_error(b"UNH+1+X:D:96A:UN'\n'BGM+1'UNT++1'", e002(b'\'', 18));
    assert_error(b"UUU' '", e002(b'\'', 5));
}

// ── well-formed input and edge cases ─────────────────────────────────────────

#[test]
fn well_formed_input_is_identical_including_spans() {
    let input = b"UNA:+.? 'UNB+UNOC:3+S+R+200101:0900+1'\r\nUNH+1+ORDERS:D:96A:UN'BGM+220+PO?+1'UNT+3+1'\nUNZ+1+1'\n";
    let segments = parse_ok(input, ReaderConfig::default());
    assert_eq!(segments.len(), 5);
    assert_eq!(segments[0].span.start, 9);
}

#[test]
fn empty_and_whitespace_only_input_is_ok_and_empty() {
    for input in [
        &b""[..],
        b" ",
        b"\r\n\t ",
        b"\xEF\xBB\xBF",
        b"\xEF\xBB\xBF\n",
    ] {
        assert_parity(input, &Ok(Vec::new()));
    }
    assert_parity(b"UNA:+.? '", &Ok(Vec::new()));
}

#[test]
fn a_una_that_is_not_at_the_head_is_e058() {
    let e058 = |start| EdifactError::DataAfterInterchangeTrailer {
        tag: "UNA".to_owned(),
        span: edifact::Span::new(start, start + 3),
    };
    assert_error(b"BGM+220'UNA+XXXXXX'", e058(8));
    assert_error(
        b"UNB+UNOC:3+S+R+260101:0900+1'UNZ+0+1'UNA:+.? 'UNB+UNOC:3+S+R+260101:0900+2'",
        e058(37),
    );
}

// ── derived entry points ─────────────────────────────────────────────────────

#[test]
fn derived_decoded_pair_agrees() {
    let unoc = b"UNB+UNOC:3+S+R+200101:0900+1'";
    let mut inputs: Vec<Vec<u8>> = vec![
        [&unoc[..], b"NAD+BY+M\xFCller'UNZ+0+1'"].concat(),
        b"UNB+UNOY:4+S+R+200101:0900+1'RFF+ON:1*ON:2'UNZ+0+1'".to_vec(),
    ];
    for tail in [
        &b"UNH+1+X:D:96A:UN''BGM+1'"[..],
        b"BGM+1'+'",
        b"UNT+3+1'B?",
        b"BGM+1'UNB+UNOC:4+S+R+20260101:0900+1'FTX+A*B'",
    ] {
        inputs.push([&unoc[..], tail].concat());
    }
    // Prologues around the sniff window, inside and beyond the reader's head.
    let body = [&unoc[..], b"NAD+BY+M\xFCller'UNZ+0+1'"].concat();
    for prologue in [
        &b"\xEF\xBB\xBFUNA:+.? '"[..],
        &[b'\n'; 4096],
        &[b'\n'; 4097],
        &[b' '; 9000],
    ] {
        inputs.push([prologue, &body].concat());
        inputs.push(prologue.to_vec());
    }
    for input in &inputs {
        assert_decoded_parity(input, ReaderConfig::default());
    }
}

#[test]
fn derived_configured_pair_agrees() {
    let input = b"UNB+UNOA:1+S+R+200101:0900+1'\r\nUNH+1+INVOIC:D:96A \n  \n\n :UN'BGM+380+1'UNT+3+1'UNZ+1+1'";
    for limit in [27, 28, 29] {
        assert_full_parity(input, ReaderConfig::default().max_segment_bytes(limit));
    }
    assert_full_parity(input, ReaderConfig::default().max_segments(2));
    assert_full_parity(input, ReaderConfig::default().max_messages(0));
    assert_full_parity(input, ReaderConfig::default().max_message_bytes(20));
    assert_full_parity(input, ReaderConfig::default().max_input_bytes(40));
    let advice = ServiceStringAdvice::for_syntax_version(Some(4));
    assert_full_parity(
        b"BGM+1'FTX+A*B'",
        ReaderConfig::default().with_service_string_advice(advice),
    );
}

// ── exhaustive sweep ─────────────────────────────────────────────────────────

const ALPHABET: &[u8] = b"UNABH:+?'* \n";

/// Every input up to `max_len` bytes over [`ALPHABET`], parsed by the slice
/// path and by readers of buffer size 1, 2, 3, 7 and 8 192; returns the number
/// of inputs checked.
fn sweep(max_len: usize) -> usize {
    let mut checked = 0;
    let mut input = Vec::with_capacity(max_len);
    for len in 0..=max_len {
        let mut digits = vec![0usize; len];
        loop {
            input.clear();
            input.extend(digits.iter().map(|&d| ALPHABET[d]));
            let slice = parity::slice_result(&input, ReaderConfig::default());
            for chunk in [1, 2, 3, 7, 8192] {
                let reader: ParseResult =
                    edifact::from_bufread(parity::ChunkedBufRead::new(&input, chunk)).collect();
                assert_eq!(
                    reader,
                    slice,
                    "slice and reader (chunk {chunk}) disagree on {:?}",
                    String::from_utf8_lossy(&input),
                );
            }
            checked += 1;
            // Odometer increment; done when every digit wrapped.
            let mut i = 0;
            while i < len {
                digits[i] += 1;
                if digits[i] < ALPHABET.len() {
                    break;
                }
                digits[i] = 0;
                i += 1;
            }
            if i == len {
                break;
            }
        }
    }
    checked
}

#[test]
fn exhaustive_short_inputs_agree() {
    if cfg!(debug_assertions) {
        assert_eq!(sweep(4), 22_621);
    } else {
        assert_eq!(sweep(5), 271_453);
    }
}

#[test]
#[ignore = "3.3 M inputs; run with --release -- --ignored"]
fn exhaustive_length_6_agrees() {
    assert_eq!(sweep(6), 3_257_437);
}

// ── decoded pair: undefined bytes, limits, fault order ───────────────────────

#[test]
fn undefined_byte_is_e003_at_its_raw_offset_on_both_paths() {
    let unog = b"UNB+UNOG:3+S+R+260101:0900+1'UNH+1+X:D:96A:UN'NAD+\xA5'UNT+3+1'UNZ+1+1'";
    let unoi = b"UNB+UNOI:3+S+R+260101:0900+1'UNH+1+X:D:96A:UN'NAD+\xA1'UNT+3+1'UNZ+1+1'";
    for input in [&unog[..], &unoi[..]] {
        assert_decoded_parity(input, ReaderConfig::default());
        assert_eq!(
            edifact::from_bytes_decoded(input),
            Err(EdifactError::InvalidText { offset: 50 })
        );
        let items: Vec<_> = edifact::from_reader_decoded(input).collect();
        assert_eq!(items.len(), 3, "UNB, UNH, then the error");
        assert_eq!(
            items.last(),
            Some(&Err(EdifactError::InvalidText { offset: 50 }))
        );
    }
    // A fault or limit before the undefined byte is reported first.
    let early = b"UNB+UNOG:3+S+R+260101:0900+1''\xA5'";
    assert_decoded_parity(early, ReaderConfig::default());
    assert_eq!(edifact::from_bytes_decoded(early), Err(e002(b'\'', 29)));
    assert_decoded_parity(&unog[..], ReaderConfig::default().max_segment_bytes(0));
}

#[test]
fn a_long_prologue_reports_the_first_limit_on_both_paths() {
    let body = b"UNB+UNOC:3+S+R+200101:0900+1'UNZ+0+1'";
    // 10 000 spaces run past the reader's 8 KiB head; 5 000 are resolved in it.
    for prologue in [10_000, 5_000] {
        let input = [&vec![b' '; prologue][..], body].concat();
        let budget = |m: u64| Some(("max_input_bytes", m, m as usize));
        let window = Some(("sniff_window", 4096, prologue));
        let o = prologue as u64;
        for (max, expected) in [
            (Some(100), budget(100)),
            (Some(o - 1), budget(o - 1)),
            (Some(o), window),
            (Some(o + 1), window),
            (None, window),
        ] {
            let mut config = ReaderConfig::default();
            config.max_input_bytes = max;
            assert_decoded_parity(&input, config);
            let got = match edifact::from_bytes_decoded_with_config(&input, config) {
                Err(EdifactError::LimitExceeded {
                    limit, max, offset, ..
                }) => Some((limit, max, offset)),
                _ => None,
            };
            assert_eq!(
                got, expected,
                "prologue {prologue}, max_input_bytes {max:?}"
            );
        }
    }
}

#[test]
fn a_fault_after_a_valid_segment_follows_it_on_both_paths() {
    let unb = b"UNB+UNOC:3+S+R+260101:0900+1'";
    for tail in [
        &b"\xFF"[..],
        b"'",
        b"+",
        b":",
        b"?",
        b"UN",
        b"U",
        b"UNA:+.? '",
        b"\xFCX'",
        b"1'",
        b"unh'",
        b"UNH\xFF'",
        b"   \xFF",
        b"\n'",
        b"*",
    ] {
        let input = [&unb[..], tail].concat();
        assert_full_parity(&input, ReaderConfig::default());
        assert_decoded_parity(&input, ReaderConfig::default());
        let mut items = edifact::from_bytes(&input);
        assert_eq!(
            items.next().map(|r| r.map(|s| s.tag().to_owned())),
            Some(Ok("UNB".to_owned())),
            "tail {tail:?}"
        );
        assert!(
            matches!(items.next(), Some(Err(_))),
            "tail {tail:?}: the fault follows the UNB"
        );
    }
}
