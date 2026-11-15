+++
title = "Segment Groups"
description = "GroupDef schemas, the indexed group tree, ordered groups with discriminators, GroupSchema placements and findings, and group-scoped validation."
weight = 75
+++

A segment group (`SG5`, `SG12`, …) is a run of segments opened by a trigger
tag. `edifact::group` builds a tree of group instances from a flat segment
slice, places every segment, and lets profile rules run once per occurrence.

| Type | Role |
|---|---|
| `GroupDef` | A group: name, trigger, child groups; with the builder also member places, `min`/`max`, order |
| `SegmentPlace` | A segment position in a group: tag, `min`/`max`, `id`, `Discriminator`s |
| `group_segments_indexed` | Builds a `SegmentGroupIndexed` tree of index ranges (no clones) |
| `GroupSchema` | A checked schema; `resolve` returns a `GroupResolution` with placements and findings |
| `PlacementValidator` | Runs a `GroupSchema` inside a `ValidationContext` |

---

## Group-aware validation

`validate_grouped` runs the flat pass, then fires `ProfileRulePack` group rules
once per group occurrence:

```rust
use edifact::{
    ValidationContext, ProfileRulePack,
    group::{GroupDef, group_segments_indexed},
    from_bytes,
};

// SG5 starts at LIN and contains SG6, which starts at QTY.
static SG6: &[GroupDef] = &[GroupDef::new("SG6", "QTY")];
static SCHEMA: &[GroupDef] = &[GroupDef::with_children("SG5", "LIN", SG6)];

let segs: Vec<_> = from_bytes(
    b"UNH+1+ORDERS:D:96A:UN'\
      LIN+1'QTY+21:10'\
      LIN+2'\
      UNT+5+1'"
).collect::<Result<_, _>>()?;

let tree = group_segments_indexed(&segs, SCHEMA, "ROOT");

let pack = ProfileRulePack::new("ORDERS")
    .for_message_type("ORDERS")
    .require_segment_in_group("SG5", "QTY", "ORDERS-SG5-QTY-M");

let report = ValidationContext::builder()
    .with_profile_pack(pack)
    .build()
    .validate_grouped(&tree, &segs);

// The second SG5 (LIN+2) has no QTY; the issue names the occurrence.
let issue = &report.errors()[0];
assert_eq!(issue.segment_group(), Some("SG5"));
assert_eq!(issue.group_path().map(|p| p.to_string()).as_deref(), Some("SG5[1]"));
# Ok::<(), edifact::EdifactError>(())
```

