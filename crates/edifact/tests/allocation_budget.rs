//! Allocation budget of the parser, measured with a counting allocator.
//!
//! Pins the parser's allocation budget on both parse paths:
//!
//! - `repeats` is unallocated unless an element actually repeats;
//! - each segment's `elements` vector is allocated once, at its exact length;
//! - every segment of `tests/fixtures/orders_conforming.edi` costs exactly the
//!   number of allocator calls its shape allows (the budget below);
//! - peak heap for the 1 MB benchmark fixture stays within committed ceilings.
//!
//! # The budget
//!
//! For one segment with `E` data elements, `X` release-escaped values and `V`
//! non-empty component values:
//!
//! - slice path (`from_bytes`): `[E ≥ 1] + X + S`, where `S` is 1 when the
//!   parser's reusable element buffer has to grow (the first segment with
//!   elements, then at most once per doubling of the largest element count);
//! - reader path (`from_reader`), segment read straight from the buffer:
//!   `[E ≥ 1] + 1 (tag) + V` — an escaped value is already owned and is moved,
//!   not copied, so it is not counted twice;
//! - add one allocation per further repetition-vector growth and per
//!   `SmallVec` spill (a repetition with more than four components);
//! - a reader segment that is assembled byte by byte (the first one after the
//!   `UNA`, or one straddling the read buffer) additionally pays for its raw
//!   byte buffer, and the first one for the 8 KiB `BufReader`.
//!
//! The numeric peak-heap assertions run on 64-bit targets only; sizes differ on
//! 32-bit. Committed figures are asserted as `0.8 × constant ≤ measured ≤
//! constant`: a regression fails, and so does an improvement, so the published
//! number is re-committed instead of going stale.

#[path = "support/counting_alloc.rs"]
mod counting_alloc;

#[allow(dead_code)]
#[path = "../benches/bench_data.rs"]
mod bench_data;

use std::borrow::Cow;
use std::io::Cursor;

use counting_alloc::{CountingAlloc, measure};
use edifact::{Segment, Span, from_bytes, from_reader};

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

const ORDERS: &[u8] = include_bytes!("fixtures/orders_conforming.edi");

/// Allocator calls of each segment on the slice path, construction included
/// in the first.
fn slice_stats(input: &[u8]) -> Vec<usize> {
    let (mut it, setup) = measure(|| from_bytes(input));
    let mut counts = Vec::new();
    let mut extra = setup.allocations;
    loop {
        let (item, stats) = measure(|| it.next());
        match item {
            None => break,
            Some(result) => {
                result.expect("input must parse");
                counts.push(stats.allocations + extra);
                extra = 0;
            }
        }
    }
    counts
}

/// Allocator calls of each segment on the reader path, construction (and so
/// the `BufReader` buffer) included in the first.
fn reader_stats(input: &[u8]) -> Vec<usize> {
    let (mut it, setup) = measure(|| from_reader(Cursor::new(input)));
    let mut counts = Vec::new();
    let mut extra = setup.allocations;
    loop {
        let (item, stats) = measure(|| it.next());
        match item {
            None => break,
            Some(result) => {
                result.expect("input must parse");
                counts.push(stats.allocations + extra);
                extra = 0;
            }
        }
    }
    counts
}

fn assert_within(name: &str, measured: usize, committed: usize) {
    let floor = committed - committed / 5;
    assert!(
        (floor..=committed).contains(&measured),
        "{name}: measured {measured}, committed {committed} (accepted {floor}..={committed}); \
         if the change is intended, re-commit {name} = {measured} and quote it in the CHANGELOG"
    );
}

// ── committed numbers ────────────────────────────────────────────────────────
//
// Measured 2026-10-18 on aarch64-apple-darwin (64-bit; the same type sizes as
// x86_64 Linux, where CI runs), rustc 1.99.0 stable and 1.85.0; debug and
// release give identical figures — they depend on type sizes and the standard
// library's growth policy, not on optimisation. A toolchain change that moves
// them re-commits them in the same change.

