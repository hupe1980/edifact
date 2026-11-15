+++
title = "Error Reference"
description = "Every EdifactError variant with its stable code E001-E061 and E066-E072, the fields it carries, when it fires, and how to fix it."
weight = 110
+++

Every error is an `EdifactError` variant with a stable, semver-protected code
(`E001`–`E061`, `E066`–`E072`) from `err.stable_code()`. The enum is
`#[non_exhaustive]`.

## Where an error points

- **`offset: usize`** — one byte position, on the lexical variants: the fault
  is a point in the stream.
- **`span: Span`** — a half-open byte range, on variants raised while
  validating a parsed segment. A `ValidationIssue` built from one keeps the
  range, so `miette` and editors underline the whole value.

`ValidationIssue` has one positional field, `span`; `issue.start_offset()` gives
its start.

---

## Quick lookup table

| Code | Variant | Source | Span field |
|---|---|---|---|
| E001 | `UnexpectedEof` | Parser | `offset` |
| E002 | `InvalidDelimiter` | Parser | `offset` |
| E003 | `InvalidText` | Parser | `offset` |
| E004 | `MessageCountMismatch` | Envelope validator | `span` |
| E005 | `SegmentCountMismatch` | Envelope validator | `span` |
| E006 | `InvalidSegmentTag` | Parser | — |
| E007 | `InvalidUna` | Parser | — |
| E008 | `MissingRequiredElement` | Deserializer, `required_element`, envelope validator, `InterchangeHeader` | — |
| E009 | `InvalidUtf8` | Writer | — |
| E010 | `Io` | Reader / Writer | — |
| E011 | `InvalidSegmentForMessage` | Directory validator | `span` |
| E012 | `InvalidElementCount` | Directory validator | `span` |
| E013 | `InvalidComponentCount` | Directory validator | `span` |
| E014 | `InvalidCodeValue` | Directory validator | `span` |
| E015 | `MissingSegment` | Directory validator | — |
| E016 | `QualifierMismatch` | Typed deserializer | `span` |
| E017 | `ConditionalRequirementNotMet` | Profile validator | `span` |
| E019 | `InvalidReleaseSequence` | Parser | `offset` |
| E020 | `SegmentTooLong` | Reader parser | `offset` |
| E021 | `MissingRequiredComponent` | Deserializer, `required_component`, envelope validator, `InterchangeHeader` | — |
| E023 | `InterchangeTooLarge` | Envelope builder | — |
| E024 | `InvalidEventSequence` | Event emitter, `Writer` interchange envelope | — |
| E025 | `InvalidElementPosition` | Directory builder | — |
| E026 | `IncompatibleReleaseScopes` | Profile pack composer | — |
| E027 | `InvalidFieldValue` | Typed deserializer, `DtmValidator` | — |
| E028 | `UnexpectedDataToken` | Parser | `offset` |
| E030 | `ValidationErrors` | Profile / directory validator | — |
| E031 | `UnrecognisedSyntaxIdentifier` | Envelope validator | — |
| E032 | `DuplicateReference` | Envelope validator | `span` |
| E033 | `UnknownDataElement` | Code-addressed access | — |
| E034 | `AmbiguousDataElement` | Code-addressed access | — |
| E035 | `SegmentLayoutMismatch` | Code-addressed access | — |
| E036 | `LimitExceeded` | Parser (`ReaderConfig` budgets), repertoire sniff, header read | `offset` |
| E037 | `RepetitionSeparatorNotDeclared` | Writer | — |
| E038 | `CharacterNotInRepertoire` | Writer / `Charset::encode` | — |
| E039 | `UnsupportedCharset` | `Charset::from_syntax_identifier` | — |
| E040 | `NonFiniteNumber` | `DecimalFloat` serialization | — |
| E041 | `CharacterRepertoireMismatch` | `Writer::begin_interchange` | — |
| E042 | `EmptyInterchange` | Envelope validator, `Writer::end_interchange`, `InterchangeBuilder` | — |
| E043 | `EmptyMessage` | Envelope validator, `MessageBuilder`, `InterchangeBuilder` | `span` |
| E044 | `PackageNotSupported` | Envelope validator | `span` |
| E045 | `BlankDataElementValue` | Syntax validator | `span` |
| E046 | `SegmentWithoutDataElements` | Syntax validator | `span` |
| E047 | `TooManyRepetitions` | Directory validator | `span` |
| E048 | `InvalidCharacterType` | Envelope / directory validator | `span` |
| E049 | `DataElementTooLong` | Envelope / directory validator | `span` |
| E050 | `DataElementTooShort` | Envelope / directory validator | `span` |
| E051 | `TrailingSeparator` | Syntax validator | `span` |
| E052 | `GroupsAndMessagesMixed` | Envelope validator, `Writer::end_interchange` | `span` |
| E053 | `InsignificantCharacters` | Directory validator | `span` |
| E054 | `GroupCountMismatch` | Envelope validator | `span` |
| E055 | `InteractiveSegmentNotSupported` | Envelope validator | `span` |
| E056 | `SecuritySegmentNotSupported` | Envelope validator | `span` |
| E057 | `ReservedServiceSegmentTag` | Envelope validator | `span` |
| E058 | `DataAfterInterchangeTrailer` | Parser, envelope validator | `span` |
| E059 | `UnsupportedSyntaxVersion` | Envelope validator | `span` |
| E060 | `InvalidDateTime` | Envelope validator | `span` |
| E061 | `EmptyGroup` | Envelope validator | `span` |
| E066 | `ActionNotPermitted` | `Contrl` constructors with `ContrlOptions` | — |
| E067 | `InvalidContrlIdentifier` | `ContrlIdentifier` | — |
| E068 | `SegmentNotPlaced` | `GroupResolution::findings`, `PlacementValidator` | `span` |
| E069 | `TooManySegmentOccurrences` | `GroupResolution::findings`, `PlacementValidator` | `span` |
| E070 | `TooManyGroupOccurrences` | `GroupResolution::findings`, `PlacementValidator` | `span` |
| E071 | `InvalidGroupSchema` | `GroupDefBuilder::build`, `GroupSchema::{new, ordered}` | — |
| E072 | `WriterPoisoned` | `Writer`, `MessageWriter`, `WriterEmitter` | — |

