+++
title = "Streaming"
description = "Process large interchanges one segment or one message at a time with reader iterators, message windows, and typed extraction."
weight = 60
+++

Process large interchanges one segment or one message at a time. Reader APIs
are synchronous (`std::io::Read`); see
[Async Integration](@/docs/async-integration.md) for async runtimes.

---

## API overview

| API | Source | Output | Memory model |
|---|---|---|---|
| `from_reader(reader)` | `impl Read` | `Iterator<Item = Result<OwnedSegment, _>>` | One segment |
| `message_windows(input)` | `&[u8]` | `Iterator<Item = Result<MessageWindow<'_>, _>>` | One message window |
| `message_windows_from_reader(reader)` | `impl Read` | `Iterator<Item = Result<MessageWindow, _>>` | One message window |
| `deserialize_each(input)` | `&[u8]` | `Iterator<Item = Result<T, _>>` | One segment |
| `deserialize_each_from_reader(reader)` | `impl Read` | `Iterator<Item = Result<T, _>>` | One segment |
| `deserialize_messages(input)` | `&[u8]` | `Iterator<Item = Result<T, _>>` | One message window |
| `deserialize_messages_from_reader(reader)` | `impl Read` | `Iterator<Item = Result<T, _>>` | One message window |

Each is a lazy iterator: first match is `.next()`, all matches `.collect()`.

