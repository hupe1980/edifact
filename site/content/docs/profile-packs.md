+++
title = "Profile Packs"
description = "Author, compose, merge, and filter business-rule packs with ProfileRulePack, including group-scoped rules."
weight = 80
+++

A `ProfileRulePack` is a named set of business rules — a MIG, an implementation
guide, a partner's requirements — plugged into a `ValidationContext`. Each rule
receives a message's segments and pushes a `ValidationIssue` per violation;
passing pushes nothing. Packs are built from public APIs only, scoped to message
types and releases, composed, and filtered by rule ID.

---

## Creating a pack

```rust
use edifact::{ProfileRulePack, SegmentSliceExt, ValidationIssue, ValidationSeverity};

let pack = ProfileRulePack::new("ORDERS-RULES")
    .for_message_type("ORDERS")
    .with_rule_fn(|segments, issues| {
        let Some(code) = segments.first_tag("BGM").and_then(|s| s.element_str(0)) else {
            return;
        };
        // Only 220 (order) and 231 (purchase order response) are accepted.
        if !matches!(code, "220" | "231") {
            issues.push(
                ValidationIssue::new(
                    ValidationSeverity::Error,
                    format!("unsupported BGM document code '{code}'"),
                )
                .with_rule_id("ORDERS-BGM-P001")
                .with_segment("BGM")
                .with_element_index(0)
                .with_suggestion("Use document code 220 or 231"),
            );
        }
    })
    .with_rule_fn(|segments, issues| {
        // A buyer NAD is mandatory.
        if segments.qualified("NAD", "BY").is_none() {
            issues.push(
                ValidationIssue::new(ValidationSeverity::Error, "buyer NAD+BY is required")
                    .with_rule_id("ORDERS-NAD-P001")
                    .with_segment("NAD")
                    .with_element_index(0),
            );
        }
    });
```

---

## Plugging into `ValidationContext`

```rust
use edifact::{ValidationContext, from_bytes};

# use edifact::{ProfileRulePack, ValidationIssue, ValidationSeverity};
# let pack = ProfileRulePack::new("ORDERS-RULES").for_message_type("ORDERS");
let segs: Vec<_> = from_bytes(b"UNH+1+ORDERS:D:96A:UN'BGM+220+PO-4711+9'UNT+3+1'")
    .collect::<Result<_, _>>()?;

let report = ValidationContext::builder()
    .with_profile_pack(pack)
    .build()
    .validate(&segs);

println!("{} error(s)", report.errors().len());
# Ok::<(), edifact::EdifactError>(())
```

---

## Merging packs

### `extend_from` — base rules first, then your own

```rust
use edifact::{ProfileRulePack, ValidationIssue, ValidationSeverity};

let base_pack = ProfileRulePack::new("ORDERS-BASE")
    .for_message_type("ORDERS")
    .with_named_rule_fn("BASE-BGM", |_segs, _issues| {
        // shared baseline rules …
    });

let partner_pack = ProfileRulePack::new("ORDERS-ACME")
    .for_message_type("ORDERS")
    .with_rule_fn(|_segs, _issues| {
        // ACME-specific rules …
    })
    .extend_from(&base_pack)?;  // base rules run first
# Ok::<(), edifact::EdifactError>(())
```

### `merge_with_override` — replace rules by ID

A delta pack replaces the base rules with the same rule ID:

```rust
use edifact::{ProfileRulePack, ValidationIssue, ValidationSeverity};

let base = ProfileRulePack::new("ORDERS-5.4")
    .for_message_type("ORDERS")
    .with_named_rule_fn("PROFILE-4711-BGM-M", |_segs, _issues| {
        // 5.4 BGM rule …
    });

let delta = ProfileRulePack::new("ORDERS-5.5-delta")
    .with_named_rule_fn("PROFILE-4711-BGM-M", |_segs, _issues| {
        // updated 5.5 BGM rule — replaces the 5.4 version
    });

let result = base.merge_with_override(delta)?;
assert_eq!(result.rule_count(), 1);
# Ok::<(), edifact::EdifactError>(())
```

Both return `EdifactError::IncompatibleReleaseScopes` when the packs are bound
to different releases (`for_release`).

---

## Scoping