---

## Variant details

### E001 — `UnexpectedEof`

```text
unexpected end of input at byte offset {offset}
```

**When**: The input ended inside a segment (no segment terminator) or inside a
leading `UNA`, which ISO 9735-1 Annex A has followed by six service characters.
Reported only when nothing earlier among the bytes present is a fault: `BGM+220`,
`UNT+3+1'U` (a tag prefix of one or two uppercase letters is not a fault yet) and
`UNA'` are `E001`, while `UNH+\xFC` is `E003`.

**Fields**: `offset: usize` — the input length.

**Fix**: Terminate every segment; check the payload was not truncated.

---

### E002 — `InvalidDelimiter`

```text
invalid delimiter byte 0x{byte:02X} at offset {offset}
```

**When**: A byte stands where it cannot: a separator or terminator where a segment
tag is required, or a non-delimiter byte inside a segment. A segment terminator
directly after another one (`''`), after only whitespace, or at the head of the
input is an empty segment — ISO 9735-1 §7.6 starts every segment with its tag —
and reported here with the offset of that terminator.

**Fields**: `byte: u8`, `offset: usize` — absolute input offset.

**Fix**: Check the UNA service string advice and the delimiter bytes in the payload;
remove doubled segment terminators.

---

### E003 — `InvalidText`

```text
invalid EDIFACT text at byte offset {offset}
```

**When**: A segment or element contains a non-UTF-8 byte sequence.

**Fields**: `offset: usize`.

**Fix**: Decode the interchange first. `UNOC`–`UNOK` are single-byte ISO 8859
repertoires whose high bytes are not valid UTF-8; `decode_interchange` reads the
repertoire from `UNB` S001 and converts, copying nothing when the payload is
already ASCII or `UNOY`. See [Character Sets](@/docs/character-sets.md).

Also raised by every decoding entry point (`Charset::decode`,
`decoding_reader`, `decode_reader`, `from_bytes_decoded`,
`from_reader_decoded`) for a byte in a slot the repertoire leaves undefined;
`offset` is the byte's offset in the raw input.

---

### E004 — `MessageCountMismatch`

```text
UNZ DE 0036 declares 5 unit(s), the interchange holds 1
```

**When**: `UNZ` DE 0036 does not match the number of messages — or of groups,
when the interchange uses them. A group's own count is `E054`. A count that is
not a number is `E048` and is not compared.

**Fields**: `expected: u32`, `actual: u32`, `span: Span` (the `UNZ`).

**Fix**: Regenerate the `UNZ` segment with the correct count, or check for missing /
extra `UNH..UNT` pairs. Reported by `CONTRL` as code 29 on `UCI`.

---

### E005 — `SegmentCountMismatch`

```text
segment count mismatch in message {message_ref}: UNT declared {expected}, found {actual}
```

**When**: The `UNT` segment's element 1 (segment count, inclusive of `UNH`/`UNT`)
does not match the actual count.

**Fields**: `expected: u64`, `actual: u64`, `message_ref: String`, `span: Span`
(the `UNT`). A count of up to 10 digits (syntax version 4) is reported exactly.

**Fix**: Recount segments and update `UNT` element 1.

---

### E006 — `InvalidSegmentTag`

```text
invalid segment tag {0:?}
```

**When**: A segment tag that is not exactly 3 ASCII uppercase letters is encountered.

**Fields**: `String` — the bad tag text.

**Fix**: Segment tags must match `[A-Z]{3}`. Check for leading/trailing whitespace or
lowercase letters.

---

### E007 — `InvalidUna`

```text
invalid UNA service string advice
```

**When**: The six service characters after `UNA` are not usable: the active ones
(component and element separator, release character, segment terminator, and a
repetition separator other than space) must be mutually distinct, printable,
non-alphanumeric ASCII. The decimal mark is not checked. Fewer than six
characters is `E001`.

**Fix**: Correct the `UNA`; the default is `UNA:+.? '`.

---

### E008 — `MissingRequiredElement`

```text
missing required element {element_index} in segment {tag}
```

**When**: A mandatory element is absent or empty: in the typed deserializer
(`#[derive(EdifactDeserialize)]`), `Segment::required_element`, a service
segment checked by envelope validation (`UNB+UNOA:3++R…`), or
`InterchangeHeader::parse`.

**Fields**: `tag: String`, `element_index: usize`.

**Fix**: Add the missing element to the segment, or mark the field `Option<T>` if it
is truly optional.

---

### E009 — `InvalidUtf8`

```text
serialized output contains invalid UTF-8
```

**When**: The serializer's internal check found non-UTF-8 output. This is a bug;
please report it.

---

### E010 — `Io`

```text
(transparent — wraps std::io::Error)
```

**When**: Any I/O error from `std::io::Read` / `std::io::Write`.

**Fields**: wraps `IoError(std::io::Error)`.

**Fix**: Check the underlying source or sink.

---

### E011 — `InvalidSegmentForMessage`

```text
segment {tag} is not valid for message type {message_type}
```

**When**: Directory validation found a segment that is not permitted in the current
message type.

**Fields**: `tag: String`, `message_type: String`, `span: Span`.

**Fix**: Remove the unsupported segment or switch to the correct message type.

---

### E012 — `InvalidElementCount`

```text
segment {tag} has {actual} elements, expected between {min} and {max}
```

**When**: Directory validation found an element count outside the allowed `[min, max]`
range.

**Fields**: `tag`, `min`, `max`, `actual`, `span: Span`.

**Fix**: Adjust the element count to within the directory-defined bounds.

---

### E013 — `InvalidComponentCount`

```text
segment {tag} element {element_index} has {actual} components, expected {expected}
```

