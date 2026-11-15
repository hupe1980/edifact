+++
title = "Validation"
description = "The Validator trait, ValidationContext, and the layered envelope / syntax / charset / structure / code-list / profile validation pipeline."
weight = 70
+++

Validation runs in layers — envelope, syntax, charset, structure, code list,
profile — each switchable, all writing into one `ValidationReport`.
Segment-group validation has its own guide: [Segment Groups](@/docs/segment-groups.md).

---

## Key types

| Type | Role |
|---|---|
| `Validator` | Trait for a custom validator |
| `ValidationContext` | Runs registered validators, layer by layer |
| `ValidationLayer` | `Envelope`, `Syntax`, `Charset`, `Structure`, `CodeList`, `Profile`; registration key and an issue's producing stage |
| `ValidationReport` | All findings: `errors`, `warnings`, `infos` |
| `ValidationIssue` | One finding: severity, message, rule ID, kind, layer, pack, location |
| `FindingKind` | `Missing`, `NotPermitted`, `Code`, `Representation`, `Count`, `Placement`, … |
| `GroupPath` / `GroupStep` | Path to the group instance holding a finding, e.g. `SG4[0]/SG8[1]` |
| `ValidationSeverity` | `Critical`, `Error`, `Warning`, `Info` |
| `ProfileRulePack` | Bundle of closure-based profile rules |
| `validate_each` | Runs a per-segment function over a slice |

---

## The `Validator` trait

```rust
use edifact::{Validator, ValidationReport, ValidationRuleContext, Segment, validate_each, EdifactError};

struct BgmCodeValidator;

impl Validator for BgmCodeValidator {
    fn validate_batch(
        &self,
        segments: &[Segment<'_>],
        report: &mut ValidationReport,
        _context: &ValidationRuleContext<'_>,
    ) {
        validate_each(segments, report, |seg| {
            if seg.tag == "BGM" {
                let code = seg.element_str(0).unwrap_or("");
                if !matches!(code, "220" | "231" | "261") {
                    return Err(EdifactError::InvalidCodeValue {
                        tag: "BGM".to_owned(),
                        element_index: 0,
                        value: code.to_owned(),
                        code_list: "1001".to_owned(),
                        span: seg.span,
                        suggestion: Some("Use 220 (order), 231 (order response) or 261 (confirmation)"),
                    });
                }
            }
            Ok(())
        });
    }
}
```

`validate_batch` receives the whole slice being validated: one `UNH..UNT`
window or a whole interchange. `validate_each` turns each `Err` into a
`ValidationIssue` on `report`. `validate_group_batch` (default: no-op) is called
once per group occurrence by [`validate_grouped`](@/docs/segment-groups.md).

---

## `ValidationContext` and layers

Validators are registered under a layer and run in registration order; a
validator whose layer is switched off is skipped.

```rust
use edifact::{
    ValidationContext, ValidationLayer, Validator, ValidationReport,
    ValidationRuleContext, Segment, from_bytes,
};

# struct BgmCodeValidator;
# impl Validator for BgmCodeValidator {
#     fn validate_batch(&self, _: &[Segment<'_>], _: &mut ValidationReport, _: &ValidationRuleContext<'_>) {}
#     fn set_message_type(&mut self, _: Option<&str>) {}
# }
let segs: Vec<_> = from_bytes(b"UNH+1+ORDERS:D:11A:UN'BGM+220+PO-4711+9'UNT+3+1'")
    .collect::<Result<_, _>>()?;

let ctx = ValidationContext::builder()
    .with_message_type("ORDERS")           // passed to set_message_type on each validator
    .with_validator(ValidationLayer::CodeList, BgmCodeValidator)
    .build();

let report = ctx.validate(&segs);
# Ok::<(), edifact::EdifactError>(())
```

