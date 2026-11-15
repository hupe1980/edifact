+++
title = "Typed Derive"
description = "Map EDIFACT segments and messages onto Rust structs with #[derive(EdifactDeserialize, EdifactSerialize)] and address fields by UN/EDIFACT data element identifier."
weight = 50
+++

`#[derive(EdifactDeserialize, EdifactSerialize)]` maps EDIFACT segments and
messages onto plain Rust structs. Composite elements get
`EdifactCompositeDeserialize` / `EdifactCompositeSerialize`.

---

## Quick example

```rust
use edifact::{EdifactDeserialize, EdifactSerialize, from_bytes};

#[derive(Debug, EdifactDeserialize, EdifactSerialize)]
#[edifact(segment = "BGM")]
struct Bgm {
    #[edifact(element = 0)]
    document_name_code: String,
    #[edifact(element = 1)]
    document_number: String,
    #[edifact(element = 2)]
    function_code: Option<String>,
}

let segs: Vec<_> = from_bytes(b"BGM+220+PO-4711+9'").collect::<Result<_, _>>()?;
let bgm = Bgm::edifact_deserialize(&segs)?;

assert_eq!(bgm.document_name_code, "220");
assert_eq!(bgm.document_number, "PO-4711");
assert_eq!(bgm.function_code.as_deref(), Some("9"));
# Ok::<(), edifact::EdifactError>(())
```

---

## Struct-level attributes (`#[edifact(...)]` on the struct)

### `segment = "TAG"` — declare a segment struct

```rust
# use edifact::{EdifactDeserialize, EdifactSerialize};
#[derive(EdifactDeserialize, EdifactSerialize)]
#[edifact(segment = "DTM")]
struct Dtm {
    #[edifact(element = 0, component = 0)]
    qualifier: String,
    #[edifact(element = 0, component = 1)]
    value: String,
    #[edifact(element = 0, component = 2)]
    format: String,
}
```