**When**: Either of two checks in `DirectoryValidator`:

1. An `expected_components` hook is registered for the element and the observed
   count differs — the hook is an **exact** count.
2. No hook applies, but the element is a composite whose components the directory
   declares (`ElementRef::composite` / `OwnedElementRef::with_components`), and
   the segment supplies **more** components than declared. Supplying fewer stays
   valid: conditional components may be omitted, and trailing empty components
   are stripped first per ISO 9735-1 §8.8.3.

**Fields**: `tag`, `element_index`, `expected: u8` (the exact or maximum count),
`actual: u8`, `span: Span`.

**Fix**: Fix the composite element arity to match the directory definition.

---

### E014 — `InvalidCodeValue`

```text
segment {tag} element {element_index}: '{value}' is not a valid code (code list {code_list})
```

**When**: A field that should hold a code-list value contains an unrecognised code.

**Fields**: `tag`, `element_index`, `value`, `code_list`, `span: Span`, `suggestion: Option<&'static str>`.

**Fix**: Use a valid code from the referenced code list. If `suggestion` is present,
it will contain a remediation hint.

---

### E015 — `MissingSegment`

```text
required segment {tag} is missing from message (position {expected_position})
```

**When**: Structural validation determined a mandatory segment is absent.

**Fields**: `tag: String`, `expected_position: String` (human-readable position hint).

**Fix**: Add the missing segment at the indicated position.

---

### E016 — `QualifierMismatch`

```text
segment {tag} has qualifier '{actual}', expected '{expected}'
```

**When**: A qualified-segment mapping (e.g. `NAD+BY`) found a qualifier that does
not match the expected value.

**Fields**: `tag`, `actual`, `expected`, `span: Span`.

**Fix**: Correct the qualifier or use the right qualified segment type.

---

### E017 — `ConditionalRequirementNotMet`

```text
segment {tag} element {element_index}: conditional requirement not met ({condition})
```

