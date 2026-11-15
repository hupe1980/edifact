# edifact ⚡

[![crates.io](https://img.shields.io/crates/v/edifact.svg)](https://crates.io/crates/edifact)
[![docs.rs](https://docs.rs/edifact/badge.svg)](https://docs.rs/edifact)
[![CI](https://github.com/hupe1980/edifact/actions/workflows/ci.yml/badge.svg)](https://github.com/hupe1980/edifact/actions)
[![license](https://img.shields.io/crates/l/edifact.svg)](#license)
[![MSRV](https://img.shields.io/badge/rust-1.88%2B-blue.svg)](#msrv-and-edition)

**EDIFACT (ISO 9735) for Rust** — zero-copy parsing, typed derive macros,
layered validation, and a writer that cannot forge an envelope. Bounded by
default against hostile input.

📖 **[Guides](https://hupe1980.github.io/edifact/docs/)** ·
🦀 **[API reference](https://docs.rs/edifact)** ·
🔐 **[Threat model](crates/edifact/THREAT_MODEL.md)** ·
📦 **[crates.io](https://crates.io/crates/edifact)**

---

## Install

```bash
cargo add edifact
```

`derive` is on by default. Optional features:

```bash
cargo add edifact --features diagnostics   # miette-powered error rendering
cargo add edifact --features serde         # Serialize/Deserialize on reports
cargo add edifact --features dtm           # DTM date/time codec (UNCL 2379), no dependencies
cargo add edifact --features dtm-time      # dtm plus conversions to the `time` crate
cargo add edifact --no-default-features    # core parse, write and validate only
```

## Quick start

Parsing borrows from the input; a value is copied only where a release escape
was resolved:

```rust
use edifact::{SegmentSliceExt, from_bytes};

let input = b"UNA:+.? 'UNH+1+ORDERS:D:11A:UN'BGM+220+PO-4711+9'NAD+BY+4000001::9'UNT+4+1'";
let segments: Vec<_> = from_bytes(input).collect::<Result<_, _>>()?;

let bgm = segments.first_tag("BGM").expect("BGM");
assert_eq!(bgm.element_str(0), Some("220"));      // document code
assert_eq!(bgm.element_str(1), Some("PO-4711"));  // document number

// Look up by tag and qualifier (element 0).
assert_eq!(segments.qualified_value("NAD", "BY", 1, 0), Some("4000001"));
# Ok::<(), edifact::EdifactError>(())
```

Map segments and whole messages onto structs, dispatching repeated segments by
their qualifier:

```rust
use edifact::{EdifactDeserialize, EdifactSerialize, from_bytes};

#[derive(Debug, EdifactDeserialize, EdifactSerialize)]
#[edifact(segment = "BGM")]
struct Bgm {
    #[edifact(element = 0)]
    doc_code: String,
    #[edifact(element = 1)]
    doc_number: String,
}

#[derive(Debug, EdifactDeserialize, EdifactSerialize)]
#[edifact(segment = "NAD", qualifier_from = 0)]
struct Nad {
    #[edifact(element = 0)]
    qualifier: String,
    #[edifact(element = 1)]
    party_id: Option<String>,
}

#[derive(Debug, EdifactDeserialize)]
struct OrderMessage {
    bgm: Option<Bgm>,
    #[edifact(qualifier = "BY")]
    buyer: Option<Nad>,
    #[edifact(qualifier = "SU")]
    supplier: Option<Nad>,
}

let input = b"UNH+1+ORDERS:D:11A:UN'BGM+220+PO-4711+9'\
              NAD+BY+4000001000002::9'NAD+SU+4000001000001::9'UNT+5+1'";
let segments: Vec<_> = from_bytes(input).collect::<Result<_, _>>()?;
let msg = OrderMessage::edifact_deserialize(&segments)?;

assert_eq!(msg.buyer.unwrap().party_id.as_deref(), Some("4000001000002"));
# Ok::<(), edifact::EdifactError>(())
```

Write it back out with delimiters escaped for you:

```rust
use edifact::to_edifact_string;
# use edifact::EdifactSerialize;
# #[derive(EdifactSerialize)]
# #[edifact(segment = "BGM")]
# struct Bgm { #[edifact(element = 0)] doc_code: String }

let wire = to_edifact_string(&Bgm { doc_code: "220".into() })?;
assert_eq!(wire, "BGM+220'");
# Ok::<(), edifact::EdifactError>(())
```

## What makes it different

Where hand-rolled EDIFACT code usually goes wrong, `edifact` takes a position
(clauses refer to ISO 9735-1:2002):

| | |
|---|---|
| **One segment type, borrowed or owned** | `Segment<'a>` holds `Cow` text: `from_bytes` borrows, `from_reader` owns (`OwnedSegment` = `Segment<'static>`). One API serves both. |
| **Zero-copy by default** | `from_bytes` allocates each segment's element vector once and copies a value only where a release escape was resolved. The [allocation budget](https://hupe1980.github.io/edifact/docs/performance/) is enforced by tests. |
| **Refuses to guess** | The slice and reader paths return the same `Result` for every input, however it is chunked. The decoding paths read the repertoire from `UNB` and refuse an unknown one instead of assuming UTF-8. |
| **Bounded by default** | 64 KiB per segment, 16 MiB per message, 256 MiB per input; a message window holds at most 128 × its bytes. A limit is an error (`E036`), never a quiet end of input. `ReaderConfig::unbounded()` opts out for trusted input. Error text escapes control and bidi characters. |
| **Interchanges are built, not assembled** | One typed `InterchangeHeader` is parsed and written. `InterchangeBuilder` and `Writer` compute `UNZ`; `MessageBuilder` and `MessageWriter` compute `UNT`. No trailing empty element or component is written. |
| **An honest writer** | A message dropped without `finish()` gets no `UNT`; after a failed write the writer is poisoned and every later call returns `E072`. Refusals write nothing. |
| **The envelope is checked against the standard** | Structure, reference pairing and all control counts; every service-segment value against its ISO 9735-10 representation for syntax versions 1–4; S004 as a real date and time; interactive, security and trailing segments refused by name. |
| **`CONTRL` from the report** | The acknowledgement is built from the validation report, so it cannot disagree with what you found. Each finding lands at the lowest level that can locate it and carry its code (ISO 9735-4 Annex A). |
| **Layered validation** | Envelope, syntax, charset, structure, code-list and profile checks write into one report with stable codes, byte spans and rule IDs. |
| **Representations enforced** | `an..35`, `n8`, `a1`: length in characters, not bytes (§6); a numeric length excludes sign, decimal mark and exponent (§10). Version-dependent positions (S004 DE 0017 `n6`/`n8`) are checked per version. |
| **Identifiers, not indices** | Address a field by its data element identifier. The derive resolves it at compile time, so a stale identifier fails the build. `SegmentLayout::audit` checks a hand-written layout against a corpus. |
| **Repetitions survive the round trip** | The repetition separator splits an element into occurrences (§8.7), from `UNA` or the `UNB` syntax version; `#[edifact(repeat)]` maps them onto a `Vec<T>`. |
| **Dates by format code** | Feature `dtm`: decode, compare and build `DTM` values for 17 UNCL 2379 codes; `dtm-time` converts to `time`. |
| **Character sets decoded, not assumed** | `decode_interchange` reads the repertoire from `UNB` S001 and transcodes `UNOA`–`UNOK`/`UNOY`, borrowing when the payload is ASCII. |
| **No `unsafe`** | `#![deny(unsafe_code)]`; property and fuzz tests over parse, write and validate. A [threat model](crates/edifact/THREAT_MODEL.md) lists each threat with its test evidence. |

### Addressing a field by identifier

A transposed positional index reads the wrong data element and still validates
clean. Addressed by identifier, a wrong reference is an error:

```rust
use edifact::{ComponentRef, ElementRef, SegmentDefinition, Status, from_bytes};

static C082: &[ComponentRef] = &[
    ComponentRef::new(1, "3039", Status::Mandatory),
    ComponentRef::new(2, "1131", Status::Conditional),
    ComponentRef::new(3, "3055", Status::Conditional),
];
static NAD_ELEMENTS: &[ElementRef] = &[
    ElementRef::new(1, "3035", Status::Mandatory, 1),
    ElementRef::composite(2, "C082", Status::Conditional, 1, C082),
];
static NAD: SegmentDefinition = SegmentDefinition::new("NAD", "Name and address", NAD_ELEMENTS);

let segs: Vec<_> = from_bytes(b"NAD+BY+4000001000002::9'").collect::<Result<Vec<_>, _>>()?;

assert_eq!(segs[0].value_by_code(&NAD, "3039")?, Some("4000001000002"));
assert_eq!(segs[0].value_by_code(&NAD, "3055")?, Some("9"));
assert!(segs[0].value_by_code(&NAD, "2380").is_err()); // DE 2380 belongs to DTM
# Ok::<(), edifact::EdifactError>(())
```

The derive performs the same resolution at compile time — see
[Typed Derive](https://hupe1980.github.io/edifact/docs/typed-derive/#element-by-identifier).

## Documentation

Guides: **[hupe1980.github.io/edifact](https://hupe1980.github.io/edifact/docs/)**.
Their Rust snippets are doctests: compiled and run, except `no_run` blocks
(file and network I/O), which are compiled only, and one `ignore`d fragment.

| Guide | |
|---|---|
| [Getting Started](https://hupe1980.github.io/edifact/docs/getting-started/) | Install, first parse, feature flags |
| [Core Concepts](https://hupe1980.github.io/edifact/docs/core-concepts/) | Wire format, UNA, release characters, repetitions, Rust type mapping |
| [Character Sets](https://hupe1980.github.io/edifact/docs/character-sets/) | `UNOA`–`UNOK`/`UNOY` decoding, encoding, and repertoire validation |
| [Parsing](https://hupe1980.github.io/edifact/docs/parsing/) | Entry points, byte spans, and the `ReaderConfig` budgets |
| [Writing](https://hupe1980.github.io/edifact/docs/writing/) | `Writer`, interchange and message builders, escaping, custom UNA, repeating elements |
| [Dates and Times](https://hupe1980.github.io/edifact/docs/dates-and-times/) | `DTM` values by UNCL 2379 format code: decode, compare, build, validate, `time` conversions |
| [Typed Derive](https://hupe1980.github.io/edifact/docs/typed-derive/) | Every derive attribute, including identifier-addressed fields |
| [Streaming](https://hupe1980.github.io/edifact/docs/streaming/) | Reader iterators, message windows, typed extraction |
| [Validation](https://hupe1980.github.io/edifact/docs/validation/) | `Validator`, `ValidationContext`, the validation layers (envelope, syntax, charset, structure, code list, profile) |
| [Segment Groups](https://hupe1980.github.io/edifact/docs/segment-groups/) | Group trees, ordered groups, placements, group-scoped rules |
| [CONTRL](https://hupe1980.github.io/edifact/docs/contrl/) | Acknowledgements and rejections from a validation report |
| [Profile Packs](https://hupe1980.github.io/edifact/docs/profile-packs/) | Authoring, composing, and filtering business rules |
| [Diagnostics](https://hupe1980.github.io/edifact/docs/diagnostics/) | `miette` integration |
| [Async Integration](https://hupe1980.github.io/edifact/docs/async-integration/) | Bridging to `tokio` |
| [Error Reference](https://hupe1980.github.io/edifact/docs/error-reference/) | Every stable code: `E001`–`E061`, `E066`–`E072` |
| [Performance](https://hupe1980.github.io/edifact/docs/performance/) | Allocation budget, memory bounds, benchmarks, tuning |

The [threat model](crates/edifact/THREAT_MODEL.md) covers hostile input,
structure injection on write and false acknowledgements, each with status and
test evidence. Report vulnerabilities as described in [SECURITY.md](SECURITY.md).

Runnable cookbooks: [`crates/edifact/examples/`](crates/edifact/examples), e.g.
`cargo run --example cookbook_parse_map_validate_write`.

## Scope

`edifact` is the **engine**, not a directory distribution.

It ships the parser, writer, validation pipeline, the table types for segment
definitions, and the **batch service segments** of ISO 9735 as layouts in
`edifact::service`: `UNB`, `UNG`, `UNH`, `UNT`, `UNE`, `UNZ`, `UNS`, `UNO`,
`UNP`, `UGH`, `UGT`, and the `CONTRL` segments `UCI`, `UCM`, `UCF`, `UCS`,
`UCD`. The syntax standard fixes them, so there is no version to pick:

```rust
use edifact::{from_bytes, service};

let segments: Vec<_> = from_bytes(b"UNB+UNOC:3+SENDER+RECEIVER+260101:0900+IC4711'UNZ+0+IC4711'")
    .collect::<Result<Vec<_>, _>>()?;

// DE 0020 by name, not by counting to element 4.
assert_eq!(segments[0].value_by_code(&service::UNB, "0020")?, Some("IC4711"));
# Ok::<(), edifact::EdifactError>(())
```

An interchange is built from a typed header, with the counts computed:

```rust
use edifact::{Charset, InterchangeBuilder, InterchangeHeader, MessageBuilder, MessageIdentifier,
              Party, PreparationDateTime, SyntaxVersion};

let header = InterchangeHeader::new(Charset::UnoC, SyntaxVersion::V3,
    Party::new("SENDER"), Party::new("RECEIVER"), PreparationDateTime::new(2026, 1, 1, 9, 0)?, "IC1");
let mut msg = MessageBuilder::new(MessageIdentifier::new("1", "ORDERS", "D", "96A", "UN"))?;
msg.write_simple("BGM", &["220", "PO-1", "9"])?;
let bytes = InterchangeBuilder::new(header).message_segments(msg.finish()?).build()?;
assert_eq!(
    bytes,
    b"UNB+UNOC:3+SENDER+RECEIVER+260101:0900+IC1'\
      UNH+1+ORDERS:D:96A:UN'BGM+220+PO-1+9'UNT+3+1'UNZ+1+IC1'"
        .to_vec(),
);
# Ok::<(), edifact::EdifactError>(())
```

It also ships **`CONTRL`** (ISO 9735-4), generated from a validation report:

```rust
use edifact::{Contrl, from_bytes, validate_envelope};

let raw = b"UNB+UNOC:3+SENDER+RECEIVER+260101:0900+IC4711'\
            UNH+MSG1+ORDERS:D:96A:UN'BGM+220+PO-1+9'UNT+3+MSG1'\
            UNZ+1+IC4711'";
let segments: Vec<_> = from_bytes(raw).collect::<Result<Vec<_>, _>>()?;
let validated = validate_envelope(&segments)?;

let wire = Contrl::acknowledgement(&validated)
    .to_interchange_string("UNOC", "3", "260101", "0930", "ACK-1")?;

// Its own interchange, addressed back the way it came.
assert!(wire.starts_with("UNB+UNOC:3+RECEIVER+SENDER+260101:0930+ACK-1'"));
assert!(wire.contains("UCI+IC4711+SENDER+RECEIVER+7'"));
# Ok::<(), edifact::EdifactError>(())
```

It does **not** ship UN/EDIFACT *directory* data (`BGM`, `DTM`, `NAD`, `C507`, …):
the [UN UNTDID licence](https://service.unece.org/trade/untdid/license.htm)
forbids distributing modified copies without UNECE's written permission, and
this crate reads derived tables as such. Supply the subset you use as `static`
tables or load it with `DirectoryValidatorBuilder`, then check it with
[`SegmentLayout::audit`](https://hupe1980.github.io/edifact/docs/validation/#auditing-a-hand-written-layout).

Not covered: interactive EDI (ISO 9735-3), the security segments and messages
(parts 5, 6, 7, 9). Packages (§7.10) are recognised, not parsed: the `UNO`
header gives the object's length and the bytes are yours.

## Development

The [`justfile`](justfile) mirrors CI, so a green `just ci` means a green build:

```bash
just check          # fmt + clippy + tests — the pre-commit gate
just test           # workspace tests, all features
just site-serve     # preview the documentation site with live reload
just ci             # everything CI runs, on this toolchain
just release-check  # package both crates (README + licences), versions, semver
just semver-install # the cargo-semver-checks release CI pins
just semver-report  # advisory: every break against the last release
```

A breaking change to `edifact`'s public API (cargo-semver-checks against the
last release) needs the next minor version and a CHANGELOG migration note.
`edifact-derive` is covered by its UI suite.

## MSRV and edition

Rust **1.88**, edition **2024**, checked by CI. Raising the MSRV is a breaking
change.

## License

Dual-licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this work shall be dual-licensed as above, without any
additional terms or conditions.