| Builder call | Effect |
|---|---|
| `.structure(bool)`, `.code_list(bool)`, `.profile(bool)` | Switch a layer |
| `.envelope(bool)` | Switch the envelope, syntax and charset layers (off by default) |
| `.with_envelope_validation()` | Add `EnvelopeValidator`, switch the envelope layer on |
| `.with_syntax_validation()` | Add `SyntaxValidator`, switch the envelope layer on |
| `.with_charset_validation()` / `_for(charset)` | Add `CharsetValidator` (declared or fixed repertoire), switch the envelope layer on |
| `.bail_on_first_critical(true)` | Skip every validator after the one that raised a `Critical` |
| `.with_message_ref(r)` | Stamp every issue with the `UNH` reference (DE 0062) |
| `.with_profile_pack_arc(arc)` | Share one `Arc<ProfileRulePack>` across contexts without cloning its closures |

Envelope, syntax and charset validators see the whole slice. While the envelope
layer is on, the other layers see it without `UNB`, `UNZ`, `UNG` and `UNE`.

```rust
use edifact::{ValidationContext, ProfileRulePack};
use std::sync::Arc;

let pack = Arc::new(ProfileRulePack::new("ORDERS-RULES").for_message_type("ORDERS"));

let ctx = ValidationContext::builder()
    .code_list(false)
    .bail_on_first_critical(true)
    .with_message_ref("MSG-42")
    .with_profile_pack_arc(Arc::clone(&pack))
    .build();
```

### Built-in envelope validation

`EnvelopeValidator` checks the ISO 9735-1 interchange structure: `UNB`/`UNZ`,
`UNG`/`UNE` groups, `UNH`/`UNT` messages, the control references that pair
them, and the three control counts.

```rust
use edifact::{ValidationContext, from_bytes};

# let segs: Vec<_> = from_bytes(b"UNB+UNOA:1+SENDER:1+RECEIVER:1+200101:1000+1'UNH+1+ORDERS:D:96A:UN'BGM+220+PO-1'UNT+3+1'UNZ+1+1'").collect::<Result<_,_>>()?;
let report = ValidationContext::builder()
    .with_envelope_validation()
    .build()
    .validate(&segs);
# Ok::<(), edifact::EdifactError>(())
```

| Rule | Code |
|---|---|
| `UNB`/`UNG`/`UNH` open and `UNZ`/`UNE`/`UNT` close; groups do not nest | `E015`, `E011` |
| `UNZ`/`UNE`/`UNT` repeat their header's reference | `E016` |
| `UNH` DE 0062 and `UNG` DE 0048 are unique in the interchange | `E032` |
| `UNT` DE 0074 equals the message's segment count | `E005` |
| `UNE` DE 0060 equals the group's message count | `E054` |
| `UNZ` DE 0036 equals the message count, or the group count when groups are used | `E004` |
| An interchange holds a message or group | `E042` |
| A group holds a message | `E061` |
| A message holds a segment between `UNH` and `UNT` | `E043` |

`validate_envelope` returns the typed `UNB` as `interchange.header`, the
`InterchangeHeader` that `InterchangeHeader::parse` returns:

```rust
use edifact::{Party, from_bytes, validate_envelope};

let segs: Vec<_> = from_bytes(b"UNB+UNOC:3+SENDER:14+RECEIVER+260101:0900+IC1'UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+3+1'UNZ+1+IC1'")
    .collect::<Result<_, _>>()?;
let result = validate_envelope(&segs)?;
assert_eq!(result.interchange.header.sender, Party::new("SENDER").with_qualifier("14"));
assert_eq!(result.interchange.header.control_ref, "IC1");
# Ok::<(), edifact::EdifactError>(())
```

#### Service segments are checked by default

`validate_envelope`, `validate_envelope_lenient` and the `Envelope` layer check
every position of `UNB`, `UNZ`, `UNG`, `UNE`, `UNH` and `UNT` against the
`service` tables (ISO 9735-10:2002 §5) for the interchange's syntax version.
Every finding is an error, so a passing `validate_envelope` means a conformant
envelope.

| Check | Code |
|---|---|
| Mandatory element / component present | `E008` / `E021` |
| Character class, maximum and fixed length | `E048`, `E049`, `E050` |
| S004 is a calendar date and a 24-hour time | `E060` |
| `UNB` S001 DE 0002 is `1`, `2`, `3` or `4` | `E059` |
| `UIB`…`UIZ` (interactive), `USA`…`USY` (security), any other `U` tag — anywhere, message bodies included | `E055`, `E056`, `E057` |
| Nothing but whitespace after the first `UNZ` outside a message | `E058` |

