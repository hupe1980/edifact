+++
title = "Writing"
description = "Serialize segments back to the EDIFACT wire format with Writer, automatic delimiter escaping, custom UNA service strings, and repeating data elements."
weight = 40
+++

Produce EDIFACT from typed structs, segments or runtime values; build messages
and interchanges with computed counts.

---

## Overview of write APIs

| API | Best for |
|---|---|
| `to_edifact_string(value)` | Quick serialization of a single derived struct |
| `segments_to_bytes(segments)` | Round-trip a parsed `Vec<Segment<'_>>` |
| `Writer::write_segment(seg)` | Round-tripping a parsed or hand-built `Segment` |
| `Writer::write_elements(tag, elements)` | **The general form** — segments mixing simple and composite data elements |
| `Writer::write_composites(tag, elements)` | Every element is a list of components; borrowed *or* owned data |
| `Writer::write_simple(tag, elements)` | Every element is one value — the commonest shape |
| `Writer::with_service_string_advice(w, ssa)` | Custom delimiters, **no** `UNA` emitted |
| `Writer::with_una(w, ssa)` | Custom delimiters, `UNA` written first |
| `InterchangeBuilder::new(header)` | A complete interchange as bytes: `UNB`, messages, computed `UNZ` |
| `Writer::begin_interchange(&header)` | The same, streamed: `UNZ` counted by the writer |
| `MessageBuilder::new(id)` | One message as `Vec<OwnedSegment>`, `UNT` computed |

Component boundaries are always **explicit**: a value containing a delimiter is
release-escaped, never split.

Trailing empty data elements and components are never written (ISO 9735-1
§8.7); an empty value between present ones keeps its separator:

```rust
use edifact::Writer;

let mut w = Writer::new(Vec::new());
w.write_simple("BGM", &["220", "", "9", ""])?;
w.write_composites("DTM", &[&["137", "20260101", ""][..]])?;
assert_eq!(w.finish()?, b"BGM+220++9'DTM+137:20260101'".to_vec());
# Ok::<(), edifact::EdifactError>(())
```

## What the writer will not write

Everything the writer emits reparses. These calls are refused before a byte
reaches the sink; the writer stays usable:

| Refused | Error |
|---|---|
| A tag that is not three ASCII uppercase letters | `InvalidSegmentTag` |
| A repeating data element with no repetition separator declared | `RepetitionSeparatorNotDeclared` (`E037`) |
| A header whose repertoire differs from the bound `Charset`'s | `CharacterRepertoireMismatch` |
| A raw `UNB` or `UNZ` inside an interchange opened by `begin_interchange` | `InvalidEventSequence` |
| An invalid header, a second `begin_interchange`, an `end_interchange` with nothing to close | see [Writing a full interchange](#writing-a-full-interchange) |

A tag cannot be escaped, so `bgm`, `BGMX` or `B+M` are refused:

```rust
use edifact::{EdifactError, Writer};

let mut writer = Writer::new(Vec::new());
let err = writer.write_simple("bgm", &["220"]).unwrap_err();
assert!(matches!(err, EdifactError::InvalidSegmentTag(_)));
```

Any other error — a failed write, a value the bound `Charset` cannot encode —
can leave a partial segment in the sink, so it **poisons** the writer (see
[Message guard and poisoning](#message-guard-and-poisoning)).

---

## Serialize a typed struct

```rust
use edifact::{EdifactSerialize, ser, to_edifact_string};

#[derive(edifact::EdifactSerialize)]
#[edifact(segment = "BGM")]
struct Bgm {
    #[edifact(element = 0)]
    doc_code: String,
    #[edifact(element = 1)]
    doc_number: String,
    #[edifact(element = 2)]
    function_code: Option<String>,
}

let bgm = Bgm {
    doc_code: "220".into(),
    doc_number: "PO-4711".into(),
    function_code: Some("9".into()),
};

let output = to_edifact_string(&bgm)?;
assert_eq!(output, "BGM+220+PO-4711+9'");

// or as bytes:
let bytes = ser::to_bytes(&bgm)?;
# Ok::<(), edifact::EdifactError>(())
```

A `None` field is an empty element; at the end of the segment it is not written:

```rust
# use edifact::to_edifact_string;
# #[derive(edifact::EdifactSerialize)]
# #[edifact(segment = "BGM")]
# struct Bgm {
#     #[edifact(element = 0)]
#     doc_code: String,
#     #[edifact(element = 1)]
#     doc_number: String,
#     #[edifact(element = 2)]
#     function_code: Option<String>,
# }
# 
# let bgm = Bgm {
#     doc_code: "220".into(),
#     doc_number: "PO-4711".into(),
#     function_code: Some("9".into()),
# };
let bgm_no_func = Bgm {
    doc_code: "220".into(),
    doc_number: "PO-4711".into(),
    function_code: None,
};
let out = to_edifact_string(&bgm_no_func)?;
assert_eq!(out, "BGM+220+PO-4711'");
# Ok::<(), edifact::EdifactError>(())
```

---

## Round-trip: parse then write

```rust
use edifact::{from_bytes, segments_to_bytes};

let input = b"UNA:+.? 'BGM+220+PO-4711+9'NAD+BY+4000001::9'";
let segs: Vec<_> = from_bytes(input).collect::<Result<_, _>>()?;

let output = segments_to_bytes(&segs)?;
assert_eq!(output, b"BGM+220+PO-4711+9'NAD+BY+4000001::9'");
# Ok::<(), edifact::EdifactError>(())
```

`segments_to_bytes` writes default delimiters and no `UNA`; use
`Writer::with_una` to keep a custom service string.

---

## Streaming writer

`Writer<W>` writes one segment at a time to any `Write` implementation:

```rust
use edifact::{Writer, Segment, Element};

let mut buf: Vec<u8> = Vec::new();
let mut writer = Writer::new(&mut buf);

writer.write_segment(&Segment::new("BGM", vec![
    Element::of(&["220"]),
    Element::of(&["PO-4711"]),
    Element::of(&["9"]),
]))?;

writer.write_segment(&Segment::new("NAD", vec![
    Element::of(&["BY"]),
    Element::of(&["4000001", "", "9"]), // composite element
]))?;

writer.finish()?;

let text = String::from_utf8(buf).unwrap();
assert_eq!(text, "BGM+220+PO-4711+9'NAD+BY+4000001::9'");
# Ok::<(), edifact::EdifactError>(())
```

`Writer::finish()` flushes the sink and returns it.

### Messages: `UNT` is computed

`Writer::begin_message` writes `UNH` and returns a `MessageWriter`; its
`finish()` writes `UNT` with the message's segment count. `MessageBuilder` does
the same into owned segments, with nothing written:

```rust
use edifact::{MessageBuilder, MessageIdentifier, Writer};

let id = MessageIdentifier::new("1", "UTILMD", "D", "11A", "UN").with_association_code("S2.1");

let mut w = Writer::new(Vec::new());
let mut msg = w.begin_message(id)?;
msg.write_simple("BGM", &["E01", "DOC-1"])?;
msg.finish()?;

let mut built = MessageBuilder::new(id)?;
built.write_simple("BGM", &["E01", "DOC-1"])?;
let segments = built.finish()?; // Vec<OwnedSegment>: UNH, BGM, UNT+3+1

assert_eq!(edifact::segments_to_bytes(&segments)?, w.finish()?);
# Ok::<(), edifact::EdifactError>(())
```

`MessageBuilder` refuses what `Writer` refuses (`E006`, `E037`, `E038` under
`with_charset`) before appending; `finish()` refuses a message without a body
(`E043`).

---

## `write_elements` — mixed simple and composite elements

`NAD` mixes a simple qualifier with a composite party identification;
`write_elements` takes both shapes:

```rust
use edifact::{DataElement, Writer};

let mut buf: Vec<u8> = Vec::new();
let mut writer = Writer::new(&mut buf);

writer.write_elements("NAD", &[
    DataElement::Simple("MS"),
    DataElement::Composite(&["9900112233445", "", "293"]),
])?;
writer.write_elements("DTM", &[
    DataElement::Composite(&["137", "20260101", "102"]),
])?;

writer.finish()?;
assert_eq!(
    String::from_utf8(buf).unwrap(),
    "NAD+MS+9900112233445::293'DTM+137:20260101:102'",
);
# Ok::<(), edifact::EdifactError>(())
```

The `elements!` macro is shorthand. Each entry borrows through `AsDataElement`:
a string is a simple element, an array, slice or `Vec` of strings a composite.
Runtime values work like literals:

```rust
use edifact::{Writer, elements};

let qualifier = String::from("MS");
let gln = "9900112233445";

let mut buf: Vec<u8> = Vec::new();
let mut writer = Writer::new(&mut buf);

writer.write_elements("NAD", elements![qualifier.as_str(), [gln, "", "293"]])?;
writer.write_elements("DTM", elements![["137", "20260101", "102"]])?;

writer.finish()?;
assert_eq!(
    String::from_utf8(buf).unwrap(),
    "NAD+MS+9900112233445::293'DTM+137:20260101:102'",
);
# Ok::<(), edifact::EdifactError>(())
```

> Composite components must be string *slices*: a `[String; N]` cannot borrow as
> `&[&str]` without allocating, so write `[id.as_str(), "", agency]`.

A literal component separator inside a value is escaped:

```rust
# use edifact::{Writer, elements};
# let mut buf: Vec<u8> = Vec::new();
# let mut writer = Writer::new(&mut buf);
writer.write_elements("NAD", elements!["MS", "ACME:INC"])?;
writer.finish()?;
// The `:` stays inside the value:
assert_eq!(String::from_utf8(buf).unwrap(), "NAD+MS+ACME?:INC'");
# Ok::<(), edifact::EdifactError>(())
```

`MessageWriter` carries the same four methods (`write_simple`,
`write_composites`, `write_elements`, `write_segment`); each segment counts
into `UNT` DE 0074.

---

## `write_simple` and `write_composites` — runtime string data

Both avoid building `Segment` values. `write_simple` takes one value per data
element; a component separator inside a value stays part of it.

```rust
use edifact::Writer;

let mut buf: Vec<u8> = Vec::new();
let mut writer = Writer::new(&mut buf);

writer.write_simple("BGM", &["220", "PO-4711", "9"])?;
writer.write_simple("FTX", &["AAA", "ACME:INC"])?;   // the `:` is escaped, not split

writer.finish()?;
assert_eq!(buf, b"BGM+220+PO-4711+9'FTX+AAA+ACME?:INC'".to_vec());
# Ok::<(), edifact::EdifactError>(())
```

`write_composites` takes explicit components, borrowed or owned —
`&[&[&str]]`, `&[Vec<String>]`, `&[[String; 3]]`:

```rust
use edifact::Writer;

let mut buf: Vec<u8> = Vec::new();
let mut writer = Writer::new(&mut buf);

// Borrowed literals …
writer.write_composites("DTM", &[&["137", "20240101", "102"][..]])?;
// … and owned values built at runtime.
let rff = vec![vec!["ON".to_string(), "PO-4711".to_string()]];
writer.write_composites("RFF", &rff)?;

writer.finish()?;
assert_eq!(buf, b"DTM+137:20240101:102'RFF+ON:PO-4711'".to_vec());
# Ok::<(), edifact::EdifactError>(())
```

---

## Custom delimiters and UNA

`Writer::with_una` writes a `UNA` header and then uses those delimiters:

```rust
use edifact::{Writer, ServiceStringAdvice};

let ssa = ServiceStringAdvice {
    component_sep: b';',
    element_sep:   b'|',
    decimal_mark:  b'.',
    release_char:  b'?',
    repetition_sep: b' ',   // 0x20 = 'not used'
    segment_term:  b'!',
};

let mut buf: Vec<u8> = Vec::new();
let mut writer = Writer::with_una(&mut buf, ssa)?;

// One `&str` per data element; the custom element separator `|` is applied.
writer.write_simple("BGM", &["220", "PO-4711", "9"])?;
writer.finish()?;

let text = String::from_utf8(buf).unwrap();
assert_eq!(text, "UNA;|.? !BGM|220|PO-4711|9!");
# Ok::<(), edifact::EdifactError>(())
```

### Delimiters without a `UNA`

A version 4 interchange declares `*` through `UNB` S001 DE 0002, not a `UNA`.
Declare the delimiters to the writer without emitting one:

```rust
use edifact::{ServiceStringAdvice, Writer, from_bytes};

let input = b"UNB+UNOC:4+S+R+260101:0900+I'RFF+ON:1*ON:2'UNZ+0+I'";
let segments: Vec<_> = from_bytes(input).collect::<Result<Vec<_>, _>>()?;

let ssa = ServiceStringAdvice::for_syntax_version(Some(4));
let mut writer = Writer::with_service_string_advice(Vec::new(), ssa)?;
for segment in &segments {
    writer.write_segment(segment)?;
}
// Byte-for-byte the input, with no UNA invented along the way.
assert_eq!(writer.finish()?, input.to_vec());
# Ok::<(), edifact::EdifactError>(())
```

---

## Repeating data elements

Repetitions (ISO 9735-1 §8.7) need a `ServiceStringAdvice` that declares a
repetition separator:

```rust
use edifact::{Element, Segment, ServiceStringAdvice, Writer};

let ssa = ServiceStringAdvice { repetition_sep: b'*', ..Default::default() };
let mut buf: Vec<u8> = Vec::new();
{
    let mut writer = Writer::with_una(&mut buf, ssa)?;
    let rff = Segment::new(
        "RFF",
        vec![Element::of(&["ON", "1"]).and_repeat(&["ON", "2"])],
    );
    writer.write_segment(&rff)?;
}
assert!(String::from_utf8(buf).unwrap().ends_with("RFF+ON:1*ON:2'"));
# Ok::<(), edifact::EdifactError>(())
```

Without one, the segment is refused with `RepetitionSeparatorNotDeclared`
(`E037`): nothing is written and the writer stays usable.

### Through the event layer

`EdifactEvent::RepeatElement` writes a further occurrence:

```rust
use edifact::{EdifactEvent, EventEmitter, ServiceStringAdvice, WriterEmitter};

let ssa = ServiceStringAdvice::from_bytes(b"UNA:+.?*'")?;
let mut emitter = WriterEmitter::with_una(Vec::new(), ssa)?;
emitter.emit(EdifactEvent::start("RFF"))?;
emitter.emit(EdifactEvent::element("ON"))?;
emitter.emit(EdifactEvent::component("1"))?;
emitter.emit(EdifactEvent::repeat("ON"))?;  // second occurrence
emitter.emit(EdifactEvent::component("2"))?;
emitter.emit(EdifactEvent::EndSegment)?;

assert_eq!(emitter.finish()?, b"UNA:+.?*'RFF+ON:1*ON:2'".to_vec());
# Ok::<(), edifact::EdifactError>(())
```

The same `E037` refusal applies.

---

## Character repertoires

`Writer::with_charset` binds the writer to the repertoire `UNB` S001 names: a
`UNOC` interchange goes out as ISO 8859-1. A character the repertoire cannot
carry is `CharacterNotInRepertoire` (`E038`); a header declaring another
repertoire is `CharacterRepertoireMismatch` (`E041`).

```rust
use edifact::{Charset, Writer};

let mut writer = Writer::new(Vec::new()).with_charset(Charset::UnoC);
writer.write_composites("NAD", &[&["BY"], &["Müller"]])?;
// `ü` goes out as the single ISO 8859-1 byte 0xFC.
assert_eq!(writer.finish()?, b"NAD+BY+M\xFCller'".to_vec());
# Ok::<(), edifact::EdifactError>(())
```

`WriterEmitter::with_charset` binds the typed path the same way:

```rust
use edifact::{Charset, EdifactEvent, EventEmitter, WriterEmitter};

let mut emitter = WriterEmitter::new(Vec::new()).with_charset(Charset::UnoC);
emitter.emit(EdifactEvent::start("NAD"))?;
emitter.emit(EdifactEvent::element("Müller"))?;
emitter.emit(EdifactEvent::EndSegment)?;

// `ü` goes out as the single Latin-1 byte 0xFC, not as two UTF-8 bytes.
assert_eq!(emitter.finish()?, b"NAD+M\xFCller'".to_vec());
# Ok::<(), edifact::EdifactError>(())
```

See [Character Sets](@/docs/character-sets.md) for the repertoires, the read
side and repertoire validation.

---

## Escape handling

`Writer` escapes every active delimiter inside a value; never pre-escape:

```rust
use edifact::{Writer, Segment, Element};

let mut buf: Vec<u8> = Vec::new();
let mut writer = Writer::new(&mut buf);

// Both `:` and `+` are delimiters — the writer escapes each of them.
writer.write_segment(&Segment::new("FTX", vec![
    Element::of(&["AAI"]),
    Element::of(&["Price: 100+VAT"]),  // ':' → '?:'  and  '+' → '?+'
]))?;
writer.finish()?;

let text = String::from_utf8(buf).unwrap();
assert_eq!(text, "FTX+AAI+Price?: 100?+VAT'");
# Ok::<(), edifact::EdifactError>(())
```

Escaped: the segment terminator, element and component separators, the release
character, and the repetition separator when one is active. A space "not used"
repetition slot is never escaped.

---

## Event-based writing (`WriterEmitter`)

`WriterEmitter` streams `EdifactSerialize` output to a `Write` sink:

```rust
use edifact::{ser, WriterEmitter, EdifactSerialize, Writer};

# #[derive(EdifactSerialize)]
# #[edifact(segment = "BGM")]
# struct Bgm { #[edifact(element = 0)] code: String }
let bgm = Bgm { code: "220".into() };

let mut buf: Vec<u8> = Vec::new();
// Takes the `io::Write` sink itself, not a `Writer`.
let mut emitter = WriterEmitter::new(&mut buf);

bgm.edifact_serialize(&mut emitter)?;

let _sink = emitter.finish()?;
# Ok::<(), edifact::EdifactError>(())
```

It allocates nothing per event and keeps no buffer; wrap a file or socket in
`BufWriter`.

---

## Writing a full interchange

The `UNB` is an `InterchangeHeader`. `InterchangeBuilder` takes it, the
messages — owned segments or `UNH`…`UNT` text — and returns bytes that pass
`validate_envelope`, or an error and no bytes:

```rust
use edifact::{Charset, InterchangeBuilder, InterchangeHeader, MessageBuilder, MessageIdentifier,
              Party, PreparationDateTime, SyntaxVersion};

let header = InterchangeHeader::new(Charset::UnoC, SyntaxVersion::V3,
    Party::new("SENDER").with_qualifier("14"), Party::new("RECEIVER").with_qualifier("14"),
    PreparationDateTime::new(2026, 8, 2, 9, 15)?, "REF001");
let mut msg = MessageBuilder::new(MessageIdentifier::new("1", "ORDERS", "D", "96A", "UN"))?;
msg.write_simple("BGM", &["220", "PO-1", "9"])?;
let bytes = InterchangeBuilder::new(header).message_segments(msg.finish()?).build()?;
assert!(bytes.starts_with(b"UNB+UNOC:3+SENDER:14+RECEIVER:14+260802:0915+REF001'"));
assert!(bytes.ends_with(b"UNZ+1+REF001'"));
# Ok::<(), edifact::EdifactError>(())
```

Versions 1–3 write the date as `YYMMDD`, version 4 as `CCYYMMDD`. A `UNA` is
written exactly when `with_service_string_advice` differs from the version's
defaults. No message is `E042`; a header value too long for the version is its
representation finding (`E049` for a 15-character control reference).

The streaming form: `begin_interchange` validates the header, binds an unbound
writer to its repertoire and writes `UNB`; the writer counts messages (or raw
`UNG`s) and `end_interchange()` writes `UNZ`:

```rust
use edifact::{Charset, InterchangeHeader, MessageIdentifier, Party, PreparationDateTime,
              SyntaxVersion, Writer};

let header = InterchangeHeader::new(Charset::UnoA, SyntaxVersion::V3,
    Party::new("SENDER"), Party::new("RECEIVER"),
    PreparationDateTime::new(2020, 1, 1, 9, 0)?, "IC1");
let mut w = Writer::new(Vec::new());
w.begin_interchange(&header)?;
let mut msg = w.begin_message(MessageIdentifier::new("1", "ORDERS", "D", "96A", "UN"))?;
msg.write_simple("BGM", &["220", "PO-4711", "9"])?;
msg.write_elements("NAD", edifact::elements!["BY", ["4000001", "", "9"]])?;
msg.finish()?;
w.end_interchange()?;
assert_eq!(
    w.finish()?,
    b"UNB+UNOA:3+SENDER+RECEIVER+200101:0900+IC1'\
      UNH+1+ORDERS:D:96A:UN'BGM+220+PO-4711+9'NAD+BY+4000001::9'UNT+4+1'\
      UNZ+1+IC1'"
        .to_vec(),
);
# Ok::<(), edifact::EdifactError>(())
```

| Refused before the first byte | Error |
|---|---|
| `begin_interchange` while one is open; `end_interchange` with none open or inside a message | `E024` |
| A header value missing, of the wrong class or length for its version | `E008`, `E021`, `E048`–`E050` |
| A two-digit year under version 4 | `E060` |
| A writer bound to another repertoire | `E041` |
| Versions 1–3 with an active repetition separator | `E007` |
| No message; grouped and ungrouped messages mixed; a count too long for DE 0036 | `E042`, `E052`, `E049` |
| A raw `UNE` whose DE 0060 is not its group's message count | `E054` |

### Wrapping message text

`InterchangeBuilder::message` takes one rendered `UNH`…`UNT` as UTF-8 text in
the interchange's service characters, with a body, a consistent `UNT` and an
unused reference. Surrounding ASCII whitespace is dropped; anything else is
refused. The text is encoded strictly into the header's repertoire:

```rust
use edifact::{Charset, EdifactError, InterchangeBuilder, InterchangeHeader, Party,
              PreparationDateTime, SyntaxVersion};

let header = InterchangeHeader::new(Charset::UnoC, SyntaxVersion::V3,
    Party::new("SENDER"), Party::new("RECEIVER"), PreparationDateTime::new(2026, 1, 1, 9, 0)?, "IC1");

let ok = InterchangeBuilder::new(header.clone())
    .message("UNH+1+ORDERS:D:96A:UN'FTX+AAA+Müller'UNT+3+1'\n")
    .build()?;
assert!(ok.windows(2).any(|w| w == b"M\xFC")); // `ü` as one Latin-1 byte
assert!(!ok.contains(&b'\n'));

let text = "UNH+1+ORDERS:D:96A:UN'FTX+AAA+a—b'UNT+3+1'";
let err = InterchangeBuilder::new(header).message(text).build().unwrap_err();
assert!(matches!(
    err,
    EdifactError::CharacterNotInRepertoire { character: '—', offset: 31, .. }
));
# Ok::<(), EdifactError>(())
```

### Message guard and poisoning

An error path never produces output that looks complete:

- Dropping a `MessageWriter` without a successful `finish()` — an early `?`, a
  panic — writes **nothing** and poisons the writer. The output keeps a `UNH`
  without `UNT`, which no envelope check accepts.
- A failed write, or an error after a call wrote bytes (such as
  `CharacterNotInRepertoire` mid-value), poisons the writer too.
- A poisoned writer returns `WriterPoisoned` from every write, `begin_*`,
  `end_interchange`, `finish` and `finish_unt`, and writes nothing more.
  `is_poisoned()` reports the state; `into_inner()` returns the sink with the
  partial output, unflushed.

```rust
use edifact::{EdifactError, MessageIdentifier, Writer};

fn map_order(w: &mut Writer<Vec<u8>>, fail: bool) -> Result<(), EdifactError> {
    let mut msg = w.begin_message(MessageIdentifier::new("1", "ORDERS", "D", "96A", "UN"))?;
    msg.write_simple("BGM", &["220", "PO-4711", "9"])?;
    if fail {
        // The caller's own error: the guard is dropped unfinished.
        return Err(EdifactError::InvalidUtf8);
    }
    msg.finish()
}

let mut w = Writer::new(Vec::new());
assert!(map_order(&mut w, true).is_err());

assert!(w.is_poisoned());
assert!(matches!(
    w.end_interchange(),
    Err(EdifactError::WriterPoisoned { .. })
));
// No `UNT`, no `UNZ`: the partial interchange cannot pass for a complete one.
let partial = w.into_inner();
assert!(partial.ends_with(b"BGM+220+PO-4711+9'"));
# Ok::<(), EdifactError>(())
```

`WriterEmitter` follows the same rules per event, and its `finish()` returns
`InvalidEventSequence` while a segment is open; `WriterEmitter::into_inner`
returns the sink.

---

## Next steps

- [Typed Derive](@/docs/typed-derive.md) — derive `EdifactSerialize` for your structs
- [Parsing](@/docs/parsing.md) — parse EDIFACT input to `Segment` slices
- [Performance](@/docs/performance.md) — allocation budget and benchmarks