`for_message_type` restricts a pack to messages whose `UNH` S009 DE 0065 matches;
call it once per type. `for_release` restricts it to an association assigned
code (DE 0057). An unscoped pack runs for every message.

```rust
# use edifact::ProfileRulePack;
let multi_pack = ProfileRulePack::new("TRADE-DOCS")
    .for_message_type("ORDERS")
    .for_message_type("ORDRSP")
    .for_message_type("INVOIC")
    .with_rule_fn(|_segs, _issues| {});
```

---

## Filtering reports by rule ID prefix

Give rules stable, namespaced IDs and filter on them:

```rust
# use edifact::{ProfileRulePack, ValidationContext, ValidationIssue, ValidationSeverity, from_bytes};
let segs: Vec<_> = from_bytes(b"UNH+1+ORDERS:D:96A:UN'BGM+220+PO-4711+9'UNT+3+1'")
    .collect::<Result<_, _>>()?;

let pack = ProfileRulePack::new("ORDERS")
    .for_message_type("ORDERS")
    .with_rule_fn(|_segs, issues| {
        issues.push(
            ValidationIssue::new(ValidationSeverity::Warning, "demo warning")
                .with_rule_id("ORDERS-DOC-P001"),
        );
    })
    .with_rule_fn(|_segs, issues| {
        issues.push(
            ValidationIssue::new(ValidationSeverity::Info, "demo info")
                .with_rule_id("ORDERS-REF-P001"),
        );
    });

let report = ValidationContext::builder()
    .with_profile_pack(pack)
    .build()
    .validate(&segs);

assert_eq!(report.total_issues(), 2);
assert_eq!(report.filter_by_rule_prefix("ORDERS-DOC-").total_issues(), 1);
assert_eq!(report.issues_for_rule_id("ORDERS-REF-P001").count(), 1);
# Ok::<(), edifact::EdifactError>(())
```

---

## Mapping violations to application errors

Map rule IDs to your own error type:

```rust
use edifact::{
    ProfileRulePack, SegmentSliceExt, ValidationContext, ValidationIssue, ValidationSeverity,
    from_bytes,
};

#[derive(Debug)]
enum TradeError {
    UnsupportedDocumentCode(String),
    MissingBuyerNad,
}

fn validate_orders(input: &[u8]) -> Result<(), Vec<TradeError>> {
    let segs: Vec<_> = from_bytes(input)
        .collect::<Result<_, _>>()
        .map_err(|_| vec![])?;

    let pack = ProfileRulePack::new("ORDERS")
        .for_message_type("ORDERS")
        .with_rule_fn(|segs, issues| {
            let Some(code) = segs.first_tag("BGM").and_then(|s| s.element_str(0)) else {
                return;
            };
            if !matches!(code, "220" | "231") {
                issues.push(
                    ValidationIssue::new(ValidationSeverity::Error, format!("code {code}"))
                        .with_rule_id("ORDERS-DOC-P001"),
                );
            }
        })
        .with_rule_fn(|segs, issues| {
            if segs.qualified("NAD", "BY").is_none() {
                issues.push(
                    ValidationIssue::new(ValidationSeverity::Error, "missing NAD+BY")
                        .with_rule_id("ORDERS-NAD-P001"),
                );
            }
        });

    let report = ValidationContext::builder()
        .with_profile_pack(pack)
        .build()
        .validate(&segs);

    if report.is_valid() {
        return Ok(());
    }

    let errors: Vec<TradeError> = report
        .errors()
        .iter()
        .filter_map(|issue| match issue.rule_id.as_deref() {
            Some("ORDERS-DOC-P001") => {
                Some(TradeError::UnsupportedDocumentCode(issue.message.clone()))
            }
            Some("ORDERS-NAD-P001") => Some(TradeError::MissingBuyerNad),
            _ => None,
        })
        .collect();

    Err(errors)
}
```

