+++
title = "Core Concepts"
description = "The EDIFACT wire format — segments, data elements, components, the UNA service string advice, release characters, and repetition — mapped onto the Rust types."
weight = 20
+++

The EDIFACT wire format and the Rust types that represent it.

---

## What is EDIFACT?

**EDIFACT** (Electronic Data Interchange For Administration, Commerce and
Transport) is the UN standard for business messages, defined in ISO 9735.

An **interchange** is flat text: **segments** end with a terminator (usually
`'`), **data elements** are separated by `+`, **components** by `:`.

---

## Wire format anatomy

```text
UNA:+.? '
UNB+UNOA:3+SENDER:14+RECEIVER:14+260101:0900+IC4711'
UNH+MSG1+ORDERS:D:96A:UN+MYREF'
BGM+220+PO-4711+9'
NAD+BY+4000001000002::9'
NAD+SU+4000001000001::9'
UNT+5+MSG1'
UNZ+1+IC4711'
```

| Part | Purpose |
|---|---|
| `UNA:+.? '` | Service string advice — defines the 6 special characters |
| `UNB` | Interchange header (sender, receiver, date, reference) |
| `UNH` | Message header — starts one functional message |
| Body segments | Payload (`BGM`, `NAD`, `LIN`, `MOA`, …) |
| `UNT` | Message trailer — declares segment count and reference |
| `UNZ` | Interchange trailer — declares message count and reference |

One interchange may hold many `UNH`…`UNT` messages.

---

## The UNA service string advice

The `UNA` segment is always exactly **9 bytes**: the literal `UNA` followed by six
service characters in fixed positions:

```text
U N A : + . ?   '
      │ │ │ │ │ └── Segment terminator   (default: ' )
      │ │ │ │ └──── Repetition separator (`*`, or space = "not used")
      │ │ │ └────── Release character    (default: ? )
      │ │ └──────── Decimal mark         (ignored on receipt)
      │ └────────── Element separator    (default: + )
      └──────────── Component separator  (default: : )
```

ISO 9735-1 numbers the positions `010`–`060`; the byte column counts from the
start of the input.

| UNA byte | Position | Purpose | Default | Splits input? |
|---|---|---|---|---|
| 3 | 010 | Component data element separator | `:` | yes |
| 4 | 020 | Data element separator | `+` | yes |
| 5 | 030 | Decimal mark | `.` | no — see below |
| 6 | 040 | Release (escape) character | `?` | escapes the next byte |
| 7 | 050 | Repetition separator | `*` | yes, **when not a space** |
| 8 | 060 | Segment terminator | `'` | yes |

The parser reads a leading `UNA` and applies its delimiters to everything
after it.

**The decimal mark is ignored.** ISO 9735-1 Annex B says the recipient ignores
position 030; §10 allows `.` or `,` per numeric value. It is the one position
where a space is legal, that need not be distinct, and that `edifact` does not
validate. The value is kept so a writer can round-trip the `UNA` and
`DecimalFloat` can format with it.

