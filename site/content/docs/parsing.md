+++
title = "Parsing"
description = "Zero-copy slice parsing, streaming readers, byte spans, UNA handling, and the ReaderConfig budgets that guard against oversized input."
weight = 30
+++

Entry points for reading EDIFACT — byte slices, readers, custom delimiters,
envelope helpers, and the interchange header.

---

## Entry points overview

Each has a `_with_config` sibling that takes a
[`ReaderConfig`](#dos-hardening-with-readerconfig). All but `from_bytes_decoded`
return a **lazy iterator** of `Result<_, EdifactError>`; `from_bytes_decoded`
returns the whole `Result<Vec<OwnedSegment>, _>`.

| Function | Input | Yields | When to use |
|---|---|---|---|
| `from_bytes` | `&[u8]` | `Segment<'input>` | In-memory buffer — the fastest path, borrows |
| `from_reader` | `impl Read` | `OwnedSegment` | A file or socket; holds one segment at a time |
| `from_bufread` | `impl BufRead` | `OwnedSegment` | As above, when you already hold a `BufRead` |
| `from_bytes_decoded` | `&[u8]` | `Vec<OwnedSegment>` | Transcodes from the repertoire the `UNB` declares |
| `from_reader_decoded` | `impl Read` | `OwnedSegment` | The streaming counterpart |

`OwnedSegment` is `Segment<'static>`; every function accepts either — see
[Core Concepts](@/docs/core-concepts.md#one-segment-type-borrowed-or-owned).
Both paths return the same `Result` for every input, however a reader chunks
it, with absolute error offsets.

---

## Zero-copy byte-slice parsing

```rust
use edifact::from_bytes;

let input: &[u8] = b"UNA:+.? 'BGM+220+PO-4711+9'NAD+BY+4000001::9'";

// collect eagerly
let segments: Vec<_> = from_bytes(input).collect::<Result<_, _>>()?;

// or iterate lazily — each segment is parsed when you ask for it
for result in from_bytes(input) {
    let seg = result?;
    println!("{}", seg.tag);
}
# Ok::<(), edifact::EdifactError>(())
```

Tag and component values **borrow from `input`**; `Cow::Owned` appears only
where a release escape was resolved. Each segment allocates its element vector
once; up to four components per element sit inline
([Performance](@/docs/performance.md#allocation-per-segment)).

---

## Accessing segment data

### `element_str(n)` — the most common pattern

```rust
# use edifact::from_bytes;
# let input: &[u8] = b"UNA:+.? 'BGM+220+PO-4711+9'NAD+BY+4000001::9'";
# let segments: Vec<_> = from_bytes(input).collect::<Result<Vec<_>, _>>()?;
let bgm = &segments[0];

// component 0 of element n
assert_eq!(bgm.element_str(0), Some("220"));
assert_eq!(bgm.element_str(99), None); // out-of-bounds → None
# Ok::<(), edifact::EdifactError>(())
```

### `get_element(n)` + `get_component(c)` — composite fields

```rust
# use edifact::from_bytes;
# let input: &[u8] = b"UNA:+.? 'BGM+220+PO-4711+9'NAD+BY+4000001::9'";
# let segments: Vec<_> = from_bytes(input).collect::<Result<Vec<_>, _>>()?;
// NAD element 1 is composite: party_id : qualifier : code_list_qual
let nad = &segments[1];
let party_id = nad
    .get_element(1)
    .and_then(|e| e.get_component(0))
    .unwrap_or("unknown");
let code = nad
    .get_element(1)
    .and_then(|e| e.get_component(2))
    .unwrap_or("");
assert_eq!(party_id, "4000001");
assert_eq!(code, "9");
# Ok::<(), edifact::EdifactError>(())
```

### `component_or_empty(n)` — avoid `Option` unwrapping

```rust
# use edifact::from_bytes;
# let input: &[u8] = b"UNA:+.? 'BGM+220+PO-4711+9'NAD+BY+4000001::9'";
# let segments: Vec<_> = from_bytes(input).collect::<Result<Vec<_>, _>>()?;
# let seg = &segments[1];
let elem = seg.get_element(1).unwrap();
let val = elem.component_or_empty(0); // "" if absent, no panic
assert_eq!(val, "4000001");
# Ok::<(), edifact::EdifactError>(())
```

---

## Byte spans

Every `Segment`, `Element` and component carries a `Span { start, end }` into
the input.

```rust
# use edifact::from_bytes;
# let input: &[u8] = b"UNA:+.? 'BGM+220+PO-4711+9'NAD+BY+4000001::9'";
# let segments: Vec<_> = from_bytes(input).collect::<Result<Vec<_>, _>>()?;
let seg = &segments[0];
println!("segment spans bytes {}..{}", seg.span.start, seg.span.end);

let elem = seg.get_element(0).unwrap();
println!("element spans bytes {}..{}", elem.span.start, elem.span.end);

if let Some(span) = elem.component_span(0) {
    let raw = &input[span.start..span.end];
    assert_eq!(raw, b"220");
}
# Ok::<(), edifact::EdifactError>(())
```

Spans are absolute on both paths; the `diagnostics` feature renders them.

---

## UNA handling

Every entry point reads a leading `UNA` (after an optional byte order mark and
whitespace):

- **Present**: its six service characters apply.
- **Absent**: defaults `:` `+` `.` `?` `'`; repetition separator `*` only when
  `UNB` S001 declares syntax version 4.
- **Malformed**: `InvalidUna`.
- **After the first segment**: a `UNA` opens a second interchange (ISO 9735-2
  §5), so parsing fails with `DataAfterInterchangeTrailer` (`E058`). Envelope
  validation reports any other segment after `UNZ` the same way. Split
  concatenated interchanges after each `UNZ`.

```rust
# use edifact::from_bytes;
// Custom delimiters via UNA: `;` component, `|` element, `!` terminator.
let custom = b"UNA;|.? !BGM|220|PO-4711|9!";
//                ││   │└── segment terminator
//                ││   └─── repetition separator (space = not used)
//                │└─────── element separator
//                └──────── component separator
let segs: Vec<_> = from_bytes(custom).collect::<Result<_, _>>()?;
assert_eq!(segs[0].tag, "BGM");
assert_eq!(segs[0].element_str(0), Some("220"));
assert_eq!(segs[0].element_str(1), Some("PO-4711"));
# Ok::<(), edifact::EdifactError>(())
```

```rust
# use edifact::{EdifactError, from_bytes};
let two = b"UNB+UNOC:3+S+R+260101:0900+1'UNZ+0+1'UNA:+.? 'UNB+UNOC:3+S+R+260101:0900+2'";
let err = from_bytes(two).collect::<Result<Vec<_>, _>>().unwrap_err();
assert_eq!(err.stable_code(), "E058");
```

---

## Reader-based parsing

### `from_reader` — streaming, one segment at a time

```rust,no_run
use edifact::from_reader;
use std::fs::File;

let f = File::open("large_interchange.edi")?;
for result in from_reader(f) {
    let seg = result?;               // OwnedSegment
    println!("{}", seg.tag);
}
# Ok::<(), edifact::EdifactError>(())
```

`from_reader` holds one segment (at most `max_segment_bytes`) plus its read
buffer. Collecting costs up to 128 × the input in heap:

```rust,no_run
use edifact::from_reader;
use std::fs::File;

let f = File::open("message.edi")?;
let segments: Vec<_> = from_reader(f).collect::<Result<Vec<_>, _>>()?;
# Ok::<(), edifact::EdifactError>(())
```

`from_reader` wraps its argument in a `BufReader`; pass an existing `BufRead`
to `from_bufread` instead.

---

## DoS hardening with `ReaderConfig`

`ReaderConfig` carries five budgets. Each is a **hard cap that reports an
error**, never a quiet end of the iterator. Three are on by default on every
entry point.

| Field | Default | Raised as |
|---|---|---|
| `max_segment_bytes` | 65 536 (64 KiB, `DEFAULT_MAX_SEGMENT_BYTES`) | `SegmentTooLong { offset, limit }` (E020) |
| `max_input_bytes` | 268 435 456 (256 MiB, `DEFAULT_MAX_INPUT_BYTES`) | `LimitExceeded { limit: "max_input_bytes", max, offset }` (E036) |
| `max_message_bytes` | 16 777 216 (16 MiB, `DEFAULT_MAX_MESSAGE_BYTES`) | `LimitExceeded { limit: "max_message_bytes", max, offset }` (E036) |
| `max_segments` | unlimited | `LimitExceeded { limit: "max_segments", max, offset }` (E036) |
| `max_messages` | unlimited | `LimitExceeded { limit: "max_messages", max, offset }` (E036) |

`max_message_bytes` counts from a `UNH` through its `UNT` terminator and bounds
a message window. A window, like a collected interchange, takes at most 128 ×
its input bytes in heap ([Performance](@/docs/performance.md#memory-by-entry-point)).

The config applies to every `*_with_config` entry point, the decoding variants,
message windows and the recovering scan.

```rust
use edifact::{ReaderConfig, from_bytes_with_config};

let config = ReaderConfig::default()
    .max_segment_bytes(8_192)   // reject any single segment over 8 KiB
    .max_message_bytes(1 << 20) // …any message over 1 MiB
    .max_input_bytes(4 << 20);  // …or more than 4 MiB in total

let segments: Vec<_> = from_bytes_with_config(b"BGM+220+test'", config)
    .collect::<Result<_, _>>()?;
assert_eq!(segments.len(), 1);
# Ok::<(), edifact::EdifactError>(())
```

Trusted input opts out explicitly. A long-lived stream drops the total and keeps
each message capped:

```rust
use edifact::ReaderConfig;

let archive = ReaderConfig::unbounded();                          // no limit at all
let stream = ReaderConfig::unbounded().max_message_bytes(16 << 20); // windows still capped
assert_eq!(stream.max_input_bytes, None);
# let _ = archive;
```

### Limits are errors

A quiet stop would let `collect::<Result<_, _>>()?` succeed on a **truncated**
interchange. A limit is reported instead:

```rust
use edifact::{EdifactError, ReaderConfig, from_bytes_with_config};

let config = ReaderConfig::default().max_segments(1);
let err = from_bytes_with_config(b"BGM+220'DTM+137'", config)
    .collect::<Result<Vec<_>, _>>()
    .unwrap_err();
assert!(matches!(
    err,
    EdifactError::LimitExceeded { limit: "max_segments", max: 1, .. },
));
```

Input ending exactly at a limit finishes normally. Segments before the
violation are still yielded. A segment that would end past `max_input_bytes`
is never yielded.

---

## Parsing a fragment: `service_string_advice`

A **fragment** — a message lifted out of its interchange — has no `UNA` and no
`UNB`, so the defaults would mis-split it. Supply the delimiters:

```rust
use edifact::{ReaderConfig, ServiceStringAdvice, from_bytes_with_config};

// The interchange this message came from declared `;` and `~`.
let ssa = ServiceStringAdvice::from_bytes(b"UNA:;.? ~")?;
let config = ReaderConfig::default().with_service_string_advice(ssa);

let segments: Vec<_> = from_bytes_with_config(b"BGM;220;PO-4711~", config)
    .collect::<Result<Vec<_>, _>>()?;
assert_eq!(segments[0].element_str(1), Some("PO-4711"));
# Ok::<(), edifact::EdifactError>(())
```

An explicit advice outranks a `UNA` in the input — useful when a partner's
header disagrees with its data. A malformed `UNA` is still an error.

---

## Finding segments

`SegmentSliceExt` adds lookups to any segment slice — borrowed, or owned from
the reader path. The qualifier is component 0 of element 0; `Segment::is`
tests one segment inside a closure:

```rust
use edifact::{OwnedSegment, SegmentSliceExt, from_bytes, from_reader};

let input = b"UNH+1+ORDERS:D:11A:UN'\
              BGM+220+PO-4711+9'\
              NAD+BY+4000001::9'\
              NAD+SU+4000002::9'\
              RFF+Z13:17009'\
              UNT+6+1'";
let borrowed: Vec<_> = from_bytes(input).collect::<Result<_, _>>()?;
let owned: Vec<OwnedSegment> = from_reader(&input[..]).collect::<Result<_, _>>()?;

for segs in [&borrowed[..], &owned[..]] {
    assert_eq!(segs.first_tag("BGM").and_then(|s| s.element_str(1)), Some("PO-4711"));
    assert_eq!(segs.qualified("NAD", "SU").and_then(|s| s.element_str(1)), Some("4000002"));
    assert_eq!(segs.qualified_value("RFF", "Z13", 0, 1), Some("17009"));
    assert_eq!(segs.tagged("NAD").count(), 2);
    assert_eq!(segs.iter().position(|s| s.is("NAD", "SU")), Some(3));
}
# Ok::<(), edifact::EdifactError>(())
```

---

## Error handling

All parse errors carry stable codes and byte offsets:

```rust
use edifact::{EdifactError, from_bytes};

let bad_input = b"BGM+incomplete"; // no segment terminator
let err = from_bytes(bad_input)
    .collect::<Result<Vec<_>, _>>()
    .unwrap_err();
assert!(matches!(err, EdifactError::UnexpectedEof { offset: 14 }));
```

A segment without terminator is a **truncation** (`E001`), so a file cut off
mid-transfer never parses as complete. All variants and codes:
[Error Reference](@/docs/error-reference.md).

---

## Strict vs. lenient envelope validation

Parsing checks syntax, not the envelope: it yields every segment even when
`UNB`/`UNZ` or `UNH`/`UNT` disagree. `validate_envelope` enforces the envelope:

```rust
use edifact::{from_bytes, validate_envelope};

let input = b"UNB+UNOA:1+SENDER+RECEIVER+200101:0900+IC1'\
              UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+3+1'\
              UNZ+1+IC1'";
let segs: Vec<_> = from_bytes(input).collect::<Result<_, _>>()?;
validate_envelope(&segs)?; // Err on the first violation
# Ok::<(), edifact::EdifactError>(())
```

`validate_envelope_lenient` collects **every** violation in a `LenientResult`
alongside the recovered interchange:

```rust
use edifact::{from_bytes, validate_envelope_lenient};

// UNZ references a different control reference *and* UNT declares a wrong count.
let input = b"UNB+UNOA:1+SENDER+RECEIVER+200101:0900+IC1'\
              UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+99+1'\
              UNZ+1+IC2'";
let segs: Vec<_> = from_bytes(input).collect::<Result<_, _>>()?;
let result = validate_envelope_lenient(&segs);
assert!(result.errors.len() >= 2); // both problems reported
# Ok::<(), edifact::EdifactError>(())
```

`interchange` is `None` when the `UNB` cannot be typed (unknown repertoire or
version, a missing party, S004 or control reference); the reasons are in
`errors`.

---

## Reading the interchange header first

`InterchangeHeader::read_from` reads the optional `UNA` and the `UNB`, nothing
further, and returns a reader that replays those bytes — route before parsing
the body:

```rust
use edifact::{InterchangeHeader, ReaderConfig, SyntaxVersion, Year, from_reader};

let raw: &[u8] = b"UNB+UNOC:3+SENDER:14+RECEIVER:14+260802:0915+REF1++++1'\
                   UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+3+1'UNZ+1+REF1'";
let (header, rest) = InterchangeHeader::read_from(raw, &ReaderConfig::default())?;
assert_eq!(header.sender.id, "SENDER");
assert_eq!(header.sender.qualifier.as_deref(), Some("14"));
assert_eq!(header.syntax_version, SyntaxVersion::V3);
assert!(header.ack_requested());
// Versions 1–3 transfer no century; state it.
assert_eq!(header.prepared.year(), Year::TwoDigit(26));
assert_eq!(header.prepared.with_century(20)?.year(), Year::Full(2026));

let segments: Vec<_> = from_reader(rest).collect::<Result<_, _>>()?;
assert_eq!(segments.len(), 5);
# Ok::<(), edifact::EdifactError>(())
```

It consumes at most 4 096 (prologue) + 9 (`UNA`) + `max_segment_bytes` bytes;
no terminator within that is `E036`. Values are decoded with the repertoire S001
names. Nothing is defaulted: a missing S001 is `E008`, syntax `X` `E059`, an
invalid S004 `E060`, DE 0031/0035 other than `1` `E014`.
`InterchangeHeader::read(&[u8])` does the same on a slice,
`InterchangeHeader::parse(&segment)` on a parsed `UNB`, and
`validate_envelope(..)?.interchange.header` is the same value.

---

## Next steps

- [Writing](@/docs/writing.md) — serialize segments back to EDIFACT bytes
- [Typed Derive](@/docs/typed-derive.md) — map segments to strongly-typed Rust structs
- [Streaming](@/docs/streaming.md) — process multi-message interchanges lazily