/// Peak live heap of `from_bytes(one_mb()).collect::<Vec<_>>()`.
const ONE_MB_SLICE_COLLECT_PEAK: usize = 25_217_536;
/// Peak live heap while streaming `one_mb()` through `from_bytes`.
const ONE_MB_SLICE_STREAM_PEAK: usize = 2_704;
/// Peak live heap while streaming `one_mb()` through `from_reader`.
const ONE_MB_READER_STREAM_PEAK: usize = 9_347;
/// Allocator calls for streaming `one_mb()` through `from_bytes`.
const ONE_MB_SLICE_ALLOCS: usize = 44_013;
/// Allocator calls of the first `orders_conforming.edi` segment on the reader
/// path (`UNH+1+ORDERS:D:11A:UN`, read byte by byte after the `UNA`):
/// 1 `BufReader` buffer + 4 raw-segment buffer (capacity 3, then 6, 12, 24 for
/// 22 bytes including the terminator) + 1 element vector + 1 tag + 5 non-empty
/// component values.
const ORDERS_READER_FIRST_SEGMENT_ALLOCS: usize = 12;

/// Ceiling on parsed heap per input byte: holding one message window (or a
/// collected interchange) of `n` input bytes takes at most `HEAP_FACTOR × n`.
/// The worst case is a message of maximal segments of one-byte elements
/// (`FTX+A+A+…`); the guides quote this figure.
const HEAP_FACTOR: usize = 128;

// ── harness self-test ────────────────────────────────────────────────────────

#[test]
fn counting_alloc_counts_a_single_vec() {
    let (v, stats) = measure(|| Vec::<u8>::with_capacity(10));
    assert_eq!(stats.allocations, 1);
    assert_eq!(stats.peak_bytes, 10);
    drop(v);
}

// ── repetitions cost nothing unless they exist ───────────────────────────────

fn assert_exact_layout(segments: &[Segment<'_>], path: &str) -> usize {
    let mut elements = 0;
    for segment in segments {
        assert_eq!(
            segment.elements.capacity(),
            segment.elements.len(),
            "{path}: {} at {} has a slack element vector",
            segment.tag,
            segment.span
        );
        for element in &segment.elements {
            assert_eq!(
                element.repeats.capacity(),
                0,
                "{path}: element at {} carries a repetition buffer",
                element.span
            );
            elements += 1;
        }
    }
    elements
}

#[test]
fn repeats_unallocated_on_both_paths() {
    let input = bench_data::one_mb();
    let slice: Vec<Segment<'_>> = from_bytes(input)
        .collect::<Result<_, _>>()
        .expect("fixture parses");
    let reader: Vec<Segment<'static>> = from_reader(Cursor::new(input))
        .collect::<Result<_, _>>()
        .expect("fixture parses");
    assert_eq!(assert_exact_layout(&slice, "slice"), 96_024);
    assert_eq!(assert_exact_layout(&reader, "reader"), 96_024);
}

#[test]
fn repeating_element_layout_and_spans() {
    type Occurrence<'s> = Vec<(&'s str, Span)>;
    fn flat<'s>(components: &'s edifact::Components<'_>) -> Occurrence<'s> {
        components
            .iter()
            .map(|(v, span)| (v.as_ref(), *span))
            .collect()
    }
    // Expected layout: the first occurrence stays in `components`, every
    // further one goes to `repeats`, with spans identical on both paths.
    /// Input, element span, first occurrence, further occurrences, segment span.
    type Case<'s> = (&'s [u8], Span, Occurrence<'s>, Vec<Occurrence<'s>>, Span);
    let cases: [Case<'_>; 2] = [
        (
            b"UNA:+.?*'RFF+ON:1*ON:2*ON:3'",
            Span::new(13, 27),
            vec![("ON", Span::new(13, 15)), ("1", Span::new(16, 17))],
            vec![
                vec![("ON", Span::new(18, 20)), ("2", Span::new(21, 22))],
                vec![("ON", Span::new(23, 25)), ("3", Span::new(26, 27))],
            ],
            Span::new(9, 28),
        ),
        (
            b"UNA:+.?*'RFF+ON:1*'",
            Span::new(13, 18),
            vec![("ON", Span::new(13, 15)), ("1", Span::new(16, 17))],
            vec![vec![("", Span::new(18, 18))]],
            Span::new(9, 19),
        ),
    ];
    for (input, element_span, first, further, segment_span) in cases {
        let slice: Vec<Segment<'_>> = from_bytes(input).collect::<Result<_, _>>().unwrap();
        let reader: Vec<Segment<'static>> = from_reader(Cursor::new(input))
            .collect::<Result<_, _>>()
            .unwrap();
        for segments in [&slice[..], &reader[..]] {
            assert_eq!(segments.len(), 1);
            assert_eq!(segments[0].span, segment_span);
            let element = &segments[0].elements[0];
            assert_eq!(element.span, element_span);
            assert_eq!(flat(&element.components), first);
            let repeats: Vec<Occurrence<'_>> = element.repeats.iter().map(flat).collect();
            assert_eq!(repeats, further);
        }
    }
}