With `segment`, the struct is a **segment struct**: `edifact_deserialize`
reads the first segment tagged `"TAG"`, `EdifactSerialize` emits one segment,
and `EdifactSegmentTag::SEGMENT_TAG` is `"TAG"`. Without it, the struct is a
[message struct](#message-structs).

### `qualifier = "VALUE"` — fixed qualifier matching

```rust
# use edifact::{EdifactDeserialize, EdifactSerialize};
#[derive(EdifactDeserialize, EdifactSerialize)]
#[edifact(segment = "NAD", qualifier = "MS")]
struct NadMs {
    #[edifact(element = 1)]
    party_id: Option<String>,
}
```

The generated `matches_segment` checks that element 0 equals `"MS"`. `*` is a
wildcard:

```text
#[edifact(segment = "NAD", qualifier = "M*")]   // matches "MS", "MR", "MT", …
```

### `qualifier_from = N` — dynamic qualifier at runtime

```rust
# use edifact::{EdifactDeserialize, EdifactSerialize};
#[derive(EdifactDeserialize, EdifactSerialize)]
#[edifact(segment = "NAD", qualifier_from = 0)]
struct Nad {
    #[edifact(element = 0)]
    qualifier: String,       // ← this field holds the runtime qualifier
    #[edifact(element = 1)]
    party_id: Option<String>,
}
```

The qualifier is read from element 0 at runtime, so the struct represents any
`NAD`. A message struct then binds variants to separate fields:

```rust
# use edifact::{EdifactDeserialize, EdifactSerialize};
# #[derive(Debug, EdifactDeserialize, EdifactSerialize)]
# #[edifact(segment = "BGM")]
# struct Bgm { #[edifact(element = 0)] code: String }
# #[derive(Debug, EdifactDeserialize, EdifactSerialize)]
# #[edifact(segment = "NAD")]
# struct Nad { #[edifact(element = 0)] qualifier: String }
#[derive(EdifactDeserialize)]
struct OrderMessage {
    bgm: Option<Bgm>,
    #[edifact(qualifier = "BY")]
    buyer: Option<Nad>,
    #[edifact(qualifier = "SU")]
    supplier: Option<Nad>,
}
```

### `layout = PATH` — resolve fields by UN/EDIFACT data element identifier

Points the struct at a `SegmentDefinition` for the
[code-addressed `element`](#element-by-identifier). The path must name a
**`const`**: identifiers resolve during const evaluation, which cannot read a
`static`:

```rust
use edifact::{
    ComponentRef, EdifactDeserialize, EdifactSerialize, ElementRef, SegmentDefinition, Status,
};

const C082: &[ComponentRef] = &[
    ComponentRef::new(1, "3039", Status::Mandatory),
    ComponentRef::new(2, "1131", Status::Conditional),
    ComponentRef::new(3, "3055", Status::Conditional),
];
const NAD_ELEMENTS: &[ElementRef] = &[
    ElementRef::new(1, "3035", Status::Mandatory, 1),
    ElementRef::composite(2, "C082", Status::Conditional, 1, C082),
];
pub const NAD: SegmentDefinition =
    SegmentDefinition::new("NAD", "Name and address", NAD_ELEMENTS);

#[derive(EdifactDeserialize, EdifactSerialize)]
#[edifact(segment = "NAD", qualifier = "MS", layout = NAD)]
struct SenderParty {
    #[edifact(element = "3039")]
    party_id: String,
    #[edifact(element = "3055")]
    agency: Option<String>,
}
```

`layout` requires `segment`.

#### Where layouts come from

`edifact::service` ships the ISO 9735 batch service segments — `UNB`, `UNG`,
`UNH`, `UNT`, `UNE`, `UNZ`, `UNS`, `UNO`, `UNP`, `UGH`, `UGT` — and the
`CONTRL` segments `UCI`, `UCM`, `UCF`, `UCS`, `UCD`:

```rust
use edifact::{EdifactDeserialize, service};

#[derive(EdifactDeserialize)]
#[edifact(segment = "UNB", layout = service::UNB)]
struct InterchangeHeader {
    #[edifact(element = "0020")]
    control_reference: String,
    #[edifact(element = "0035")]
    test_indicator: Option<String>,
}
```

#### Message-level segments are yours to supply

Layouts for **message-level** segments — `BGM`, `DTM`, `NAD`, `LIN`, composites
`C002`, `C507`, `C082`, … — are yours to supply. The
[UN UNTDID licence](https://service.unece.org/trade/untdid/license.htm)
forbids distributing modified directory content, so the crate ships nothing
from the directories.

Write the subset you use as `const` tables (as above), generate them from your
directory of record, or load them at startup with `DirectoryValidatorBuilder`.
**Audit the result against real messages**: see
[Auditing a hand-written layout](@/docs/validation.md#auditing-a-hand-written-layout).

A composite that repeats a data element by design — `C080` is `3036` five
times — uses `ComponentRef::repeated`. Five `ComponentRef::new` entries are
five positions, and the code then resolves as ambiguous:

```rust
use edifact::{ComponentRef, Status};

const C080: &[ComponentRef] = &[
    ComponentRef::repeated(1, "3036", Status::Mandatory, 5),
    ComponentRef::new(6, "3045", Status::Conditional),
];
```

---

## Field-level attributes (`#[edifact(...)]` on a field)

### `element = N` — positional element index (0-based)

```text
#[edifact(element = 2)]
function_code: Option<String>,
```

Maps the field to **component 0 of element N**.

### `element = N, component = C` — composite component

```text
#[edifact(element = 0, component = 1)]
date_value: String,
```

Maps the field to one component of a composite (e.g. `DTM` C507 components
0/1/2).

### `element = "3055"` — data element identifier {#element-by-identifier}

Under a struct-level `layout`, `element` also accepts a UN/EDIFACT data element
identifier instead of an index:

```text
#[edifact(element = "3055")]
agency: Option<String>,
```

The identifier resolves against the layout **at compile time**, to element and
component (`3055` in `C082`). A transposed positional index reads the wrong
data element silently; an identifier cannot:

| Mistake | Positional form | Code form |
|---|---|---|
| Identifier not in this segment | reads a neighbouring element | **compile error** |
| Identifier defined at two positions | — | **compile error** |
| Two fields claiming one slot | compile error | **compile error** |
| Wrong segment's definition | — | compile error (or `E035` at runtime) |

With `required`, an identifier naming a whole data element reports
`MissingRequiredElement` (`E008`); one naming a component reports
`MissingRequiredComponent` (`E021`).

`component = N` may accompany a code that names a whole data element; with a
code that names a component it is a compile error.

For runtime lookups against the same metadata, see
[`Segment::value_by_code`](@/docs/validation.md#code-addressed-element-access).

### `composite` — full composite element

```rust
# use edifact::{
#     EdifactCompositeDeserialize, EdifactCompositeSerialize, EdifactDeserialize, EdifactSerialize,
# };
#[derive(EdifactCompositeDeserialize, EdifactCompositeSerialize)]
struct PartyId {
    id: String,
    code_list: Option<String>,
    agency: Option<String>,
}

#[derive(EdifactDeserialize, EdifactSerialize)]
#[edifact(segment = "NAD")]
struct Nad {
    #[edifact(element = 0)]
    qualifier: String,
    #[edifact(element = 1, composite)]
    party: Option<PartyId>,
}
```

`composite` hands the whole element to the field's
`EdifactCompositeDeserialize` impl. Field *n* maps to component *n*, so
`NAD+BY+4711::9` fills `id = "4711"`, `code_list = None`, `agency = Some("9")`;
`#[edifact(component = N)]` pins a field. A bare `String` component is
mandatory (`E021` when absent or empty); `Option<String>` is not.

### `group` — repeated segment group

```rust
# use edifact::{EdifactDeserialize, EdifactSerialize};
# #[derive(EdifactDeserialize, EdifactSerialize)]
# #[edifact(segment = "BGM")]
# #[derive(Debug)] struct Bgm { #[edifact(element = 0)] code: String }
# #[derive(EdifactDeserialize, EdifactSerialize)]
# #[edifact(segment = "DTM")]
# #[derive(Debug)] struct Dtm { #[edifact(element = 0)] value: String }
# #[derive(EdifactDeserialize, EdifactSerialize)]
# #[edifact(segment = "NAD")]
# #[derive(Debug)] struct Nad { #[edifact(element = 0)] qualifier: String }
# #[derive(EdifactDeserialize, EdifactSerialize)]
# #[edifact(segment = "LIN")]
# #[derive(Debug)] struct Lin { #[edifact(element = 0)] line_no: String }
#[derive(EdifactDeserialize)]
struct OrderMessage {
    bgm: Option<Bgm>,
    #[edifact(group)]
    lines: Vec<Lin>,     // every LIN segment in the message
}
```

`group` on a `Vec<T>` collects every segment `T::matches_segment` accepts; `T`
must implement `EdifactSegmentTag`. For segment-group *structure*, see
[Segment Groups](@/docs/segment-groups.md).

### `repeat` — repeated *data element*

`group` repeats the **segment**; `repeat` repeats the **data element inside one
segment** (ISO 9735-1 §8.7):

```text
RFF+ON:1'RFF+ON:2'       two RFF segments          → #[edifact(group)]
RFF+ON:1*ON:2            one RFF, two occurrences  → #[edifact(repeat)]
```

Each **occurrence** carries every component, hence `ON` twice. On write, a
non-repeating field in the same element is emitted once per occurrence; a
repeating one contributes its *k*-th item.

```rust
use edifact::{EdifactDeserialize, EdifactSerialize, from_bytes};

#[derive(Debug, EdifactDeserialize, EdifactSerialize)]
#[edifact(segment = "RFF")]
struct OrderReferences {
    #[edifact(element = 0, component = 0)]
    qualifier: String,
    #[edifact(element = 0, component = 1, repeat)]
    numbers: Vec<String>,
}

// `UNA` position 050 declares `*`; so does syntax version 4 by default.
let segments: Vec<_> = from_bytes(b"UNA:+.?*'RFF+ON:1*ON:2*ON:3'")
    .collect::<Result<Vec<_>, _>>()?;
let rff = OrderReferences::edifact_deserialize(&segments)?;

assert_eq!(rff.qualifier, "ON");
assert_eq!(rff.numbers, ["1", "2", "3"]);
# Ok::<(), edifact::EdifactError>(())
```

An occurrence without the component yields `""`: positions are significant
(§8.8.4). An empty `Vec` writes the element as absent (§8.8.2).

Writing needs an active repetition separator, else
[`E037`](@/docs/error-reference.md#e037-repetitionseparatornotdeclared) — see
[Writing](@/docs/writing.md#repeating-data-elements).

### `required` — mandatory `Option<T>`

`#[edifact(required)]` on an `Option<T>` field of a segment struct makes an
absent or empty value `MissingRequiredElement` (`E008`) — or `E021` for a
component — while the Rust type stays `Option<T>`. It is a compile error on a
non-`Option` field, with `composite`, or in a message struct.

### `qualifier = "VALUE"` — message-field qualifier filter

```rust
# use edifact::{EdifactDeserialize, EdifactSerialize};
# #[derive(EdifactDeserialize, EdifactSerialize)]
# #[edifact(segment = "BGM")]
# #[derive(Debug)] struct Bgm { #[edifact(element = 0)] code: String }
# #[derive(EdifactDeserialize, EdifactSerialize)]
# #[edifact(segment = "DTM")]
# #[derive(Debug)] struct Dtm { #[edifact(element = 0)] value: String }
# #[derive(EdifactDeserialize, EdifactSerialize)]
# #[edifact(segment = "NAD")]
# #[derive(Debug)] struct Nad { #[edifact(element = 0)] qualifier: String }
# #[derive(EdifactDeserialize, EdifactSerialize)]
# #[edifact(segment = "LIN")]
# #[derive(Debug)] struct Lin { #[edifact(element = 0)] line_no: String }
#[derive(EdifactDeserialize)]
struct OrderMessage {
    #[edifact(qualifier = "BY")]
    buyer: Option<Nad>,
    #[edifact(qualifier = "SU")]
    supplier: Option<Nad>,
}
```

In a message struct, `qualifier` on a field selects the segment by element 0.

---

## Message structs

A struct **without** `#[edifact(segment = "TAG")]` is a **message struct**.
Each field maps to a segment type:

```rust
# use edifact::{EdifactDeserialize, EdifactSerialize};
# #[derive(EdifactDeserialize, EdifactSerialize)]
# #[edifact(segment = "BGM")]
# #[derive(Debug)] struct Bgm { #[edifact(element = 0)] code: String }
# #[derive(EdifactDeserialize, EdifactSerialize)]
# #[edifact(segment = "DTM")]
# #[derive(Debug)] struct Dtm { #[edifact(element = 0)] value: String }
# #[derive(EdifactDeserialize, EdifactSerialize)]
# #[edifact(segment = "NAD")]
# #[derive(Debug)] struct Nad { #[edifact(element = 0)] qualifier: String }
# #[derive(EdifactDeserialize, EdifactSerialize)]
# #[edifact(segment = "LIN")]
# #[derive(Debug)] struct Lin { #[edifact(element = 0)] line_no: String }
# use edifact::from_bytes;
# let input = b"BGM+220'DTM+137'NAD+BY'NAD+SU'LIN+1'";
#[derive(Debug, EdifactDeserialize)]
struct OrderMessage {
    bgm: Option<Bgm>,                // finds first BGM segment
    dtm: Option<Dtm>,                // finds first DTM segment
    #[edifact(qualifier = "BY")]
    buyer: Option<Nad>,              // finds NAD where element 0 == "BY"
    #[edifact(qualifier = "SU")]
    supplier: Option<Nad>,           // finds NAD where element 0 == "SU"
    #[edifact(group)]
    lines: Vec<Lin>,                 // collects all LIN segments
}

let segs: Vec<_> = from_bytes(input).collect::<Result<_, _>>()?;
let msg = OrderMessage::edifact_deserialize(&segs)?;
# Ok::<(), edifact::EdifactError>(())
```

Field rules:
- `Option<T>` — field is optional; returns `None` if the segment is not found.
- `T` (non-optional) — field is required; returns `EdifactError::MissingSegment` if absent.
- `Vec<T>` with `#[edifact(group)]` — zero or more matches.

---

## Attribute summary table

| Attribute | Target | Description |
|---|---|---|
| `segment = "TAG"` | struct | Declares a segment struct with the given 3-letter tag |
| `qualifier = "Q"` | struct | Filter — segment must have element 0 == `"Q"` (supports `*` suffix wildcard) |
| `qualifier_from = N` | struct | Element N holds the runtime qualifier value |
| `layout = PATH` | struct | `const SegmentDefinition` that code-addressed `element` attributes resolve against |
| `element = N` | field | Positional element index (0-based) |
| `element = "DE"` | field | UN/EDIFACT data element identifier, resolved against `layout` at compile time |
| `component = C` | field | Component index within the element (use with `element`) |
| `composite` | field | Map the whole element to `EdifactCompositeDeserialize` |
| `group` | field | Collect all matching **segments** into a `Vec<T>` |
| `repeat` | field | Map the occurrences of a repeating **data element** into a `Vec<T>` |
| `required` | field (segment struct) | Absent or empty `Option<T>` is an error |
| `qualifier = "Q"` | field (message struct) | Filter which qualifier variant to bind to this field |

---

## Generated trait implementations

For a **segment struct** `#[edifact(segment = "TAG")]` the macro generates:

| Trait | Method | Notes |
|---|---|---|
| `EdifactDeserialize` | `edifact_deserialize(&[Segment<'_>])` | Finds the first matching segment |
| `EdifactSerialize` | `edifact_serialize(&mut E)` | Emits one `StartSegment` .. `EndSegment` event sequence |
| `EdifactSegmentTag` | `SEGMENT_TAG` | The `"TAG"` string as a const |
| `EdifactSegmentTag` | `QUALIFIER_PATTERN` | `Some("Q")` or `None` |
| `EdifactSegmentTag` | `matches_segment(seg)` | Checks tag and qualifier |

`&[OwnedSegment]` coerces to `&[Segment<'_>]`, so one `edifact_deserialize`
serves both parse paths.

---

## Limitations

Each is a compile error with a span:

- **No generic or lifetime parameters**: use `String`, not `&str`.
- **Named fields only**: no tuple or unit structs.
- **One segment type per segment struct**; message structs compose them.
- **`layout` must be a `const`** (`const SegmentDefinition`, `const &[ElementRef]`).

---

## Troubleshooting

### "only support structs with named fields"

The derive was applied to a tuple struct, unit struct or enum. Use a struct
with named fields.

### "data element {code} is not defined exactly once in the segment layout"

The identifier is missing from the `layout` or appears at more than one
position — the compile-time form of runtime `E033`/`E034`. A repeat by design
needs `ComponentRef::repeated` ([Where layouts come from](#where-layouts-come-from)).

### "use either #[edifact(qualifier = ...)] or #[edifact(qualifier_from = ...)], not both"

`qualifier` is a fixed filter; `qualifier_from` stores the runtime value. Pick
one.

### "`EdifactSegmentTag` is not implemented for `MyStruct`"

`Vec<T>` fields need `T: EdifactSegmentTag`, which only segment structs
derive.

### "trait not satisfied" on a field type

Non-`String` fields are read with `FromStr` and written with `Display`; the
type needs both.

---

## Next steps

- [Streaming](@/docs/streaming.md) — use `deserialize_messages_from_reader` for reader-based typed extraction
- [Writing](@/docs/writing.md) — `EdifactSerialize` and the event model
- [Validation](@/docs/validation.md) — validate typed messages against business rules
