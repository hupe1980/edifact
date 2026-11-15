+++
title = "Diagnostics"
description = "Render EDIFACT errors and validation findings with miette: stable codes, help text, and source spans underlined."
weight = 90
+++

The opt-in `diagnostics` feature implements
[`miette::Diagnostic`](https://docs.rs/miette) for the error and validation
types. Parsing is unchanged.

```bash
cargo add edifact --features diagnostics
cargo add miette --features fancy   # to render; edifact does not re-export it
```

## What each type provides

| Type | `code` | `help` | Labels / related |
|---|---|---|---|
| `EdifactError` | stable code (`E001`…) | remediation hint, where one exists | none: the offset or span is in the message and the fields |
| `ValidationIssue` | error code, else rule id | the issue's suggestion | one label at its `span`: kind and group path (`missing at SG4[0]/SG8[1]/RFF`) |
| `ValidationReport` | `VALIDATION` | error, warning and info counts | every issue via `related()`: errors, then warnings, then infos |

Severity follows the validation pipeline: Error and Critical render as errors,
Warning as warnings, Info as advice.

## Errors from `main`

`EdifactError` converts into `miette::Report` with `?`:

```rust
fn main() -> miette::Result<()> {
    let input = b"UNH+1+ORDERS:D:11A:UN'BGM+220+PO-4711+9'UNT+3+1'";
    let _segs: Vec<_> = edifact::from_bytes(input).collect::<Result<_, _>>()?;
    Ok(())
}
```

Or wrap an error explicitly:

```rust
fn report(input: &[u8]) {
    if let Err(err) = edifact::from_bytes(input).collect::<Result<Vec<_>, _>>() {
        eprintln!("{:?}", miette::Report::new(err));
    }
}
```

## Underlining findings in the input

Attach the source to a `ValidationReport`; each issue with a span is
underlined:

```rust
use edifact::{ProfileRulePack, ValidationContext, from_bytes};

let input = "UNH+1+ORDERS:D:96A:UN'BGM+220'DTM+137:20240101:102'UNT+4+1'";
let segs: Vec<_> = from_bytes(input.as_bytes()).collect::<Result<_, _>>()?;
let report = ValidationContext::builder()
    .with_profile_pack(ProfileRulePack::new("AHB").forbid_segment("DTM", "AHB-DTM-X"))
    .build()
    .validate(&segs);

eprintln!("{:?}", miette::Report::new(report).with_source_code(input));
# Ok::<(), edifact::EdifactError>(())
```

```text
Error: AHB-DTM-X

  × [ERROR] segment DTM must not appear
   ╭────
 1 │ UNH+1+ORDERS:D:96A:UN'BGM+220'DTM+137:20240101:102'UNT+4+1'
   ·                               ──────────┬──────────
   ·                                         ╰── not_permitted at DTM
   ╰────
```

## Deterministic text for snapshots

`ValidationReport::render_deterministic()` (also its `Display`) needs no
feature and gives the same text for the same report:

```rust
use edifact::{ProfileRulePack, ValidationContext, from_bytes};

let segs: Vec<_> = from_bytes(b"BGM+220+PO-4711+9'DTM+137:20240101:102'").collect::<Result<_, _>>()?;
let report = ValidationContext::builder()
    .with_profile_pack(ProfileRulePack::new("AHB").forbid_segment("DTM", "AHB-DTM-X"))
    .build()
    .validate(&segs);
assert!(report.render_deterministic().contains("[rule=AHB-DTM-X]"));
# Ok::<(), edifact::EdifactError>(())
```

Each line is the message followed by the tokens that are set, in this order:
`[CODE]`, `[segment=…]`, `[rule=…]`, `[element=…]`, `[component=…]`,
`[occurrence=…]`, `[position=…]`, `[path=…]`, `[layer=…]`, `[kind=…]`,
`[pack=…]`, `[span=…]`, `[hint=…]`. Within a severity bucket, lines sort by
span start, tag, rule id, element and component index, code and message, then
segment position, occurrence, layer, kind, pack and group path.

## Text from the input is escaped

Input-derived values — a reference, a qualifier, a tag — appear in error text,
issue text, `render_deterministic()` and the `miette` help and labels with
control characters escaped (`\r`, `\n`, `\t`, `\0`, otherwise `\u{1b}` style),
as are U+2028/U+2029 and the bidirectional formatting characters (U+061C,
U+200E/F, U+202A–E, U+2066–9). One diagnostic is one line: a sender cannot
forge log lines, reorder text or drive a terminal. The fields keep the raw
values.

```rust
use edifact::{EdifactError, Span};

let err = EdifactError::EmptyMessage { message_ref: "1\r\nFAKE".into(), span: Span::new(0, 9) };
assert_eq!(err.to_string(), r"message 1\r\nFAKE has no segments between UNH and UNT");
```

## Example

```bash
cargo run -p edifact --example cookbook_diagnostics --features diagnostics
```

[`cookbook_diagnostics.rs`](https://github.com/hupe1980/edifact/tree/main/crates/edifact/examples/cookbook_diagnostics.rs)
raises a code-value error from a custom validator and renders it with
`miette::Report`.

## Next steps

- [Error Reference](@/docs/error-reference.md) — every error code
- [Validation](@/docs/validation.md) — `ValidationReport`
- [Getting Started](@/docs/getting-started.md) — feature flags