Representations that differ between syntax versions:

| Data element | Versions 1–3 | Version 4 |
|---|---|---|
| 0008, 0014 | `an..14` | `an..35` |
| 0017 (S004 date) | `n6` `YYMMDD` | `n8` `CCYYMMDD` |
| 0051 | `an..2` | `an..3` |
| 0074 | `n..6` | `n..10` |
| 0135 | `a3` | `an..3` |

With an unknown DE 0002 (`E059`) either form is accepted. A count that is not
a number is `E048` and is not compared.

```rust
use edifact::{from_bytes, validate_envelope_lenient};

let segs: Vec<_> = from_bytes(
    b"UNB+UNOC:3+S+R+261399:2599+1'UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+3+1'UNZ+1+1'",
)
.collect::<Result<_, _>>()?;
let codes: Vec<_> = validate_envelope_lenient(&segs)
    .errors
    .iter()
    .map(|e| e.stable_code())
    .collect();
assert_eq!(codes, ["E060", "E060"]); // month 13, hour 25
# Ok::<(), edifact::EdifactError>(())
```

A context that also runs `DirectoryValidator(service::lookup)` reports a
finding with the same code, segment, element and component once, from the
`Envelope` layer. It is matched on its span or, without one (`E008`, `E021`),
on its `segment_position`; a finding with neither is never merged.

`segment_position` counts from each message's own `UNH` (`UNH` = 1) in every
layer. A segment outside every message (`UNB`, `UNG`, `UNE`, `UNZ`) has none.

### Syntax validation

`SyntaxValidator` checks rules that hold for every interchange and needs no
configuration:

| Rule | Source | Code |
|---|---|---|
| A segment carries at least one data element besides its tag | §7.6, §8.6 | `E046` |
| No data element value is made only of spaces | §9.4 | `E045` (warning) |
| No segment or composite ends in a separator with nothing after it | §8.8.2, §8.8.3 | `E051` (warning) |

An interior omission keeps its separator (§8.8.2): `BGM+220++9'` is correct.
`DTM+'` is not `E046`: an empty data element is how §8.5 spells a mandatory
segment with no data.

```rust
use edifact::{ValidationContext, from_bytes};

let segments: Vec<_> = from_bytes(b"FTX+   'DTM'").collect::<Result<Vec<_>, _>>()?;

let report = ValidationContext::builder()
    .with_syntax_validation()
    .build()
    .validate(&segments);

assert_eq!(report.errors()[0].error_code(), Some("E046"));   // DTM has no data element
assert_eq!(report.warnings()[0].error_code(), Some("E045")); // FTX value is only spaces
# Ok::<(), edifact::EdifactError>(())
```

---

## Collecting issues, and turning them into a `Result`

Every validate call runs every enabled validator and returns a
`ValidationReport`. Ask the report for a `Result` when you want one:

| Call | Returns |
|---|---|
| `ctx.validate(&segs)` | `ValidationReport` — every issue found |
| `ctx.validate(&segs).result()` | `Result<ValidationReport, ValidationReport>` — `Err` when any `Error`/`Critical` was found |
| `ctx.validate_with(&segs, &meta)` | as `validate`, with typed per-call metadata for rule closures |
| `ctx.validate_grouped(&tree, &segs)` | as `validate`, plus the [group pass](@/docs/segment-groups.md) |

`&[OwnedSegment]` coerces to `&[Segment<'_>]`, so reader output needs no
conversion.

```rust
# use edifact::{ValidationContext, from_bytes};
# let segs: Vec<_> = from_bytes(b"BGM+220+PO-4711+9'").collect::<Result<_,_>>()?;
# let ctx = ValidationContext::builder().build();
match ctx.validate(&segs).result() {
    Ok(report) => println!("valid, {} warnings", report.warnings().len()),
    Err(report) => {
        for issue in report.errors() {
            eprintln!("error [{}]: {}", issue.error_code().unwrap_or("?"), issue.message);
        }
    }
}
# Ok::<(), edifact::EdifactError>(())
```

---

## `ValidationReport` — working with results