Each is bounded by default
([`ReaderConfig`](@/docs/parsing.md#dos-hardening-with-readerconfig)): 64 KiB
per segment, 16 MiB per message window (≤ 128 × that in heap), 256 MiB per
stream; `_with_config` variants change them
([Performance](@/docs/performance.md#memory-by-entry-point)).

A long-lived trusted stream lifts the total and keeps each window capped:

```rust
use edifact::{ReaderConfig, message_windows_from_reader_with_config};

let config = ReaderConfig::unbounded().max_message_bytes(16 << 20);
let input = std::io::Cursor::new(b"UNH+1+ORDERS:D:96A:UN'BGM+220+1'UNT+3+1'".to_vec());
for window in message_windows_from_reader_with_config(input, config) {
    assert_eq!(window?.segments.len(), 3);
}
# Ok::<(), edifact::EdifactError>(())
```

---

## Segment-level streaming

### `from_reader` — raw segment stream

```rust,no_run
use edifact::from_reader;
use std::fs::File;

fn main() -> Result<(), edifact::EdifactError> {
    let f = File::open("interchange.edi")?;

    for result in from_reader(f) {
        let seg = result?;
        println!("tag={} elements={}", seg.tag, seg.elements.len());
    }
    Ok(())
}
```

Each `OwnedSegment` is dropped before the next is parsed.

---

## Message windows

A **message window** holds the segments from a `UNH` through its `UNT`.
Envelope segments (`UNB`, `UNZ`, `UNG`, `UNE`) are skipped.

### `message_windows` — byte-slice source

```rust
use edifact::message_windows;

let interchange = b"\
    UNB+UNOA:1+S+R+200101:0900+1'\
    UNH+1+ORDERS:D:96A:UN'BGM+220+PO-001+9'UNT+3+1'\
    UNH+2+ORDERS:D:96A:UN'BGM+220+PO-002+9'UNT+3+2'\
    UNZ+2+1'";

for result in message_windows(interchange) {
    let window = result?;
    // message_type: UNH DE 0065; association_code: DE 0057 (Option<Cow<str>>)
    // segments: [UNH, BGM, UNT]
    println!("{:?}: {} segments", window.message_type, window.segments.len());
}
# Ok::<(), edifact::EdifactError>(())
```

### `message_windows_from_reader` — reader source

```rust,no_run
use edifact::message_windows_from_reader;
use std::fs::File;

fn main() -> Result<(), edifact::EdifactError> {
    let f = File::open("multi_message.edi")?;
    for result in message_windows_from_reader(f) {
        let window = result?;
        println!("message type: {:?}", window.message_type);
    }
    Ok(())
}
```

**Errors**: a `UNH` without `UNT` before end of input is `UnexpectedEof`
(`E001`); a second `UNH` before `UNT` is `InvalidSegmentForMessage` (`E011`); a
window over `max_message_bytes` is `E036`; reader I/O errors are `Io`. The
iterator then ends.

---

## Typed streaming — extract matching segments

`deserialize_each` yields one `T` per segment `T` matches and skips the rest:

```rust
use edifact::{EdifactDeserialize, deserialize_each};

#[derive(Debug, EdifactDeserialize)]
#[edifact(segment = "BGM")]
struct Bgm {
    #[edifact(element = 0)]
    doc_code: String,
    #[edifact(element = 1)]
    doc_id: String,
}

let input = b"UNH+1+ORDERS:D:11A:UN'BGM+220+PO-001+9'BGM+231+PO-002+9'UNT+4+1'";

// Stop after the first BGM — nothing past it is parsed.
let first: Bgm = deserialize_each(input).next().transpose()?.expect("a BGM");
assert_eq!(first.doc_id, "PO-001");

// Or drain the iterator for every match.
let all: Vec<Bgm> = deserialize_each(input).collect::<Result<_, _>>()?;
assert_eq!(all.len(), 2);
assert_eq!(all[1].doc_id, "PO-002");
# Ok::<(), edifact::EdifactError>(())
```

### Reader variant

```rust
use edifact::deserialize_each_from_reader;
use std::io::Cursor;

# use edifact::EdifactDeserialize;
# #[derive(Debug, EdifactDeserialize)]
# #[edifact(segment = "BGM")]
# struct Bgm { #[edifact(element = 0)] doc_code: String, #[edifact(element = 1)] doc_id: String }
let input = Cursor::new(b"BGM+220+PO-001+9'BGM+231+PO-002+9'".to_vec());

let all: Vec<Bgm> = deserialize_each_from_reader(input).collect::<Result<_, _>>()?;
assert_eq!(all.len(), 2);
# Ok::<(), edifact::EdifactError>(())
```

---

## Message-level typed streaming

`deserialize_messages_from_reader` deserializes each `UNH`…`UNT` window into a
message struct:

```rust
use edifact::{EdifactDeserialize, deserialize_messages_from_reader};
use std::io::Cursor;

#[derive(Debug, EdifactDeserialize)]
#[edifact(segment = "BGM")]
struct Bgm {
    #[edifact(element = 0)]
    doc_code: String,
    #[edifact(element = 1)]
    doc_id: String,
}

#[derive(Debug, EdifactDeserialize)]
struct OrderMessage {
    bgm: Option<Bgm>,
}

let interchange = Cursor::new(b"\
    UNB+UNOA:1+S+R+200101:0900+1'\
    UNH+1+ORDERS:D:96A:UN'BGM+220+PO-001+9'UNT+3+1'\
    UNH+2+ORDERS:D:96A:UN'BGM+220+PO-002+9'UNT+3+2'\
    UNZ+2+1'".to_vec());

let messages: Vec<OrderMessage> =
    deserialize_messages_from_reader::<OrderMessage, _>(interchange)
        .collect::<Result<_, _>>()?;

assert_eq!(messages.len(), 2);
assert_eq!(messages[0].bgm.as_ref().unwrap().doc_id, "PO-001");
assert_eq!(messages[1].bgm.as_ref().unwrap().doc_id, "PO-002");
# Ok::<(), edifact::EdifactError>(())
```

The window's `OwnedSegment`s go straight to `T::edifact_deserialize`, with no
conversion. Memory is one window plus the `T`.

---

## Progressive (per-window) validation

Validate each message as it arrives:

```rust
use edifact::{
    ProfileRulePack, ValidationContext, ValidationIssue, ValidationSeverity,
    message_windows_from_reader,
};
use std::io::Cursor;

let input = Cursor::new(b"\
    UNH+1+ORDERS:D:11A:UN'BGM+220+PO-001+9'UNT+3+1'\
    UNH+2+ORDERS:D:11A:UN'BGM+220+PO-002+9'UNT+3+2'".to_vec());

let pack = ProfileRulePack::new("ORDERS-PROGRESSIVE")
    .for_message_type("ORDERS")
    .with_rule_fn(|segs, issues| {
        if !segs.iter().any(|s| s.tag == "BGM") {
            issues.push(
                ValidationIssue::new(
                    ValidationSeverity::Error,
                    "every ORDERS message must contain a BGM segment",
                )
                .with_rule_id("ORDERS-P001"),
            );
        }
    });

let ctx = ValidationContext::builder()
    .with_profile_pack(pack)
    .build();

for result in message_windows_from_reader(input) {
    let window = result?;
    let report = ctx.validate(&window.segments);
    if !report.is_valid() {
        for e in report.errors() {
            eprintln!("{}", e.message);
        }
    }
}
# Ok::<(), edifact::EdifactError>(())
```

Full example: [`cookbook_streamed_progressive_validation.rs`](https://github.com/hupe1980/edifact/tree/main/crates/edifact/examples/cookbook_streamed_progressive_validation.rs).

---

## Manual window assembly with `from_reader`

For custom boundaries, group the raw segment stream yourself:

```rust
use edifact::{from_reader, OwnedSegment};
use std::io::Cursor;

let input = Cursor::new(b"\
    UNH+1+ORDERS:D:11A:UN'\
    BGM+220+PO-001+9'\
    UNT+3+1'".to_vec());

let mut current: Vec<OwnedSegment> = Vec::new();
let mut in_message = false;

for result in from_reader(input) {
    let seg = result?;
    match seg.tag() {
        "UNH" => {
            current.clear();
            in_message = true;
        }
        "UNT" => {
            if in_message {
                current.push(seg);
                println!("window complete: {} segments", current.len());
                in_message = false;
            }
            continue;
        }
        _ => {}
    }
    if in_message {
        current.push(seg);
    }
}
# Ok::<(), edifact::EdifactError>(())
```

---

## Decoding a stream without making it eager

A `UNOC`…`UNOK` stream must be read up to its `UNB` before decoding.
`decode_reader` does that up front and returns a `Result`.
`from_reader_decoded` defers it to the first `next()`, so the pipeline stays a
plain `Iterator`:

```rust
use edifact::from_reader_decoded;

let mut raw = b"UNB+UNOC:3+S+R+260101:0900+IC1'NAD+BY+M".to_vec();
raw.push(0xFC); // `ü` in ISO 8859-1
raw.extend_from_slice(b"ller'UNZ+0+IC1'");

// No `?` before the pipeline — construction cannot fail.
let segments: Vec<_> = from_reader_decoded(std::io::Cursor::new(raw))
    .collect::<Result<Vec<_>, _>>()?;
assert_eq!(segments[1].element_str(1), Some("Müller"));
# Ok::<(), edifact::EdifactError>(())
```

An unsupported repertoire (`UNOX`, `KECA`) arrives as the first item, and the
iterator ends:

```rust
# use edifact::{EdifactError, from_reader_decoded};
let raw = b"UNB+UNOX:3+S+R+260101:0900+IC1'UNZ+0+IC1'";
let mut stream = from_reader_decoded(std::io::Cursor::new(&raw[..]));

assert!(matches!(
    stream.next().unwrap(),
    Err(EdifactError::UnsupportedCharset { .. })
));
assert!(stream.next().is_none());
# Ok::<(), edifact::EdifactError>(())
```

`from_bytes_decoded` is the slice counterpart; it returns owned segments
because the decoded buffer belongs to the call. To stay zero-copy, hold the
buffer: `decode_interchange`, then `from_bytes`.

Use the decoding entry points by default: a `UNOC` test corpus stored as UTF-8
parses without them, and the first conformant partner message fails.

---

## Next steps

- [Validation](@/docs/validation.md) — validate windows and messages
- [Async Integration](@/docs/async-integration.md) — bridging to tokio
- [Performance](@/docs/performance.md) — allocation budget, memory bounds and benchmarks