**The repetition separator depends on the syntax version.** Version 4 defines
it with default `*`; versions 1–3 have none and put a space there. Without a
`UNA`, `edifact` reads the version from `UNB` S001 DE 0002 and activates `*`
only for version 4 — in version 3, `*` is ordinary level A data. See
[Repeating data elements](#repeating-data-elements).

A fragment with neither `UNA` nor `UNB` records no delimiters, so supply them:

```rust
use edifact::{ReaderConfig, ServiceStringAdvice, from_bytes_with_config};

let ssa = ServiceStringAdvice::from_bytes(b"UNA:;.? ~")?;
let config = ReaderConfig::default().with_service_string_advice(ssa);

let segments: Vec<_> = from_bytes_with_config(b"BGM;220;PO-4711~", config)
    .collect::<Result<Vec<_>, _>>()?;
assert_eq!(segments[0].element_str(1), Some("PO-4711"));
# Ok::<(), edifact::EdifactError>(())
```

A malformed `UNA` — too short, a duplicated active delimiter, an alphanumeric
delimiter — is an error, never a fallback to defaults.

---

## Segment structure

A segment has:

1. **Tag** — exactly 3 uppercase ASCII letters (e.g. `BGM`, `NAD`, `UNH`)
2. **Elements** — separated by the element separator (`+`)
3. **Components** — within an element, separated by the component separator (`:`)
4. **Terminator** — marks the end of the segment (`'`)

Example breakdown:

```text
BGM  +  220  +  PO-4711  +  9  '
 │       │         │        │
 tag   elem 0   elem 1   elem 2 (all single-component)

NAD+BY+4000001000002::9'
 │   │        │         │
 │   │        │         └── component 2 of element 1
 │   │        └──────────── component 0 of element 1
 │   └───────────────────── element 0 (one component)
 └───────────────────────── tag
```

`::` omits component 1 and keeps the agency qualifier at component 2 (ISO
9735-1 §8.8.3).

Element 0 of `NAD` (`BY`) is the **qualifier**. The derive's
`qualifier_from = 0` dispatches `NAD+BY` and `NAD+SU` to different structs.

---

## Release characters

The **release character** (`?` by default) escapes the next byte, allowing delimiters
to appear as literal text:

```text
BGM+Test?+value'
         ^^
         '?+' means a literal '+', not an element separator
```

The parser resolves release sequences: you read `"Test+value"`.

A `?` at the end of input is `E019 InvalidReleaseSequence`.

---

## Rust type mapping

### One segment type, borrowed or owned

`edifact` has a **single** segment type. `Segment<'a>` holds its text as
`Cow<'a, str>`, which covers both parsing modes:

```text
pub struct Segment<'a> {
    pub tag: Cow<'a, str>,            // borrowed from the input, or owned
    pub span: Span,                   // byte range of the whole segment
    pub tag_span: Span,               // byte range of just the tag
    pub elements: Vec<Element<'a>>,
}
```

- `from_bytes` borrows straight out of the input buffer and yields
  `Segment<'input>` — text is borrowed; the element vector is the only
  allocation ([budget](@/docs/performance.md#allocation-per-segment)).
- `from_reader` has no buffer to borrow from, so it yields `Segment<'static>`,
  which is aliased as **`OwnedSegment`**.

`Segment` is covariant in `'a`, so a `&[OwnedSegment]` is accepted wherever a
`&[Segment<'_>]` is. Every function takes that one shape:

```rust
use edifact::{OwnedSegment, Segment};

fn count_bgm(segments: &[Segment<'_>]) -> usize {
    segments.iter().filter(|s| s.tag == "BGM").count()
}

let borrowed: Vec<Segment<'_>> =
    edifact::from_bytes(b"BGM+220'").collect::<Result<_, _>>()?;
let owned: Vec<OwnedSegment> =
    edifact::from_reader(std::io::Cursor::new(b"BGM+220'")).collect::<Result<_, _>>()?;

assert_eq!(count_bgm(&borrowed), 1);
assert_eq!(count_bgm(&owned), 1);   // same function, no conversion
# Ok::<(), edifact::EdifactError>(())
```

To keep a segment past the buffer it was parsed from, call
`Segment::into_owned()`.

`segment.tag` compares directly against a string literal (`segment.tag == "BGM"`);
`segment.tag()` is the `&str` for the places that need one, such as a `match`.

### `Element<'a>` — component holder

```text
pub struct Element<'a> {
    pub span: Span,
    pub components: Components<'a>,       // (value, span) — inline for ≤4 components
    pub repeats: Vec<Components<'a>>,     // further occurrences; usually empty
}
```

`Cow::Borrowed` is used when the component contains no release sequences.
`Cow::Owned` is used only when an escape was resolved (the decoded string differs
from the raw bytes) or when the segment came from a reader.

### Reading values

Every accessor lives on `Segment`, so it is available on both parsing paths:

| Method | Returns |
|---|---|
| `element_str(n)` | component 0 of element `n` |
| `component_str(elem, comp)` | one specific component |
| `get_element(n)` | the whole `Element` |
| `optional_element(n)` / `optional_component(e, c)` | as above, but empty counts as absent |
| `required_element(n)` / `required_component(e, c)` | `Err` when absent or empty |
| `parsed_element::<T>(n)` | parsed into `T`, `Err` when absent or unparseable |
| `repeated_component(elem, comp)` | one component across every occurrence |
| `value_by_code(&layout, "3055")` | addressed by data-element identifier |

### Repeating data elements

ISO 9735-1 §8.7 lets one data element occur several times in a single slot,
separated by the repetition separator — UNA position 050, or `*` by default in a
syntax version 4 interchange:

```text
RFF+ON:1*ON:2*ON:3'
    └──┬─┘ └──┬─┘ └──┬─┘
       0      1      2     ← three occurrences of element 0
```

`components` always holds occurrence 0, so every positional accessor —
`element_str`, `component_str`, `value_by_code`, and the derive macros — keeps
reading the first occurrence and behaves identically on the interchanges that do
not use the feature. The remaining occurrences live in `repeats`:

```rust
// `UNA` byte 7 declares `*` as the repetition separator.
let segments: Vec<_> = edifact::from_bytes(b"UNA:+.?*'RFF+ON:1*ON:2'")
    .collect::<Result<Vec<_>, _>>()?;
let rff = segments[0].get_element(0).unwrap();

assert_eq!(rff.repeat_count(), 2);
assert_eq!(rff.get_component(1), Some("1"));                  // occurrence 0
assert_eq!(rff.repetition(1).unwrap()[1].0.as_ref(), "2");    // occurrence 1

// Or read one component across every occurrence at once:
let all: Vec<&str> = segments[0].repeated_component(0, 1).collect();
assert_eq!(all, ["1", "2"]);
# Ok::<(), edifact::EdifactError>(())
```

Without an active separator the byte is ordinary data: `RFF+ON:1*ON:2'` in a
syntax version 3 interchange yields the single component `1*ON`. A value that
legitimately contains the separator is release-escaped by the writer and
unescaped on the way back in, so `a?*b` round-trips as `a*b`.

### `Span` — byte position

```rust
pub struct Span { pub start: usize, pub end: usize }
```

Every `Segment`, `Element`, and component carries a `Span` into the original input.
Diagnostics use these spans to show precise error locations.

---

## Envelope vs. body segments

EDIFACT distinguishes **envelope** segments from **body** segments:

| Segment | Role |
|---|---|
| `UNA` | Service string advice (optional, always first) |
| `UNB` | Interchange header — mandatory outer wrapper |
| `UNZ` | Interchange trailer |
| `UNG` | Functional group header (optional) |
| `UNE` | Functional group trailer (optional) |
| `UNH` | Message header — begins a logical message |
| `UNT` | Message trailer |
| Everything else | Message body (`BGM`, `NAD`, `LIN`, …) |

Envelope validation checks reference parity (`UNB`↔`UNZ`, `UNH`↔`UNT`),
every service value against its ISO 9735-10 representation for syntax
versions 1–4, and control tags in body positions. Interactive (`UI…`),
security (`US…`) and other `U` tags are errors (`E055`–`E057`); anything after
`UNZ` is `E058`. See [Validation](@/docs/validation.md).

---

## Message types

The `UNH` segment element 1, component 0 carries the **message type** (`ORDERS`, `INVOIC`, …). `ValidationContext` and `ProfileRulePack` scope rules
to it:

```text
UNH + 1 + ORDERS : D : 96A : UN '
             ^^^   ^   ^^^   ^^
             type  dir rel   org
```

| Component | Meaning |
|---|---|
| 0 | Message type (`ORDERS`, `INVOIC`, …) |
| 1 | Message version number (`D` = draft) |
| 2 | Message release number (`96A`, `11A`, …) |
| 3 | Controlling agency (`UN`) |

---

## Character set

The parser works on **UTF-8** and rejects invalid sequences with `E003
InvalidText`. UTF-8 is not a superset of `UNOC` (ISO 8859-1, where `ü` is the
single byte `0xFC`), so decode by the repertoire `UNB` S001 names first:

```rust
use edifact::{decode_interchange, from_bytes};

let mut raw = b"UNB+UNOC:3+S+R+260101:0900+IC1'NAD+BY+M".to_vec();
raw.push(0xFC);
raw.extend_from_slice(b"ller'UNZ+0+IC1'");

let utf8 = decode_interchange(&raw)?;                     // reads UNB S001
let segments: Vec<_> = from_bytes(&utf8).collect::<Result<Vec<_>, _>>()?;
assert_eq!(segments[1].element_str(1), Some("Müller"));
# Ok::<(), edifact::EdifactError>(())
```

`decode_interchange` copies nothing for ASCII or `UNOY` payloads. See
[Character Sets](@/docs/character-sets.md).

---

## Further reading

- [Parsing guide](@/docs/parsing.md) — `from_bytes`, `from_reader`, reader config
- [Typed Derive guide](@/docs/typed-derive.md) — mapping segments to Rust structs
- [Error Reference](@/docs/error-reference.md) — all error codes and their meanings