```rust
# use edifact::{ValidationContext, from_bytes};
# let segs: Vec<_> = from_bytes(b"BGM+220+PO-4711+9'").collect::<Result<_,_>>()?;
# let ctx = ValidationContext::builder().build();
let report = ctx.validate(&segs);

println!("valid: {}", report.is_valid()); // no errors, no criticals
println!("{} errors, {} warnings, {} infos, {} total",
    report.errors().len(), report.warnings().len(), report.infos().len(), report.total_issues());

// Stable text for snapshots and golden tests.
println!("{}", report.render_deterministic());

// Filter by rule ID prefix or exact rule ID.
let profile_issues = report.filter_by_rule_prefix("ORDERS-DEMO-");
let count = report.issues_for_rule_id("ORDERS-P001").count();
# let _ = (profile_issues, count);
# Ok::<(), edifact::EdifactError>(())
```

---

## `ValidationIssue` — building findings

```rust
use edifact::{Span, ValidationIssue, ValidationSeverity};

let issue = ValidationIssue::new(
    ValidationSeverity::Error,
    "BGM document code 999 is not accepted",
)
.with_rule_id("ORDERS-BGM-001")
.with_segment("BGM")
.with_element_index(0)
.with_error_code("E014")
.with_suggestion("Use code 220, 231, or 261")
.with_span(Span::new(42, 60));
```

| Builder method | Type | Purpose |
|---|---|---|
| `.with_rule_id(id)` | `&str` | Stable ID for filtering and mapping |
| `.with_segment(tag)` | `&str` | Segment tag |
| `.with_element_index(n)` | `usize` | 0-based element (CONTRL DE 0098 is `n + 2`); stored as `u16` |
| `.with_component_index(n)` | `usize` | 0-based component (CONTRL DE 0104 is `n + 1`); stored as `u16` |
| `.with_error_code(code)` | `impl Into<Cow<'static, str>>` | Stable code (e.g. `"E014"`); a `&'static str` does not allocate |
| `.with_suggestion(text)` | `&str` | Remediation hint |
| `.with_span(span)` | `Span` | Byte range in the input; `issue.start_offset()` reads the start |
| `.with_segment_occurrence(n)` | `usize` | 0-based occurrence among segments with the same tag; stored as `u32` |
| `.with_segment_position(n)` | `usize` | 1-based position in the message, `UNH` = 1 (CONTRL DE 0096); stored as `u32` |
| `.with_segment_group(name)` | `impl Into<Arc<str>>` | Innermost group (e.g. `"SG5"`); set by group rules |
| `.with_group_path(path)` | `GroupPath` | Group instance path (e.g. `SG4[0]/SG8[1]`); set by group rules, also fills an empty `segment_group` |
| `.with_layer(layer)` | `ValidationLayer` | Producing stage; filled from the registration layer when unset |
| `.with_kind(kind)` | `FindingKind` | Kind of fault |
| `.with_pack(name)` | `impl Into<Arc<str>>` | Producing pack; set by `ProfileRulePack` |
| `.with_message_ref(r)` | `impl Into<String>` | `UNH` reference (DE 0062); set by `with_message_ref` on the builder |
| `.with_context_entry(k, v)` | `(impl Into<String>, impl Into<String>)` | One domain metadata entry |
| `.with_context_entries(iter)` | `impl IntoIterator<Item=(K,V)>` | Several entries; duplicate keys overwrite |

Positional builders never clamp: a value that does not fit leaves the field
`None`, because a clamped index points at a plausible but wrong segment.
Occurrence and position are different coordinates — the 70 000th `QTY` after
`UNH`, `BGM`, `LIN` is occurrence `69_999`, position `70_003`:

```rust
use edifact::{ValidationIssue, ValidationSeverity};

let issue = ValidationIssue::new(ValidationSeverity::Error, "QTY qualifier must be 220")
    .with_segment("QTY")
    .with_segment_occurrence(69_999)
    .with_segment_position(70_003)
    .with_element_index(0);
assert_eq!(issue.segment_position(), Some(70_003));
assert_eq!(issue.with_element_index(70_000).element_index(), None); // not 65_535
```

### Classifying findings: layer, kind and pack

Routing reads structured fields, never a rule ID:

| Field | Type | Filled by |
|---|---|---|
| `kind()` | `Option<FindingKind>` | Every built-in validator; converted errors use `FindingKind::for_error`. A caller's issue without a kind stays `None`. |
| `layer()` | `Option<ValidationLayer>` | Built-in validators set their own; `ValidationContext` fills the registration layer when unset. |
| `pack()` | `Option<&str>` | `ProfileRulePack` stamps its name when unset. |
| `group_path()` | `Option<&GroupPath>` | The group pass of `validate_grouped`. |

| Validator | Layer | Kind |
|---|---|---|
| `EnvelopeValidator` | `Envelope` | `FindingKind::for_error` (`ControlMismatch`, `Envelope`, `Duplicate`, …) |
| `SyntaxValidator` | `Syntax` | `Representation` (`E045`), `Syntax` (`E046`, `E051`) |
| `CharsetValidator` | `Charset` | `Charset` |
| `DirectoryValidator` | `CodeList` for `E014`, `Structure` otherwise | `FindingKind::for_error`; missing segment → `Missing`, out of order → `Placement` |
| `ProfileRulePack` helpers | `Profile` (from the context) | `require_segment*` → `Missing`, `forbid_segment*` → `NotPermitted`, `require_qualifier*` → `Code` |

```rust
use edifact::{FindingKind, ProfileRulePack, ValidationContext, ValidationLayer, from_bytes};

let segs: Vec<_> = from_bytes(b"UNH+1+ORDERS:D:96A:UN'BGM+220'DTM+137:20240101:102'UNT+4+1'")
    .collect::<Result<_, _>>()?;
let report = ValidationContext::builder()
    .with_syntax_validation()
    .with_profile_pack(ProfileRulePack::new("AHB-ORDERS").forbid_segment("DTM", "AHB-DTM-X"))
    .build()
    .validate(&segs);

let issue = &report.errors()[0];
assert_eq!(issue.layer(), Some(ValidationLayer::Profile));
assert_eq!(issue.kind(), Some(FindingKind::NotPermitted));
assert_eq!(issue.pack(), Some("AHB-ORDERS"));
assert_eq!(issue.segment_position(), Some(3)); // UNH = 1
// Syntax, envelope and charset findings answer with a CONTRL; the rest do not.
let for_contrl = report.iter_issues().filter(|i| {
    matches!(
        i.layer(),
        Some(ValidationLayer::Envelope | ValidationLayer::Syntax | ValidationLayer::Charset)
    )
});
assert_eq!(for_contrl.count(), 0);
# Ok::<(), edifact::EdifactError>(())
```

With `serde`, optional fields serialise only when set; an unknown kind reads as
`FindingKind::Other`.

### Rule IDs and context entries

A rule ID names a rule for filtering and overriding. A namespaced form reads
well in reports: `<PACK>-<SCOPE>-<TAG>-<STATUS>`, e.g. `PROFILE-4711-BGM-M`.
Put domain metadata in context entries rather than parsing it back out of the
ID:

```rust
use edifact::{ValidationIssue, ValidationSeverity};

let issue = ValidationIssue::new(ValidationSeverity::Error, "BGM code invalid")
    .with_rule_id("PROFILE-4711-BGM-M")
    .with_context_entry("pid", "4711")
    .with_context_entry("partner", "9900123456789");

assert_eq!(issue.context_get("pid"), Some("4711"));
```

---

## Custom validator implementations

A validator that compares segments implements `Validator` on a struct:

```rust
use edifact::{Validator, ValidationReport, ValidationRuleContext, ValidationIssue, ValidationSeverity, Segment};

struct DeliveryDateRequired;

impl Validator for DeliveryDateRequired {
    fn validate_batch(
        &self,
        segments: &[Segment<'_>],
        report: &mut ValidationReport,
        _context: &ValidationRuleContext<'_>,
    ) {
        let is_order = segments.iter().any(|s| s.is("BGM", "220"));
        let has_delivery = segments.iter().any(|s| s.is("DTM", "2"));
        if is_order && !has_delivery {
            report.add_error(
                ValidationIssue::new(ValidationSeverity::Error, "an order needs DTM+2 (delivery date)")
                    .with_rule_id("ORDERS-DTM-2-M")
                    .with_segment("DTM"),
            );
        }
    }
}
```