**When**: An element that is required by a conditional rule (e.g. "required when
element 0 is `BY`") is absent.

**Fields**: `tag`, `element_index`, `condition: String`, `span: Span`.

**Fix**: Provide the conditionally required element, or remove the element that
triggered the condition.

---

### E019 — `InvalidReleaseSequence`

```text
invalid release sequence at byte offset {offset}: dangling release character
```

**When**: A release character (`?` by default) appears at the end of input with no
following byte to escape.

**Fields**: `offset: usize`.

**Fix**: Remove the trailing release character, or add the character that should be
escaped.

---

### E020 — `SegmentTooLong`

```text
segment starting at byte offset {offset} exceeded maximum length of {limit} bytes
```

**When**: A segment is longer than `ReaderConfig::max_segment_bytes`, measured from
the first byte of its tag to the byte before its terminator (whitespace between
segments does not count). Detected once `max_segment_bytes + 1` bytes have been
read, unless an earlier byte of the segment is already a fault. A guard against
adversarially crafted or truncated input.

**Fields**: `offset: usize` — the tag's absolute start, `limit: usize`.

**Fix**: Increase `ReaderConfig::max_segment_bytes` if the segment is legitimately
large, or investigate why the terminator is missing.

---

### E021 — `MissingRequiredComponent`

```text
missing required component {component_index} in element {element_index} of segment {tag}
```

**When**: A composite is present but lacks a mandatory component: in the typed
deserializer, `Segment::required_component`, a service segment checked by
envelope validation (`UNB` without DE 0002), or `InterchangeHeader::parse`.

**Fields**: `tag: String`, `element_index: usize`, `component_index: usize`.

**Fix**: Provide the missing component inside the composite element, or mark the
field `Option<T>` if it is truly optional.

---

### E023 — `InterchangeTooLarge`

```text
interchange too large: count {count} exceeds u32::MAX
```

**When**: An interchange being built has more than 4 294 967 295 items —
corrupted or synthetic input.

**Fields**: `count: u64`.

**Fix**: Check the input; split very large interchanges.

---

### E024 — `InvalidEventSequence`

```text
invalid event sequence: {message}
```

**When**: An `EventEmitter` received events in an invalid order — for example an
`Element` event before `StartSegment`, or a `ComponentElement` event before
`Element` — or `WriterEmitter::finish` was called with a segment still open.
The refused event writes nothing.

The `Writer` raises it for its interchange envelope too: `begin_interchange`
while an interchange is open, `end_interchange` with none open or inside a
message, and a raw `UNB` or `UNZ` written while an interchange opened by
`begin_interchange` is open — the writer owns that `UNZ` count.

**Fields**: `message: &'static str` describing the violation.

**Fix**: Emit `StartSegment` before `Element`, and `Element` before
`ComponentElement`; open and close an interchange with `begin_interchange` and
`end_interchange()` only. Always a bug in the calling code.

---

### E025 — `InvalidElementPosition`

```text
element definition contains invalid position 0; positions must be >= 1 (one-based)
```

**When**: `OwnedElementRef::try_new` received `position = 0`. Positions are
one-based.

**Fix**: Pass a position ≥ 1. `OwnedElementRef::new_unchecked` panics on 0
instead of returning this error.

---

### E026 — `IncompatibleReleaseScopes`

```text
incompatible release scopes: cannot compose {current:?} with {incoming:?}
```

**When**: Two `ProfileRulePack` values with different release scopes were
composed with `extend_from` or `merge_with_override`. Both must share the
release scope, or at most one may carry one.

**Fields**: `current: String` (scope on the receiving pack), `incoming: String`
(scope on the pack being merged in).

**Fix**: Ensure both packs target the same release with
`ProfileRulePack::for_release`, or remove the scope from one pack before
composing.

---

### E027 — `InvalidFieldValue`

```text
segment {tag} element {element_index}: invalid field value "{value}"
```

**When**: The typed deserializer found a qualifier element that was present but
held an empty or otherwise invalid value. `DtmValidator` reports a `DTM` value
that contradicts its format code under this code (see
[Dates and times](@/docs/dates-and-times.md#validating)).

**Fields**: `tag: String`, `element_index: usize`, `value: String`.

**Fix**: Check that the qualifier element in segment `tag` at position
`element_index` contains a recognised non-empty value.

---

### E028 — `UnexpectedDataToken`

```text
unexpected data token at byte offset {offset}: data element before segment tag
```

**When**: A data or component separator came before the first segment tag — a
partial write, a missing tag, or corruption.

**Fields**: `offset: usize`.

**Fix**: Verify the input starts with a valid segment tag (three uppercase
ASCII letters) and that no data or component separators appear before it.

---

### E030 — `ValidationErrors`

```text
validation failed with {error_count} error(s)
```

**When**: Built by the caller to carry a `ValidationReport` with errors as an
`EdifactError`. `ValidationReport::result` returns
`Result<ValidationReport, ValidationReport>` and does not produce it.

**Fields**: `error_count: usize`, `report: Box<ValidationReport>`.

**Fix**: Inspect `report` for the issues, their locations and rule IDs.

---

### E031 — `UnrecognisedSyntaxIdentifier`

```text
unrecognised syntax identifier 'XXXX': expected UNOA-UNOK or UNOY
```

**When**: `UNB` S001 DE 0001 names a repertoire this crate does not recognise.
It recognises `UNOA`–`UNOK` and `UNOY`; `UNOX` and `KECA` are refused with
[E039](#e039-unsupportedcharset).

**Fields**: `0: String` — the offending identifier.

**Fix**: Declare the repertoire the payload is actually written in.

---

### E032 — `DuplicateReference`

```text
duplicate {tag} reference '{reference}' at bytes {span}
```

**When**: Two messages in one interchange share a `UNH` reference (DE 0062), or
two groups a `UNG` reference (DE 0048). ISO 9735-1 requires both to be unique:
a receiver keying on the reference would process one and drop the rest.

**Fields**: `tag: String` (`UNH` or `UNG`), `reference: String`, `span: Span`.

**Fix**: Assign a distinct control reference to every message and group.

---

### E033 — `UnknownDataElement`

```text
segment {tag} has no data element {data_element} in its definition
```

**When**: A code-addressed accessor — `Segment::value_by_code`,
`span_by_code`, `element_by_code`, or `SegmentLayout::resolve_code` — was given a
UN/EDIFACT data element identifier that the supplied `SegmentDefinition` /
`OwnedSegmentDef` does not declare.

A stale or mistyped identifier fails here instead of reading whatever sits at
the wrong index. The derive form (`#[edifact(element = "3055")]` under
`#[edifact(layout = ...)]`) catches it at compile time.

**Fields**: `tag: String`, `data_element: String`.

**Fix**: Check the identifier against the directory definition for that segment.

---

### E034 — `AmbiguousDataElement`

```text
segment {tag} defines data element {data_element} at more than one position
```

**When**: The identifier resolves to more than one position in the definition,
so code-addressed access cannot pick one.

**Fields**: `tag: String`, `data_element: String`.

**Fix**: Address the element positionally (`element_str` / `component_str`), or
split the definition so the identifier is unique.

---

### E035 — `SegmentLayoutMismatch`

```text
segment layout is for {expected}, but the segment is {actual}
```

**When**: A `SegmentLayout` was applied to a segment with a different tag — for
example the `NAD` definition on a `DTM` segment. Rejected before any lookup.

**Fields**: `expected: String` (the layout's tag), `actual: String`.

**Fix**: Look the definition up by the segment's own tag.

---

### E036 — `LimitExceeded`

```text
input exceeded the {limit} limit of {max} at byte offset {offset}
```

**When**: The input carries more than a `ReaderConfig` budget allows —
`max_segments` or `max_messages` for the whole input, `max_message_bytes`
for any one `UNH`…`UNT` message, or `max_input_bytes` — or the recovering scan
recorded more faults than `RecoveryConfig::max_faults` allows, or — on the
decoding entry points — the `UNB` syntax identifier lies outside the 4 KiB sniff
window ([Character Sets](@/docs/character-sets.md#byte-order-mark-and-leading-whitespace);
when `max_input_bytes` ends before the identifier, that limit is reported),
or `InterchangeHeader::read`/`read_from` found no `UNB` terminator within
4 096 + 9 + `max_segment_bytes` bytes (`limit: "header_read"`).
The per-segment size guard has its own code (`E020 SegmentTooLong`).
Without any configuration it fires at `max_input_bytes` = 268 435 456 (256 MiB)
and `max_message_bytes` = 16 777 216 (16 MiB).

`max_input_bytes` counts **every consumed byte**: a byte order mark, the `UNA`,
whitespace as well as segments. Input is consumed
in units (the prologue, a run of whitespace, one segment
with its terminator), and a unit that ends past the budget is refused before it
is parsed — so trailing whitespace beyond the budget is an `E036` too, and a
syntax error that lies beyond the budget is reported as `E036`, not as the
syntax error. When a segment is both too long and crosses the byte budget, the
one detected first in the byte stream wins. The slice and reader paths report
the same `E036` at the same offset for the same input.

Input that ends *exactly* at a limit is not a violation. Why a limit is an error
rather than a quiet stop: [Parsing](@/docs/parsing.md#limits-are-errors).

**Fields** (the variant is `#[non_exhaustive]`; match it with `..`):
`limit: &'static str` (`"max_segments"`, `"max_messages"`,
`"max_message_bytes"`, `"max_input_bytes"`, `"max_faults"`, `"sniff_window"`
or `"header_read"`),
`max: u64`, and `offset: usize` — for a count or per-message budget the start of the segment
that would have exceeded it, for `max_input_bytes` the budget itself (the first
byte outside it).

```rust
use edifact::{EdifactError, ReaderConfig, from_bytes_with_config};

let config = ReaderConfig::default().max_input_bytes(8);
let err = from_bytes_with_config(b"BGM+220'   \n", config)
    .collect::<Result<Vec<_>, _>>()
    .unwrap_err();
assert!(matches!(
    err,
    EdifactError::LimitExceeded { limit: "max_input_bytes", max: 8, offset: 8, .. }
));
```

**Fix**: Raise the corresponding `ReaderConfig` budget if the input is legitimate,
or reject it as oversized; `ReaderConfig::unbounded()` lifts every limit for
trusted input. Segments parsed before the violation are still delivered, so a
manual iteration can keep the partial result. On a long-lived stream use
`ReaderConfig::unbounded().max_message_bytes(n)`: each message stays capped,
the stream does not.

---

### E037 — `RepetitionSeparatorNotDeclared`

```text
cannot write a repeating data element: the active service string advice
declares no repetition separator
```

**When**: A `Segment` carrying an `Element` with repetitions (ISO 9735-1 §8.7) was
handed to a `Writer` whose `ServiceStringAdvice` holds the space "not used"
sentinel at UNA position 7.

**Why**: There is no byte to put between the occurrences; a space would read
back as one occurrence containing a space.

**Fix**: Build the writer with `Writer::with_una` and a `ServiceStringAdvice`
whose `repetition_sep` is set, or flatten the repetitions.

**Recovery**: A refusal: nothing is written and the writer stays usable.

---

### E038 — `CharacterNotInRepertoire`

```text
character 'ü' at offset 1 is not in the UNOA character repertoire
```

**When**: A writer bound with [`Writer::with_charset`](@/docs/writing.md#character-repertoires)
was handed a value containing a character the declared repertoire cannot carry —
or `Charset::encode` was called with one. Such a character has no byte in the
table the receiver decodes with.

**Fix**: Transliterate the value (`ü` → `ue`), or declare a wider repertoire —
`UNOC` for Latin-1, `UNOY` for full UTF-8.

**Recovery**: The check runs while encoding, so part of the segment may already
be written and the writer is poisoned (`E072`). To skip bad records and
continue, check values with `Charset::first_violation` before writing.

---

### E039 — `UnsupportedCharset`

```text
character repertoire 'UNOX' is not supported
```

**When**: An interchange declares `UNOX` (ISO/IEC 2022 code extension) or `KECA`
(a Korean identifier outside ISO 9735) in `UNB` S001 DE 0001.

**Why**: Delimiter scanning is sound only for ASCII-transparent repertoires.
`UNOX` is stateful and `KECA` multi-byte: a `+` byte inside a multi-byte
sequence would be read as an element separator.

**Fix**: Ask the partner for `UNOC` or `UNOY`, or transcode the interchange with
a dedicated codec before handing it to `edifact`.

---

### E040 — `NonFiniteNumber`

```text
non-finite number NaN has no EDIFACT representation
```

**When**: `DecimalFloat(f64::NAN)` or `DecimalFloat(f32::INFINITY)` was
serialized.

**Why**: An EDIFACT number is digits with an optional sign and decimal mark
(ISO 9735-1 §10); `NaN` and `inf` are not numbers to any receiver.

**Fix**: Check the calculation, or omit the element rather than emitting a
placeholder.

---

### E041 — `CharacterRepertoireMismatch`

```text
UNB declares repertoire UNOA, but the writer encodes UNOC
```

**When**: `Writer::begin_interchange` was called with a header whose repertoire
differs from the one the writer was bound to with `Writer::with_charset`.

**Why**: The receiver would decode the body with the wrong table.

**Fix**: Declare the writer's own repertoire in the header, or leave the writer
unbound: `begin_interchange` binds it to the header's.

---

### E042 — `EmptyInterchange`

```text
interchange IC4711 contains no message or group
```

**When**: A `UNB`/`UNZ` pair encloses nothing. ISO 9735-1 §7.2 requires an
interchange to "contain at least one group, or one message or one package".

**Why**: `UNZ+0` agrees with the absent content, so no count check sees it.

**Fields**: `control_ref: String` — `UNB` DE 0020.

`Writer::end_interchange` and `InterchangeBuilder::build` refuse to write one.

**Fix**: Send nothing rather than an empty envelope.

---

### E043 — `EmptyMessage`

```text
message MSG1 has no segments between UNH and UNT
```

**When**: A `UNH`/`UNT` pair encloses nothing. ISO 9735-1 §7.4 requires a message
to "contain at least one additional segment".

**Why**: `UNT+2` is consistent, so the segment count does not catch it.
`MessageBuilder::finish` and `InterchangeBuilder::build` refuse to produce one.

**Fields**: `message_ref: String` (`UNH` DE 0062), `span: Span` of the `UNH`.

**CONTRL**: reported on the message's `UCM` as `13` "missing" — the body
segments are what is missing. ISO 9735-4 Annex A permits `32` "lower level
empty" only on `UCI`/`UCF`.

**Fix**: Omit the message entirely if it has no content.

---

### E044 — `PackageNotSupported`

```text
segment UNO opens or closes a package, which this crate does not parse
```

**When**: The interchange carries a package — `UNO`…`UNP` (ISO 9735-1 §7.10,
elaborated by ISO 9735-8).

**Why**: A package holds binary data of the length declared in `UNO` S022
DE 0810; it is not EDIFACT-encoded and cannot be tokenized.

**Fields**: `tag: String` (`UNO` or `UNP`), `span: Span`.

**Fix**: Split the object out of the byte stream using the declared length, then
parse the remaining segments. `service::UNO` and `service::UNP` ship as layouts
so the header and trailer themselves can be read by data element identifier.

---

### E045 — `BlankDataElementValue`

```text
segment FTX element 1 component 0: value is only spaces
```

**When**: A data element value consists of nothing but spaces. ISO 9735-1 §9.4:
"A data element value containing only space(s) shall not be allowed."

**Why**: Trailing spaces must be suppressed (§9.2), so an all-space value is an
element that should have been omitted. A **warning**: the value is readable.

**Fields**: `tag: String`, `element_index: usize`, `component_index: usize`,
`span: Span`.

**Fix**: Omit the element instead of padding it. Emitted by
[`SyntaxValidator`](@/docs/validation.md#syntax-validation).

---

### E046 — `SegmentWithoutDataElements`

```text
segment DTM contains no data element
```

**When**: A segment carries nothing but its tag. ISO 9735-1 §7.6: "A segment
shall contain at least one data element in addition to the segment tag."

**Why**: §8.6 adds that a conditional segment whose only content is the tag
"shall be omitted in its entirety" — so `DTM'` is either a mandatory segment that
lost its data or a conditional one that should not have been sent. `DTM+'` is
*not* this error: an empty data element is present, which is how
EDIFACT spells a mandatory segment with no data to carry (§8.5).

**Fields**: `tag: String`, `span: Span`.

**Fix**: Supply the data, or drop the segment. Emitted by
[`SyntaxValidator`](@/docs/validation.md#syntax-validation).

---

### E047 — `TooManyRepetitions`

```text
segment RFF element 0 occurs 3 times, at most 1 allowed
```

**When**: A data element occurred more times than its definition's `max_repeat`
allows. ISO 9735-1 §7.6 requires a segment specification to state each element's
maximum number of occurrences.

**Fields**: `tag`, `element_index`, `max: u8`, `actual: usize`, `span: Span`.

**Fix**: Reduce the occurrences, or correct the definition if the directory
allows more. Reported by `CONTRL` as code 35.

---

### E048 — `InvalidCharacterType`

```text
segment UNZ element 0 component 0: "abc" is not n..6
```

**When**: A value's characters do not match its declared representation class —
a letter in an `n` field, for instance.

**Why**: ISO 9735-1 §10 fixes what "numeric" admits: digits, an optional leading
minus, a decimal mark (`.` or `,`), and an exponent. It excludes the space
character and the plus sign explicitly, and requires at least one digit after a
decimal mark — so `1.` and `.` are rejected while `.5` and `2.00` are not.

**Fields**: `tag`, `element_index`, `component_index`, `repr: String`,
`value: String`, `span: Span`.

**Fix**: Send a value of the declared class. Reported by `CONTRL` as code 37.

---

### E049 — `DataElementTooLong`

```text
segment UNZ element 1 component 0: 20 characters exceeds an..14
```

**When**: A value is longer than its declared representation allows.

**Why**: Length is counted in **characters**, not bytes — ISO 9735-1 §6: "one
graphic character shall be counted as one character, irrespective of the number
of bytes/octets required to encode it", so `ü` counts once. §5 excludes the
release character, which is automatic here because release sequences are resolved
before validation. For a numeric value §10 excludes more still: the sign, the
decimal mark, and the exponent — `-123.45` is five characters, not seven.

**Fields**: `tag`, `element_index`, `component_index`, `repr: String`,
`actual: usize`, `span: Span`.

**Fix**: Shorten the value. Reported by `CONTRL` as code 39.

---

### E050 — `DataElementTooShort`

```text
segment UNB element 0 component 0: 3 characters is short of a4
```

**When**: A value is shorter than its declared **fixed-length** representation.

**Why**: Only a fixed representation (`n8`, `a1`, `a4`) has a minimum above one;
a variable one (`an..35`) is satisfied by any non-empty value, because an empty
value means the element is absent (§8.1) rather than too short.

**Fields**: `tag`, `element_index`, `component_index`, `repr: String`,
`actual: usize`, `span: Span`.

**Fix**: Pad the value, or correct the definition if the directory declares it
variable. Reported by `CONTRL` as code 40.

---

### E051 — `TrailingSeparator`

```text
segment BGM ends in a separator that carries no value
```

**When**: A segment ends in an empty data element, or a composite ends in an
empty component — `BGM+220+'` and `DTM+137:20260101:'`.

**Why**: ISO 9735-1 §8.8.2: "If one or more non-repeating composite data elements
or stand-alone data elements at the end of a segment are omitted, the data
element separators which would normally follow them shall also be omitted."
§8.8.3 says the same for components at the end of a composite.

An **interior** omission keeps its separator (§8.8.2): `BGM+220++9'` is correct and is not reported. So is `BGM+'` —
§8.5 spells a mandatory segment with no data to carry exactly that way.

Raised as a **warning**: a correct parser reads the value anyway.

**Fields**: `tag: String`, `element_index: Option<usize>` (`Some` when the
trailing separators close a composite, `None` when they close the segment),
`span: Span`.

**Fix**: Stop emitting separators once the last value is written. Reported by
`CONTRL` as code 45.

---

### E052 — `GroupsAndMessagesMixed`

```text
interchange mixes groups with ungrouped messages
```

**When**: A `UNH` appears outside every `UNG`…`UNE` in an interchange that uses
groups, or `Writer::end_interchange` would close such an interchange.

**Why**: ISO 9735-1 §7.2 lists what an interchange may contain, and the entries
are exclusive — groups containing messages, *or* bare messages, never both. A
message outside every group has no group to be counted in, so `UNZ` DE 0036
cannot describe the interchange at all.

**Fields**: `span: Span`.

**Fix**: Put every message inside a group, or none of them. Reported by `CONTRL`
as code 30.

---

### E053 — `InsignificantCharacters`

```text
segment ZZZ element 0 component 0: leading zeroes are not suppressed
```

**When**: A **variable-length** value carries characters ISO 9735-1 §9.2 requires
the sender to suppress: leading zeroes in a numeric value, trailing spaces in an
alphabetic or alphanumeric one.

**Why**: A receiver comparing `007` with the code `7`, or `"ACME "` with
`"ACME"`, does not match them. A **warning**: the value is readable.

Two exemptions. §9.2 allows "a single zero before a decimal mark", so
`0.5` is correct while `00.5` is not. And it governs *variable* length elements
only: a **fixed**-length numeric field is zero-padded by design and a fixed text
one space-padded, so neither is reported.

Requires a declared [representation](@/docs/validation.md#data-element-representations)
— without one there is no way to know whether the length is fixed.

**Fields**: `tag`, `element_index`, `component_index`, `kind: Insignificant`,
`span: Span`.

**Fix**: Suppress the characters before sending. Reported by `CONTRL` as code 12.

---

### E054 — `GroupCountMismatch`

```text
UNE DE 0060 of group G1 declares 5 message(s), the group holds 1
```

**When**: `UNE` DE 0060 does not match the number of messages between its `UNG`
and `UNE`.

**Fields**: `group_ref: String`, `expected: u32`, `actual: u32`, `span: Span`
(the `UNE`).

**Fix**: Correct DE 0060. Reported by `CONTRL` as code 29 on the group's `UCF`.

---

### E055 — `InteractiveSegmentNotSupported`

```text
segment UIH belongs to interactive EDI (ISO 9735-3), which is not supported
```

**When**: `UIB`, `UIH`, `UIR`, `UIT` or `UIZ` appears anywhere — as the first
segment (instead of `E015`), between `UNB` and `UNH`, or in a message body.

**Fields**: `tag: String`, `span: Span`.

**Fix**: Exchange the data as a batch interchange. Reported by `CONTRL` as code
47.

---

### E056 — `SecuritySegmentNotSupported`

```text
segment USH is a security segment (ISO 9735-5/-7/-9), which is not supported
```

**When**: `USA`, `USB`, `USC`, `USD`, `USE`, `USF`, `USH`, `USL`, `USR`, `UST`,
`USU`, `USX` or `USY` appears anywhere, message bodies included. The crate does
not verify security headers, so a secured message is refused, never passed as if
it were plain.

**Fields**: `tag: String`, `span: Span`.

**Fix**: Verify and remove the security segments before parsing, or agree on
unsecured exchange. Reported by `CONTRL` as code 47.

---

### E057 — `ReservedServiceSegmentTag`

```text
segment tag UXX is reserved for service segments (ISO 9735-1 §7.7) and defines none
```

**When**: A tag starting with `U` is neither a batch service segment (`UNB`
`UNZ` `UNG` `UNE` `UNH` `UNT` `UNS` `UNO` `UNP` `UGH` `UGT` `UCI` `UCF` `UCM`
`UCS` `UCD`) nor `E055`/`E056`.

**Fields**: `tag: String`, `span: Span`.

**Fix**: Rename or remove the segment. Reported by `CONTRL` as code 15.

---

### E058 — `DataAfterInterchangeTrailer`

```text
segment UNB follows the interchange trailer UNZ; one input holds one interchange
```

**When**: Anything but whitespace follows the first `UNZ` outside a message: a
second interchange, a stray segment. Reported once, at the first such segment;
the first interchange is validated as if it were the whole input. A `UNA` after
the first segment is `E058` from the parser (span: the three bytes `UNA`), on
the slice, the reader and the recovering path alike.

**Fields**: `tag: String`, `span: Span`.

**Fix**: Split the input after each `UNZ` and parse every interchange on its
own. Reported by `CONTRL` as code 33 on `UCI`.

---

### E059 — `UnsupportedSyntaxVersion`

```text
UNB S001 DE 0002 syntax version "9" is not supported; expected 1, 2, 3 or 4
```

**When**: DE 0002 is anything but `1`, `2`, `3` or `4` — syntax `X` included.
Reported once; representations that differ between versions then accept either
form.

**Fields**: `value: String`, `span: Span` (DE 0002).

**Fix**: Declare a supported version. Reported by `CONTRL` as code 2 on `UCI`.

---

### E060 — `InvalidDateTime`

```text
segment UNB element 3 component 0: "261399" is not a valid date
```

**When**: `UNB` or `UNG` S004 has the right digit count but is no calendar date
or 24-hour time: month 01–12, day valid for the month, hour 00–23, minute
00–59. `YYMMDD` (syntax 1–3) admits 29 February in every year divisible by 4,
because the century is not transferred; `CCYYMMDD` (syntax 4) follows the
Gregorian rule. A value of the wrong length is `E049`/`E050`, never also
`E060`.

**Fields**: `tag`, `element_index`, `component_index` (`0` date, `1` time),
`value`, `span: Span`.

**Fix**: Send a real date and time. Reported by `CONTRL` as code 12.

---

### E061 — `EmptyGroup`

```text
group G1 contains no message
```

**When**: A `UNG`…`UNE` holds no message (ISO 9735-1:2002 §7.3).

**Fields**: `group_ref: String`, `span: Span` (the `UNG`).

**Fix**: Put at least one message in the group, or omit it. Reported by
`CONTRL` as code 32 on the group's `UCF`.

---

### E066 — `ActionNotPermitted`

```text
CONTRL UCI action code 8 is not permitted by the action policy
```

**When**: A `CONTRL` built with `ContrlOptions` (`receipt_with`,
`acknowledgement_with`, `from_report_with`, `from_recovered`) would carry a
DE 0083 action code on `UCI`, `UCF` or `UCM` that the configured `ActionPolicy`
does not admit — a receipt (`8`) under a policy admitting only `7` and `4`, or a
warning-only `UCM` (`7`) under one admitting only `4`.

The verdict is never rewritten into a permitted code. The first violation
(`UCI`, then each `UCF` and `UCM` in document order) is returned.

**Fields**: `segment_tag: &'static str` (`UCI`, `UCF`, `UCM`),
`action: &'static str` (`4`, `7`, `8`).

**Fix**: Permit the code at that level, or report what the policy admits — for
example turn warnings off with `ContrlOptions::report_warnings(false)` when
`UCM` admits only `4`. Not a fault of the subject interchange, so it has no
`CONTRL` location.

---

### E067 — `InvalidContrlIdentifier`

```text
CONTRL message identifier: data element 0057 value "2.0b-xx" is not valid
```

**When**: `ContrlIdentifier::new` or `with_association` received a component
that violates its S009 representation: DE 0052, DE 0054 or DE 0051 empty or
longer than three characters, DE 0057 empty or longer than six.

**Fields**: `data_element: &'static str` (`0052`, `0054`, `0051`, `0057`),
`value: String`.

**Fix**: Use the identifier the partner agreement names, within those lengths.
Not a fault of the subject interchange, so it has no `CONTRL` location.

---

### E068 — `SegmentNotPlaced`

```text
segment LOC is not placed: out of order: it fits a place already passed
```

**When**: Resolving a message against a group schema
(`GroupSchema::resolve(..).findings(..)`, `PlacementValidator`), a segment fits
no entry of the **ordered** group it arrived in, nor any place of an enclosing
group. It is skipped — never attached to a wrong group — and recorded on the
instance it was skipped in (`SegmentGroupIndexed::unplaced`).

**Fields**: `tag: String`, `reason: UnplacedReason` (`OutOfOrder` — it fits a
place already passed; `NotHere` — it fits a place elsewhere in the schema;
`Unknown` — it fits no place), `span: Span`. The issue carries kind
`placement`, layer `structure`, the `group_path` of the instance and, inside a
message, the `segment_position`.

**Fix**: Move the segment to the place the implementation guide gives it, or
remove it. An ordered root that does not declare `UNT` reports the trailer this
way — declare it, or resolve `MessageWindow::body()`.

---

### E069 — `TooManySegmentOccurrences`

```text
segment DTM occurs more than 1 time(s) in its group
```

**When**: A segment place occurs in one group instance more often than its
`max`, and no later entry of the group matches the extra occurrence (a place at
its `max` yields to a later matching entry first). One issue per extra
occurrence, kind `count`; a place `id` is in the context under `place`.

**Fields**: `tag: String`, `max: u32`, `span: Span` (the extra occurrence).

**Fix**: Reduce the repetitions to the guide's maximum.

---

### E070 — `TooManyGroupOccurrences`

```text
segment group SG5 occurs more than 1 time(s)
```

**When**: A segment group occurs in its parent instance more often than its
`max`. One issue per extra instance, kind `count`, with the `group_path` of the
extra instance.

**Fields**: `group: String`, `max: u32`, `span: Span` (the extra instance's
trigger segment).

**Fix**: Reduce the group's repetitions to the guide's maximum.

---

### E071 — `InvalidGroupSchema`

```text
invalid group schema at "SG5": max is 0
```

**When**: `GroupDefBuilder::build`, `GroupSchema::new` or `GroupSchema::ordered`
found a malformed schema: an empty group or root name, a tag that is not three
upper-case ASCII letters, `min` greater than `max`, `max` 0, a
`Discriminator::one_of` with no code, or groups nested deeper than 64 levels.
`GroupSchema` also checks groups built with the `const` constructors, which
bypass the builder.

**Fields**: `group: String` (the group at fault, or the root name),
`reason: &'static str`.

**Fix**: Correct the schema. A configuration fault, not a fault of a message:
it has no `CONTRL` location.

A place or group below its `min` is reported as
[`E015 MissingSegment`](#e015-missingsegment), kind `missing`, with the
instance's `group_path` and the group's trigger tag.

---

### E072 — `WriterPoisoned`

```text
writer is poisoned: a message was dropped without finish(); no further output was written
```

**When**: A write or completion method of `Writer`, `MessageWriter` or
`WriterEmitter` was called after an earlier failure poisoned the writer:

- a write to the sink failed (`E010`);
- a call failed after it had written bytes, such as `E038` in the middle of a
  value;
- a `MessageWriter` was dropped without a successful `finish()` — an early `?`
  or a panic;
- `MessageWriter::finish` failed to write `UNT`.

Refusals — `E006`, `E024`, `E037`, `E041` and every `begin_interchange`/`end_interchange` refusal — write nothing and do not poison.

**Fields**: `reason: &'static str`, the cause.

**Fix**: The output is incomplete; do not send it. Start a new `Writer`.
`Writer::into_inner` (or `WriterEmitter::into_inner`) returns the sink with the
partial bytes for inspection. Close every message with `MessageWriter::finish()`.

---

## Retired codes

A code is never reissued, so a stored code always means the same condition:

| Code | Former variant | Retired because |
|---|---|---|
| E018 | `ValidationFailed` | Superseded by `ValidationErrors` (E030). |
| E022 | `UnexpectedMessageType` | Message dispatch is a `match` on `MessageWindow::message_type`, not a registry. |
| E029 | `FunctionalGroupNotSupported` | Functional groups (`UNG`/`UNE`) are parsed and validated natively. |
| E062–E065 | — | Reserved; never assigned. |

---

## Matching errors

`EdifactError` is `#[non_exhaustive]`: include a wildcard arm.

```rust
use edifact::EdifactError;

fn handle(err: EdifactError) {
    match err {
        EdifactError::UnexpectedEof { offset } => {
            eprintln!("E001 truncated input at byte {offset}");
        }
        EdifactError::InvalidCodeValue { value, code_list, .. } => {
            eprintln!("E014 bad code '{value}' in list {code_list}");
        }
        EdifactError::Io(e) => {
            eprintln!("E010 I/O: {e}");
        }
        other => {
            eprintln!("{} {other}", other.stable_code());
        }
    }
}
```

---

## Stable codes in logs

Log `err.stable_code()` as a searchable field:

```rust,ignore
use edifact::from_bytes;

match from_bytes(b"BAD").collect::<Result<Vec<_>, _>>() {
    Ok(_) => {}
    Err(e) => {
        tracing::error!(code = e.stable_code(), error = %e, "EDIFACT parse error");
    }
}
```

---

## Diagnostics

The `diagnostics` feature implements `miette::Diagnostic` for `EdifactError`:
the stable code, severity and a help text. See [Diagnostics](@/docs/diagnostics.md).

---

## Next steps

- [Diagnostics](@/docs/diagnostics.md) — miette span-annotated rendering
- [Validation](@/docs/validation.md) — `ValidationReport` vs `EdifactError`
- [Performance](@/docs/performance.md) — error-free fast paths