See [`cookbook_profile_error_mapping.rs`](https://github.com/hupe1980/edifact/tree/main/crates/edifact/examples/cookbook_profile_error_mapping.rs)
combines this with a custom `Validator`.

---

## Implementing `ProfileRule` as a struct

For a rule with state, implement `ProfileRule`:

```rust
use edifact::{
    ProfileRule, ProfileRulePack, Segment, SegmentSliceExt, ValidationIssue,
    ValidationRuleContext, ValidationSeverity,
};

struct AllowedCodesRule {
    allowed: Vec<String>,
}

impl ProfileRule for AllowedCodesRule {
    fn evaluate(
        &self,
        segments: &[Segment<'_>],
        _context: &ValidationRuleContext<'_>,
        issues: &mut Vec<ValidationIssue>,
    ) {
        let Some(code) = segments.first_tag("BGM").and_then(|s| s.element_str(0)) else {
            return;
        };
        if !self.allowed.iter().any(|a| a == code) {
            issues.push(
                ValidationIssue::new(
                    ValidationSeverity::Error,
                    format!("code '{code}' is not in the allowed list"),
                )
                .with_rule_id("ORDERS-ALLOWED-CODE"),
            );
        }
    }
}

let pack = ProfileRulePack::new("ORDERS")
    .with_rule(AllowedCodesRule {
        allowed: vec!["220".into(), "231".into()],
    });
```

`ProfileRule` is `Send + Sync`, so a pack can be shared across threads.

---

## Group-scoped rules

A group-scoped rule fires once per occurrence of a named segment group and sees
only that occurrence's segments — for invariants such as "every `SG5` holds a
`LIN`". The schema is a `GroupDef` tree; see [Segment Groups](@/docs/segment-groups.md).

### Helpers

```rust
use edifact::{ProfileRulePack, group::GroupDef};

// Schema: SG5 contains LIN and zero or more SG6 children.
static SG6: &[GroupDef] = &[GroupDef::new("SG6", "RFF")];
static ORDERS_SCHEMA: &[GroupDef] = &[GroupDef::with_children("SG5", "LIN", SG6)];

let pack = ProfileRulePack::new("ORDERS-GROUPS")
    .for_message_type("ORDERS")
    // Require LIN in every SG5 occurrence:
    .require_segment_in_group("SG5", "LIN", "ORDERS-SG5-LIN-M")
    // Forbid a legacy segment in SG5:
    .forbid_segment_in_group("SG5", "OLD", "ORDERS-SG5-OLD-F")
    // Require qualifier "1" on LIN within SG5 (element 0, component 0):
    .require_qualifier_in_group("SG5", "LIN", 0, 0, "1", "ORDERS-SG5-LIN-Q1");
```

Build the tree with `group::group_segments_indexed` and run
`ctx.validate_grouped(&tree, &segs)`.

### Custom group rules

`with_scoped_group_rule_fn(group, rule_id, closure)` takes the group occurrence,
its segments, the rule context and the issue list:

```rust
use edifact::{
    ProfileRulePack, ValidationIssue, ValidationSeverity,
    group::SegmentGroupIndexed,
    ValidationRuleContext,
    Segment,
};

let pack = ProfileRulePack::new("ORDERS-GROUPS")
    .for_message_type("ORDERS")
    .with_scoped_group_rule_fn(
        "SG5",              // fires only inside SG5 occurrences
        "ORDERS-SG5-QTY-M", // stable rule id
        |_group: &SegmentGroupIndexed<'_>,
         segs: &[Segment<'_>],
         _ctx: &ValidationRuleContext<'_>,
         issues: &mut Vec<ValidationIssue>| {
            if !segs.iter().any(|s| s.tag == "QTY") {
                issues.push(
                    ValidationIssue::new(
                        ValidationSeverity::Error,
                        "every SG5 must contain a QTY segment",
                    )
                    .with_rule_id("ORDERS-SG5-QTY-M")
                    .with_segment_group("SG5"),
                );
            }
        },
    );
```

Every issue a group rule emits is stamped with the `segment_group` and the
`group_path` of the occurrence — `SG4[0]/SG8[1]`, zero-based and counted per
group name among siblings — unless the rule set them. A rule at the tree root
gets no path.

---

## What the pack stamps on its issues

A pack stamps its name on every issue it emits without a `pack` set, and a
`ValidationContext` stamps the registration layer (`Profile` for packs) on every
issue without a `layer`. The helper rules set their own `kind`:

| Helper | Kind |
|---|---|
| `require_segment`, `require_segment_in_group` | `FindingKind::Missing` |
| `forbid_segment`, `forbid_segment_in_group` | `FindingKind::NotPermitted` |
| `require_qualifier`, `require_qualifier_in_group` | `FindingKind::Code` |

`forbid_segment*` and `require_qualifier*` also set `segment_occurrence` and, when
the slice they see holds the segment's `UNH`, `segment_position` (`UNH` = 1).
Findings of packs registered side by side are told apart by `pack()`, missing
from not-permitted by `kind()`:

```rust
use edifact::{FindingKind, ProfileRulePack, ValidationContext, from_bytes};

let segs: Vec<_> = from_bytes(b"UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+3+1'")
    .collect::<Result<_, _>>()?;
let report = ValidationContext::builder()
    .with_profile_pack(ProfileRulePack::new("MIG-ORDERS").require_segment("DTM", "MIG-DTM-M"))
    .with_profile_pack(ProfileRulePack::new("AHB-ORDERS").forbid_segment("BGM", "AHB-BGM-X"))
    .build()
    .validate(&segs);

let mig: Vec<_> = report.iter_issues().filter(|i| i.pack() == Some("MIG-ORDERS")).collect();
assert_eq!(mig[0].kind(), Some(FindingKind::Missing));
let ahb: Vec<_> = report.iter_issues().filter(|i| i.pack() == Some("AHB-ORDERS")).collect();
assert_eq!(ahb[0].kind(), Some(FindingKind::NotPermitted));
assert_eq!(ahb[0].segment_position(), Some(2));
# Ok::<(), edifact::EdifactError>(())
```

The name is that of the pack that *runs* the rule: rules taken over with
`extend_from` or `merge_with_override` carry the combined pack's name. Register
packs separately when their findings must stay apart.

---

## Capping a noisy rule

A rule that sweeps every segment can emit one issue per segment, and a group
rule fires once per occurrence. `with_max_issues_per_rule` caps it:

```rust
use edifact::{ProfileRulePack, ValidationContext, ValidationIssue, ValidationSeverity, from_bytes};

let pack = ProfileRulePack::new("NOISY")
    .with_max_issues_per_rule(2)
    .with_rule_fn(|segments, issues| {
        for segment in segments {
            issues.push(ValidationIssue::new(
                ValidationSeverity::Warning,
                format!("saw {}", segment.tag()),
            ));
        }
    });

let segments: Vec<_> = from_bytes(b"BGM+1'DTM+2'RFF+3'NAD+4'")
    .collect::<Result<Vec<_>, _>>()?;
let report = ValidationContext::builder()
    .with_profile_pack(pack)
    .build()
    .validate(&segments);

assert_eq!(report.total_issues(), 2); // four segments, capped at two
# Ok::<(), edifact::EdifactError>(())
```

The cap is per rule per call, not per group occurrence: a group rule's budget
covers the whole tree walk, and once spent the rule is not called again on that
pass.

---

## Pack introspection

```rust
# use edifact::ProfileRulePack;
let pack = ProfileRulePack::new("MY-PACK")
    .for_message_type("ORDERS")
    .with_rule_fn(|_segs, _issues| {})
    .with_rule_fn(|_segs, _issues| {})
    .require_segment_in_group("SG5", "LIN", "SG5-LIN-M");

assert_eq!(pack.name(), "MY-PACK");
assert_eq!(pack.rule_count(), 2);
assert_eq!(pack.group_rule_count(), 1);
assert_eq!(pack.message_types().collect::<Vec<_>>(), ["ORDERS"]);
```

---

## Best practices

1. **Namespace rule IDs** with the pack: `ORDERS-DOC-P001`, not `P001`.
2. **Scope packs** to the message types their logic assumes.
3. **One rule per concern**: reports and filters stay precise.
4. **Use `with_rule_fn`**; `with_contextual_rule_fn` only when the rule needs
   the `ValidationRuleContext` (message reference, type, metadata).
5. **Cap sweeping rules** with `with_max_issues_per_rule`.

---

## Next steps

- [Validation](@/docs/validation.md) — multi-layer `ValidationContext` and the `Validator` trait
- [Streaming](@/docs/streaming.md) — progressive per-window validation over reader streams
- [Diagnostics](@/docs/diagnostics.md) — human-friendly rendering of `ValidationReport`