Full example with fixtures:
[`cookbook_fixture_validation.rs`](https://github.com/hupe1980/edifact/tree/main/crates/edifact/examples/cookbook_fixture_validation.rs).

---

## Progressive streaming validation

Validate each `UNH..UNT` window as the reader yields it:

```rust
use edifact::{
    ValidationContext, ProfileRulePack, ValidationIssue, ValidationSeverity,
    message_windows_from_reader,
};
use std::io::Cursor;

let input = Cursor::new(b"\
    UNH+1+ORDERS:D:11A:UN'BGM+220+PO-001+9'UNT+3+1'\
    UNH+2+ORDERS:D:11A:UN'BGM+220+PO-002+9'UNT+3+2'".to_vec());

let ctx = ValidationContext::builder()
    .with_profile_pack(
        ProfileRulePack::new("ORDERS-REQUIRED")
            .for_message_type("ORDERS")
            .with_rule_fn(|segs, issues| {
                if !segs.iter().any(|s| s.tag == "BGM") {
                    issues.push(
                        ValidationIssue::new(ValidationSeverity::Error, "BGM is required")
                            .with_rule_id("ORDERS-REQ-BGM"),
                    );
                }
            }),
    )
    .build();

for result in message_windows_from_reader(input) {
    let window = result?;
    let report = ctx.validate(&window.segments);
    println!("message {:?}: {} error(s)", window.message_type, report.errors().len());
}
# Ok::<(), edifact::EdifactError>(())
```

Full example:
[`cookbook_streamed_progressive_validation.rs`](https://github.com/hupe1980/edifact/tree/main/crates/edifact/examples/cookbook_streamed_progressive_validation.rs).

---

## Directory validator

`DirectoryValidator` checks segments against a segment definition dictionary:

```rust
use edifact::{DirectoryValidatorBuilder, OwnedSegmentDef, OwnedElementRef, Status};

let validator = DirectoryValidatorBuilder::new("CUSTOM-D96A")
    .add_segment(
        OwnedSegmentDef::new_unchecked(
            "BGM".to_owned(),
            "Beginning of message".to_owned(),
            vec![
                OwnedElementRef::new_unchecked(1, "1001".to_owned(), Status::Conditional, 1),
                OwnedElementRef::new_unchecked(2, "1004".to_owned(), Status::Conditional, 1),
                OwnedElementRef::new_unchecked(3, "1225".to_owned(), Status::Conditional, 1),
            ],
        ),
    )
    .build();
```

Declaring a composite's components (`ElementRef::composite`,
`OwnedElementRef::with_components`):

- enables the mandatory-component check;
- caps the composite's arity: more components than declared is `E013` (fewer is
  normal);