// ── per-segment budget ───────────────────────────────────────────────────────

#[test]
fn orders_fixture_slice_budget() {
    // UNH pays one extra allocation: the parser's reusable element buffer
    // grows to its first capacity (4) there and is reused afterwards.
    assert_eq!(slice_stats(ORDERS), [2, 1, 1, 1, 1, 1, 1, 1, 1]);
}

#[test]
fn orders_fixture_reader_budget() {
    let counts = reader_stats(ORDERS);
    assert_eq!(counts.len(), 9);
    // Segments 2–9 come straight from the read buffer: one element vector,
    // one tag, one per non-empty component value.
    //   BGM+220+PO-4711+9            1 + 1 + 3
    //   DTM+137:20260517:102         1 + 1 + 3
    //   NAD+BY+4000001000002::9      1 + 1 + 3
    //   NAD+SU+4000001000001::9      1 + 1 + 3
    //   LIN+1++4012345678901:SRV     1 + 1 + 3
    //   QTY+21:10:PCE                1 + 1 + 3
    //   PRI+AAA:12.50                1 + 1 + 2
    //   UNT+9+1                      1 + 1 + 2
    assert_eq!(counts[1..], [5, 5, 5, 5, 5, 5, 4, 4]);
    assert_eq!(
        counts[0], ORDERS_READER_FIRST_SEGMENT_ALLOCS,
        "UNH (slow path): re-commit ORDERS_READER_FIRST_SEGMENT_ALLOCS if intended"
    );
}

#[test]
fn escaped_value_costs_one_allocation() {
    // BGM warms the slice parser's element buffer to capacity 4, so the FTX
    // figure is the steady state: 1 element vector + 1 escaped value.
    let input: &[u8] = b"BGM+1+2+3+4'FTX+AAA+++A?+B'";
    assert_eq!(slice_stats(input)[1], 2);
    // Reader: 1 element vector + 1 tag + 2 non-empty values; the escaped
    // value is moved into the owned segment, not copied again.
    assert_eq!(reader_stats(input)[1], 4);

    let segments: Vec<Segment<'_>> = from_bytes(input).collect::<Result<_, _>>().unwrap();
    let ftx = &segments[1];
    for element in &ftx.elements {
        for (value, _) in &element.components {
            let escaped = value.as_ref() == "A+B";
            assert_eq!(
                matches!(value, Cow::Owned(_)),
                escaped,
                "only the escaped value may be owned, got {value:?}"
            );
        }
    }
    assert_eq!(ftx.component_str(3, 0), Some("A+B"));

    // A segment without data elements allocates nothing at all.
    assert_eq!(slice_stats(b"BGM+1'UNS'")[1], 0);
    // More than four components spill the SmallVec once: 1 element vector
    // + 1 spill.
    assert_eq!(slice_stats(b"BGM+1+2+3+4'NAD+BY+1:2:3:4:5'")[1], 2);
}

#[cfg(target_pointer_width = "64")]
#[test]
fn one_mb_slice_allocation_total() {
    let input = bench_data::one_mb();
    let ((), stats) = measure(|| {
        let mut segments = 0usize;
        for segment in from_bytes(input) {
            segment.expect("fixture parses");
            segments += 1;
        }
        assert_eq!(segments, 44_011);
    });
    assert!(
        stats.allocations <= 44_011 + 4,
        "{} allocations for 44 011 segments",
        stats.allocations
    );
    assert_within(
        "ONE_MB_SLICE_ALLOCS",
        stats.allocations,
        ONE_MB_SLICE_ALLOCS,
    );
}

// ── committed peak heap for the 1 MB fixture ─────────────────────────────────

#[cfg(target_pointer_width = "64")]
#[test]
fn one_mb_peak_heap_slice_collect() {
    let input = bench_data::one_mb();
    let (segments, stats) = measure(|| {
        from_bytes(input)
            .collect::<Result<Vec<_>, _>>()
            .expect("fixture parses")
    });
    assert_eq!(segments.len(), 44_011);
    drop(segments);
    assert!(
        stats.peak_bytes <= 28 * input.len(),
        "collect peak {} B exceeds 28 × input ({} B)",
        stats.peak_bytes,
        input.len()
    );
    assert_within(
        "ONE_MB_SLICE_COLLECT_PEAK",
        stats.peak_bytes,
        ONE_MB_SLICE_COLLECT_PEAK,
    );
}