`render_deterministic` prints the path with the tag, e.g.
`[path=SG4[0]/SG8[1]/RFF]`. Segments from a reader go into the same call, and
`.result()` works as for the flat pass. Writing group rules:
[Profile Packs — Group-scoped rules](@/docs/profile-packs.md#group-scoped-rules).

### Schemas built at runtime

`GroupDef::new` and `GroupDef::with_children` are `const` over `'static` data.
A schema read from a MIG at startup uses `GroupDef::builder`, which owns its
strings behind `Arc` (cloning a group is O(1)):

```rust
use edifact::group::{GroupDef, SegmentPlace, group_segments_indexed};
use edifact::from_bytes;

// Stand-in for names read from a MIG file.
let names: Vec<String> = vec!["SG5".into(), "LIN".into()];
let schema = vec![GroupDef::builder(names[0].as_str(), SegmentPlace::owned(names[1].as_str())).build()?];
drop(names); // the schema owns what it needs

let segs: Vec<_> = from_bytes(b"UNH+1+ORDERS:D:96A:UN'LIN+1'UNT+3+1'")
    .collect::<Result<_, _>>()?;
let tree = group_segments_indexed(&segs, &schema, "ROOT");

assert_eq!(tree.children[0].definition, "SG5");
# Ok::<(), edifact::EdifactError>(())
```

In a `static`, a nested child slice must be a named `static` of its own
(`static SG6` above). Written inline it fails with `E0493`, because `GroupDef`
can own shared data.

### Open and ordered groups

An **open** group declares only its trigger and child groups; every segment
that triggers nothing is a direct segment of it. The `const` constructors build
open groups.

An **ordered** group is built with `GroupDef::builder` and declares member
segments with `.segment(..)` or is marked `.ordered()`. Its entries are walked
in guide order: trigger, then member places and child groups. Each
`SegmentPlace` and group can carry:

| Option | Meaning |
|---|---|
| `min` / `max` | Occurrences per parent instance (default 0 / unbounded) |
| `id` | E.g. the MIG segment number; returned with placements and findings |
| `Discriminator::one_of(element, component, codes)` | Matches only one of the codes; an absent value does not match |
| `.optional()` | An absent value does not rule the place out |
| `.accept_unclaimed()` | A code no place of the same tag admits still matches (a code fault, not a misplacement) |
| `Discriminator::not_used(element, component)` | A value there rules the place out |

Discriminator positions are zero-based and read the element's first
repetition; all discriminators on a place must match.

A segment that fits no entry of an ordered group is **unplaced**: recorded on
its instance (`SegmentGroupIndexed::unplaced`, excluded from
`direct_segment_indices`) with a reason — `OutOfOrder` (fits a place already
passed), `NotHere` (fits a place elsewhere in the schema) or `Unknown`. It is
never attached to a wrong group.

This matters for shared tags. With `SG4(IDE){SG6(RFF), SG8(SEQ)}` all open, the
`RFF+Z19` that belongs to `SG8` opens a second `SG6`. Declaring `SG8`'s members
fixes it:

```rust
use edifact::group::{GroupDef, SegmentPlace, group_segments_indexed};

let sg8 = GroupDef::builder("SG8", SegmentPlace::new("SEQ"))
    .segment(SegmentPlace::new("RFF"))
    .segment(SegmentPlace::new("CCI"))
    .build()?;
let schema = [GroupDef::builder("SG4", SegmentPlace::new("IDE"))
    .group(GroupDef::new("SG6", "RFF"))
    .group(sg8)
    .build()?];

let segments: Vec<_> = edifact::from_bytes(b"IDE+24+X'RFF+Z13:1'SEQ+Z01'RFF+Z19:ABC'CCI+Z30'")
    .collect::<Result<_, _>>()?;
let tree = group_segments_indexed(&segments, &schema, "ROOT");
let sg4 = &tree.children[0];
assert_eq!(
    sg4.children.iter().map(|g| (g.definition, g.total_span.clone())).collect::<Vec<_>>(),
    [("SG6", 1..2), ("SG8", 2..5)],
);
# Ok::<(), edifact::EdifactError>(())
```

### Placements and findings: `GroupSchema`

`GroupSchema::new(root, groups)` (open root) or `GroupSchema::ordered(root,
entries)` (ordered root; declare `UNT` as its last entry, or it is unplaced)
checks a schema once. Tags must be three upper-case letters, `min <= max`,
`max >= 1`, `one_of` lists a code, at most 64 nested groups; otherwise
`E071 InvalidGroupSchema`.

`resolve(&segments)` returns a `GroupResolution`: the tree, `placement(i)`
(place, group and occurrence of segment `i`), `unplaced()`, `group_path(i)`,
and `findings(&segments)`:

| Finding | Code | Kind |
|---|---|---|
| unplaced segment | `E068 SegmentNotPlaced` | `placement` |
| a place beyond its `max` | `E069 TooManySegmentOccurrences` | `count` |
| a group beyond its `max` | `E070 TooManyGroupOccurrences` | `count` |
| a place or group below its `min` | `E015 MissingSegment` | `missing` |

Every finding has layer `structure`, the instance's `group_path`, the segment's
span and, inside a message, its `segment_position`; a place `id` is in the
context under `place`. `PlacementValidator` runs the same resolution inside a
`ValidationContext`:

```rust
use std::sync::Arc;
use edifact::group::{Discriminator, Entry, GroupDef, GroupSchema, SegmentPlace};
use edifact::{FindingKind, PlacementValidator, ValidationContext, ValidationLayer};

let malo = |code: &str, id: &str| {
    GroupDef::builder("SG5", SegmentPlace::new("LOC").with_id(id).when(Discriminator::one_of(0, 0, [code])))
        .ordered()
        .build()
};
let sg4 = GroupDef::builder("SG4", SegmentPlace::new("IDE"))
    .group(malo("Z16", "00047")?)
    .group(malo("Z22", "00048")?)
    .ordered()
    .build()?;
let schema = GroupSchema::ordered(
    "UTILMD",
    [
        Entry::Segment(SegmentPlace::new("UNH").with_min(1).with_max(1)),
        Entry::Group(sg4),
        Entry::Segment(SegmentPlace::new("UNT").with_min(1).with_max(1)),
    ],
)?;

let segments: Vec<_> = edifact::from_bytes(
    b"UNH+1+UTILMD:D:11A:UN'IDE+24+V1'LOC+Z22+1'LOC+Z16+2'UNT+5+1'",
)
.collect::<Result<_, _>>()?;

let resolution = schema.resolve(&segments);
assert_eq!(resolution.placement(2).and_then(|p| p.place.id()), Some("00048"));

let report = ValidationContext::builder()
    .with_validator(ValidationLayer::Structure, PlacementValidator::new(Arc::new(schema)))
    .build()
    .validate(&segments);
// LOC+Z16 after LOC+Z22 is out of guide order.
let issue = &report.errors()[0];
assert_eq!(issue.kind(), Some(FindingKind::Placement));
assert_eq!(issue.error_code(), Some("E068"));
assert_eq!(issue.segment_position(), Some(4));
# Ok::<(), edifact::EdifactError>(())
```

Resolution takes `O(n × reachable places)` time and `O(n)` extra memory, with no
recursion over the input. `direct_segment_indices` is linear in the span plus
the number of children.

### How a tag is resolved

For each segment, innermost open group first:

1. **The innermost group's own candidates.** Open: its child-group triggers, in
   declaration order. Ordered: its current entry, then later entries; the first
   match wins, except that a place already at its `max` yields to a later
   matching entry.
2. **The reachable places of each enclosing group, innermost first.** Open: all
   its child-group triggers. Ordered: its current and later entries. A match
   closes the groups inside it.
3. Otherwise the segment is a direct segment of an open group, or unplaced in
   an ordered one.

Step 1 comes first because one trigger tag often appears at several levels.
`UTILMD` has message-level parties in SG2 and a Vorgang's parties in SG12
inside SG4, both triggered by `NAD`:

```rust
use edifact::group::{GroupDef, group_segments_indexed};

static SG12: &[GroupDef] = &[GroupDef::new("SG12", "NAD")];
static SCHEMA: &[GroupDef] = &[
    GroupDef::new("SG2", "NAD"),
    GroupDef::with_children("SG4", "IDE", SG12),
];

let segments: Vec<_> = edifact::from_bytes(
    b"BGM+E01'NAD+MS+SENDER'NAD+MR+RECEIVER'IDE+24+V1'NAD+Z09+KUNDE'DTM+92:20260101:102'",
)
.collect::<Result<Vec<_>, _>>()?;

let tree = group_segments_indexed(&segments, SCHEMA, "ROOT");

// The message-level parties are SG2 …
assert_eq!(
    tree.children.iter().map(|g| g.definition).collect::<Vec<_>>(),
    ["SG2", "SG2", "SG4"],
);
// … and the party inside the Vorgang nests as SG12.
let sg4 = tree.children.last().unwrap();
assert_eq!(sg4.children[0].definition, "SG12");
# Ok::<(), edifact::EdifactError>(())
```

A group ends at the first segment its branch cannot consume, not at one an
outer branch could also consume; otherwise SG12 would be unreachable. A group's
own trigger is not among its candidates, so a repeated trigger opens the next
occurrence instead of nesting.

### Reading groups back

The tree stores index ranges. `segments(&all)` resolves a group against the
slice it was built from, `descendants()` walks the subtree in document order,
and `find(name)` yields every group with that name at any depth:

```rust
use edifact::group::{GroupDef, group_segments_indexed};

static SG12: &[GroupDef] = &[GroupDef::new("SG12", "NAD")];
static SCHEMA: &[GroupDef] = &[
    GroupDef::new("SG2", "NAD"),
    GroupDef::with_children("SG4", "IDE", SG12),
];

let segments: Vec<_> = edifact::from_bytes(
    b"NAD+MS+SENDER'IDE+24+V1'NAD+Z09+KUNDE'IDE+24+V2'NAD+VY+PARTY'",
)
.collect::<Result<Vec<_>, _>>()?;
let tree = group_segments_indexed(&segments, SCHEMA, "ROOT");

// Every SG12 in the message, whichever Vorgang it belongs to.
let parties: Vec<&str> = tree
    .find("SG12")
    .filter_map(|group| group.segments(&segments).first())
    .filter_map(|nad| nad.element_str(0))
    .collect();
assert_eq!(parties, ["Z09", "VY"]);
# Ok::<(), edifact::EdifactError>(())
```

### What a group actually spans

In an open group, a tag that triggers nothing belongs to whichever group is
open. Nothing closes the last group at `UNT`, so the trailer lands inside it.
`MessageWindow::segments` includes `UNH` and `UNT`; pass
`MessageWindow::body()` (the segments between them) when group boundaries
matter.

---

## Next steps

- [Profile Packs](@/docs/profile-packs.md) — group-scoped rules
- [Validation](@/docs/validation.md) — layers, reports, directory checks
- [Error Reference](@/docs/error-reference.md) — `E015`, `E068`–`E071`