- makes components addressable by identifier ([below](#code-addressed-element-access)).

An `expected_components` hook wins for its element: it is an exact count, where
declared components are an upper bound.

`DirectoryValidator` checks segments one at a time: presence, arity, length,
representation, repetition. Message grammar (segment-group order and counts) is
the job of [`GroupSchema`](@/docs/segment-groups.md#placements-and-findings-groupschema)
and `ProfileRulePack`.

---

## Data element representations

Attach a directory representation with `with_repr` and `DirectoryValidator`
checks it:

```rust
use edifact::{ComponentRef, ElementRef, Repr, SegmentDefinition, Status};

static C507: &[ComponentRef] = &[
    ComponentRef::new(1, "2005", Status::Mandatory).with_repr(Repr::an_up_to(3)),
    ComponentRef::new(2, "2380", Status::Conditional).with_repr(Repr::an_up_to(35)),
    ComponentRef::new(3, "2379", Status::Conditional).with_repr(Repr::an_up_to(3)),
];
static DTM_ELEMENTS: &[ElementRef] =
    &[ElementRef::composite(1, "C507", Status::Mandatory, 1, C507)];
static DTM: SegmentDefinition = SegmentDefinition::new("DTM", "Date/time/period", DTM_ELEMENTS);
```

| Violation | Code | `CONTRL` |
|---|---|---|
| Wrong character class | [`E048`](@/docs/error-reference.md#e048-invalidcharactertype) | 37 |
| Longer than the maximum | [`E049`](@/docs/error-reference.md#e049-dataelementtoolong) | 39 |
| Shorter than a fixed length | [`E050`](@/docs/error-reference.md#e050-dataelementtooshort) | 40 |
| More repetitions than `max_repeat` | [`E047`](@/docs/error-reference.md#e047-toomanyrepetitions) | 35 |
| Insignificant leading zero / trailing space (warning) | [`E053`](@/docs/error-reference.md#e053-insignificantcharacters) | — |

A position without a representation is not checked.

| Rule (ISO 9735-1) | Effect |
|---|---|
| Length counts characters, not bytes (§6); the release character is excluded (§5) | A `UNOC` `ü` is one character |
| A numeric length excludes sign, decimal mark and exponent (§10) | `-123.45` fits `n..5` |
| `n` admits no space or plus sign, and a decimal mark needs a digit after it (§10) | `2`, `2.00`, `.5`, `-3E4` pass; `1.`, `.`, `+1`, `1 2` fail |
| Leading zeroes and trailing spaces are suppressed in variable-length values (§9.2) | `00.5` is `E053`; `0.5` and fixed-length `n3` `007` pass |

### Service tables

`edifact::service` declares the representations of ISO 9735-10:2002 §5 for
syntax versions 1–4. `UNZ+abc+IC1'` (DE 0036 is `n..6`) and a 20-character
`UNB` DE 0020 (`an..14`) are rejected with no directory: by the envelope layer,
and by `DirectoryValidator` with `service::lookup`, which adds `E053`.

A position whose form differs between versions declares both; the checker picks
by `UNB` S001 DE 0002 (versions 1 and 2 use the version 3 form):

```rust
use edifact::{ComponentRef, Repr, Status};

const DATE: ComponentRef = ComponentRef::new(1, "0017", Status::Mandatory)
    .with_repr_by_syntax_version(Repr::n(6), Repr::n(8));
```

Without a known version (a bare message window, or DE 0002 outside `1`–`4`)
both forms pass; a seven-digit date still fails.

---

## Auditing a hand-written layout

A hand-written `SegmentDefinition` that disagrees with the wire makes
`value_by_code` return the wrong component, and nothing inside the program can
notice. `SegmentLayout::audit` checks the definition against real messages:

```rust
use edifact::{ComponentRef, ElementRef, SegmentDefinition, SegmentLayout, Status, from_bytes};

// A hand-authored C507 that stops one component short of the directory.
static C507: &[ComponentRef] = &[
    ComponentRef::new(1, "2005", Status::Mandatory),
    ComponentRef::new(2, "2380", Status::Conditional),
];
static DTM_ELEMENTS: &[ElementRef] =
    &[ElementRef::composite(1, "C507", Status::Mandatory, 1, C507)];
static DTM: SegmentDefinition = SegmentDefinition::new("DTM", "Date/time/period", DTM_ELEMENTS);

let corpus: Vec<_> = from_bytes(b"DTM+137:20260101:102'").collect::<Result<Vec<_>, _>>()?;
let audit = DTM.audit(&corpus);

// The format qualifier `102` has nowhere to go.
assert!(audit.has_contradictions());
# Ok::<(), edifact::EdifactError>(())
```

| Finding | Means |
|---|---|
| `UndeclaredElement` / `UndeclaredComponent` | The wire carries a value the layout has no slot for: the layout is wrong. |
| `MandatoryNeverPopulated` | A mandatory slot is empty everywhere: status or position is wrong. |
| `NeverObserved` | No fixture reaches this slot: the corpus cannot confirm it. |

`has_contradictions` excludes `NeverObserved`; `unconfirmed()` lists those
positions so they can be checked by hand or covered by a fixture:

```rust
# use edifact::{ComponentRef, ElementRef, SegmentDefinition, SegmentLayout, Status, from_bytes};
# static C507: &[ComponentRef] = &[
#     ComponentRef::new(1, "2005", Status::Mandatory),
#     ComponentRef::new(2, "2380", Status::Conditional),
#     ComponentRef::new(3, "2379", Status::Conditional),
# ];
# static DTM_ELEMENTS: &[ElementRef] =
#     &[ElementRef::composite(1, "C507", Status::Mandatory, 1, C507)];
# static DTM: SegmentDefinition = SegmentDefinition::new("DTM", "Date/time/period", DTM_ELEMENTS);
// No fixture carries a format qualifier.
let corpus: Vec<_> = from_bytes(b"DTM+137:20260101'").collect::<Result<Vec<_>, _>>()?;
let audit = DTM.audit(&corpus);

assert!(!audit.has_contradictions());          // nothing is disproved …
let pending: Vec<&str> = audit.unconfirmed().map(|s| s.data_element.as_str()).collect();
assert_eq!(pending, ["2379"]);                 // … but 2379 is unverified
# Ok::<(), edifact::EdifactError>(())
```

In a test, assert `!audit.has_contradictions()` together with
`audit.segments_examined() > 0`: an audit over a corpus without the segment
proves nothing. Findings are deduplicated per position. `LayoutAudit`
implements `Display` for review output.

A mandatory component of an absent conditional composite is required only when
the composite is present (§8.7). `UNB` S005 DE 0022 without S005 is
`NeverObserved`, not a contradiction.

### Auditing a whole directory

`audit_directory` audits every definition whose tag the corpus contains:

```rust
use edifact::{audit_directory, from_bytes, service};

let corpus: Vec<_> = from_bytes(
    b"UNB+UNOC:3+S+R+260101:0900+IC1'UNH+M1+ORDERS:D:96A:UN'UNT+2+M1'UNZ+1+IC1'",
)
.collect::<Result<Vec<_>, _>>()?;

for audit in audit_directory(service::lookup, &corpus) {
    assert!(!audit.has_contradictions(), "{audit}");
}
# Ok::<(), edifact::EdifactError>(())
```

For a tag the corpus lacks, call `SegmentLayout::audit` directly.

---

## Code-addressed element access

A transposed positional index (`seg.element_str(4)`) reads the wrong data
element and still validates clean. With a `SegmentLayout` (`SegmentDefinition`
or `OwnedSegmentDef`), address data by its UN/EDIFACT identifier instead; a
wrong identifier is an error:

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
static NAD: SegmentDefinition =
    SegmentDefinition::new("NAD", "Name and address", NAD_ELEMENTS);

let segments: Vec<_> = from_bytes(b"NAD+MS+9900112233445::293'").collect::<Result<Vec<_>, _>>()?;
let nad = &segments[0];

assert_eq!(nad.value_by_code(&NAD, "3039")?, Some("9900112233445"));
assert_eq!(nad.value_by_code(&NAD, "3055")?, Some("293"));

// DE 2380 belongs to DTM, not NAD.
assert!(nad.value_by_code(&NAD, "2380").is_err());
# Ok::<(), edifact::EdifactError>(())
```

| Method | On | Returns |
|---|---|---|
| `value_by_code(layout, de)` | `Segment` | `Result<Option<&str>, EdifactError>` |
| `span_by_code(layout, de)` | `Segment` | `Result<Option<Span>, EdifactError>` — for `ValidationIssue::with_span` |
| `element_by_code(layout, de)` | `Segment` | `Result<Option<&Element>, EdifactError>` — the enclosing composite when `de` is a component |
| `SegmentLayout::resolve_code(de)` | `SegmentDefinition`, `OwnedSegmentDef` | `Result<ElementPath, EdifactError>` — resolve once, read many with `value_at` / `span_at` |

Failures: `UnknownDataElement` (`E033`), `AmbiguousDataElement` (`E034`) when a
directory repeats a code, `SegmentLayoutMismatch` (`E035`) when the layout's tag
differs from the segment's. The derive resolves the same addressing at compile
time: [Typed Derive](@/docs/typed-derive.md#element-by-identifier).

---

## Next steps

- [Segment Groups](@/docs/segment-groups.md) — group trees, placements, group rules
- [Profile Packs](@/docs/profile-packs.md) — composable business-rule bundles
- [Error Reference](@/docs/error-reference.md) — stable codes for `ValidationIssue::error_code`
- [Diagnostics](@/docs/diagnostics.md) — human-readable error rendering