#[cfg(target_pointer_width = "64")]
#[test]
fn one_mb_peak_heap_slice_stream() {
    let input = bench_data::one_mb();
    let ((), stats) = measure(|| {
        for segment in from_bytes(input) {
            segment.expect("fixture parses");
        }
    });
    assert!(
        stats.peak_bytes <= 8 * 1024,
        "slice stream peak {} B",
        stats.peak_bytes
    );
    assert_within(
        "ONE_MB_SLICE_STREAM_PEAK",
        stats.peak_bytes,
        ONE_MB_SLICE_STREAM_PEAK,
    );
}

#[cfg(target_pointer_width = "64")]
#[test]
fn one_mb_peak_heap_reader_stream() {
    let input = bench_data::one_mb();
    let ((), stats) = measure(|| {
        for segment in from_reader(Cursor::new(input)) {
            segment.expect("fixture parses");
        }
    });
    assert!(
        stats.peak_bytes <= 24 * 1024,
        "reader stream peak {} B",
        stats.peak_bytes
    );
    assert_within(
        "ONE_MB_READER_STREAM_PEAK",
        stats.peak_bytes,
        ONE_MB_READER_STREAM_PEAK,
    );
}

// ── zero-allocation paths ────────────────────────────────────────────────────

#[test]
fn writer_emitter_allocates_nothing_per_event() {
    use edifact::{EdifactEvent, EventEmitter, WriterEmitter};

    // The buffer is pre-sized so growth of the sink is not what is measured.
    let mut emitter = WriterEmitter::new(Vec::<u8>::with_capacity(64 * 1024));
    let ((), stats) = measure(|| {
        for _ in 0..50 {
            emitter.emit(EdifactEvent::start("FTX")).unwrap();
            emitter.emit(EdifactEvent::element("AAA")).unwrap();
            emitter.emit(EdifactEvent::element("A?B")).unwrap();
            emitter.emit(EdifactEvent::component("C+D")).unwrap();
            emitter.emit(EdifactEvent::component("E:F")).unwrap();
            emitter.emit(EdifactEvent::element("G'H")).unwrap();
            emitter.emit(EdifactEvent::component("plain")).unwrap();
            emitter.emit(EdifactEvent::EndSegment).unwrap();
        }
    });
    assert_eq!(stats.allocations, 0, "{stats:?}");
    assert_eq!(emitter.segment_count(), 50);
    let out = emitter.finish().unwrap();
    assert!(out.starts_with(b"FTX+AAA+A??B:C?+D:E?:F+G?'H:plain'"));
}

const ENVELOPE: &[u8] = b"UNB+UNOC:3+SENDER+RECEIVER+260101:0900+IC1'\
UNG+ORDERS+SND+RCV+260101:0900+GRP1+UN+D:96A'\
UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+3+1'\
UNH+2+INVOIC:D:96A:UN'BGM+380'UNT+3+2'\
UNH+3+ORDERS:D:96A:UN'BGM+220'UNT+3+3'\
UNE+3+GRP1'UNZ+1+IC1'";

/// The first `tag` segment of [`ENVELOPE`], leaked: `parse_unh` and
/// `parse_ung` take `&'a Segment<'a>`, and an owned segment borrowed for its
/// own data lifetime cannot be dropped afterwards.
fn leaked_segment(tag: &str) -> &'static Segment<'static> {
    let segment = from_bytes(ENVELOPE)
        .map(|s| s.expect("envelope parses"))
        .find(|s| s.tag == tag)
        .expect("tag present");
    Box::leak(Box::new(segment))
}

#[test]
fn parse_unh_allocates_nothing() {
    let unh = leaked_segment("UNH");
    let (id, stats) = measure(|| edifact::parse_unh(unh));
    let id = id.expect("UNH parses");
    assert_eq!(stats.allocations, 0, "{stats:?}");
    assert_eq!(id.message_type, "ORDERS");
}

#[test]
fn parse_ung_allocates_nothing() {
    let ung = leaked_segment("UNG");
    let (id, stats) = measure(|| edifact::parse_ung(ung));
    id.expect("UNG parses");
    assert_eq!(stats.allocations, 0, "{stats:?}");
}

#[test]
fn messages_by_type_iter_allocates_nothing() {
    let segments: Vec<Segment<'_>> = from_bytes(ENVELOPE)
        .collect::<Result<_, _>>()
        .expect("envelope parses");
    let validated = edifact::validate_envelope(&segments).expect("valid envelope");
    let (count, stats) = measure(|| validated.iter_messages_by_type("ORDERS").count());
    assert_eq!(stats.allocations, 0, "{stats:?}");
    assert_eq!(count, 2);
}

// ── heap factor of a window ──────────────────────────────────────────────────

/// `max_message_bytes` of the worst-case window: 1 MiB. The factor is linear in
/// the bytes held, so it transfers to the 16 MiB default.
const WINDOW_BYTES: usize = 1 << 20;

/// `UNB`, then a message of exactly [`WINDOW_BYTES`]: `UNH`, `FTX` segments of
/// one-byte elements at the default `max_segment_bytes`, and `UNT`.
fn worst_case_window() -> Vec<u8> {
    const UNH: &[u8] = b"UNH+1+ORDERS:D:96A:UN'";
    const UNT: &[u8] = b"UNT+18+1'";
    // `FTX+A+A…'` of `bytes` bytes, terminator included; an odd size starts
    // with one two-byte element.
    let segment = |bytes: usize| {
        let mut ftx = b"FTX".to_vec();
        if bytes % 2 == 1 {
            ftx.extend_from_slice(b"+AA");
        }
        while ftx.len() < bytes - 1 {
            ftx.extend_from_slice(b"+A");
        }
        ftx.push(b'\'');
        assert_eq!(ftx.len(), bytes);
        ftx
    };
    // A full segment: `max_segment_bytes` before the terminator.
    let full = edifact::ReaderConfig::DEFAULT_MAX_SEGMENT_BYTES + 1;
    let mut input = b"UNB+UNOC:3+S+R+260101:0900+IC1'".to_vec();
    let start = input.len();
    input.extend_from_slice(UNH);
    let mut rest = WINDOW_BYTES - UNH.len() - UNT.len();
    while rest > 0 {
        let bytes = if rest > full { full - 1 } else { rest };
        input.extend_from_slice(&segment(bytes));
        rest -= bytes;
    }
    input.extend_from_slice(UNT);
    assert_eq!(input.len() - start, WINDOW_BYTES);
    input.extend_from_slice(b"UNZ+1+IC1'");
    input
}

/// Measured 2026-11-15 (release, aarch64-apple-darwin): 110.5 × on the slice
/// path, 104.6 × on the reader path, 110.5 × for `collect`.
#[cfg(target_pointer_width = "64")]
#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "parses 3 MiB of one-byte elements; runs in release (`just test-release-limits`)"
)]
fn worst_case_window_heap_within_factor() {
    use edifact::{
        ReaderConfig, from_bytes_with_config, message_windows_from_reader_with_config,
        message_windows_with_config,
    };

    let input = worst_case_window();
    let config = ReaderConfig::default().max_message_bytes(WINDOW_BYTES as u64);
    let check = |path: &str, peak: usize, bytes: usize| {
        assert!(
            peak <= HEAP_FACTOR * bytes,
            "{path}: peak heap {peak} B exceeds {HEAP_FACTOR} × {bytes} B",
        );
        // Not much below the ceiling either: the input really is the worst case.
        assert!(
            peak * 2 > HEAP_FACTOR * bytes,
            "{path}: peak heap {peak} B is far below the ceiling; re-measure HEAP_FACTOR",
        );
    };

    let (window, stats) = measure(|| {
        message_windows_with_config(&input, config)
            .next()
            .expect("one window")
            .expect("within the budget")
    });
    assert_eq!(window.segments.len(), 18);
    drop(window);
    check("message_windows", stats.peak_bytes, WINDOW_BYTES);

    let (window, stats) = measure(|| {
        message_windows_from_reader_with_config(&input[..], config)
            .next()
            .expect("one window")
            .expect("within the budget")
    });
    assert_eq!(window.segments.len(), 18);
    drop(window);
    check(
        "message_windows_from_reader",
        stats.peak_bytes,
        WINDOW_BYTES,
    );

    // Collecting the whole interchange: the same factor over its bytes.
    let (segments, stats) = measure(|| {
        from_bytes_with_config(&input, config)
            .collect::<Result<Vec<_>, _>>()
            .expect("within the budget")
    });
    assert_eq!(segments.len(), 20);
    drop(segments);
    check("from_bytes(..).collect()", stats.peak_bytes, input.len());
}
