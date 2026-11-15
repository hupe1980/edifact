+++
title = "CONTRL Acknowledgements"
description = "Generate the ISO 9735-4 syntax and service report message from a validation report — receipt, acknowledgement, and rejection with per-segment error placement."
weight = 85
+++

`CONTRL` tells a partner that their interchange arrived, was syntactically
accepted, or was rejected and where. It is defined by **ISO 9735-4**, part of
the syntax standard, not of a directory release. `edifact` builds one from a
[`ValidationReport`](@/docs/validation.md), so the acknowledgement you send and
the findings you log cannot disagree.

A `CONTRL` reports on **syntax**, not content (§5.2): "acknowledged" means the
interchange parsed and its envelope was consistent, not that anyone agreed to
the order. Application replies (`APERAK`, `ORDRSP`, …) are out of scope.

---

## Receipt, acknowledgement, rejection

§5.3.1 allows at most two `CONTRL` messages per subject interchange: an
optional receipt, then the result of the check.

| | Action code | When | Built with |
|---|---|---|---|
| **Receipt** | `8` | Immediately, before checking | `Contrl::receipt` |
| **Acknowledgement** | `7` | After a clean syntax check | `Contrl::acknowledgement` |
| **Rejection** | `4` | After a failed syntax check | `Contrl::from_report` |

The second message is mandatory when the subject's `UNB` DE 0031 requests an
acknowledgement: check `interchange.ack_requested()`.

```rust
use edifact::{Contrl, from_bytes, validate_envelope};

let raw = b"UNB+UNOC:3+SENDER+RECEIVER+260101:0900+IC4711'\
            UNH+MSG1+ORDERS:D:96A:UN'BGM+220+PO-1+9'UNT+3+MSG1'\
            UNZ+1+IC4711'";
let segments: Vec<_> = from_bytes(raw).collect::<Result<Vec<_>, _>>()?;
let validated = validate_envelope(&segments)?;

let wire = Contrl::acknowledgement(&validated)
    .with_message_reference("ACK1")
    .to_edifact_string()?;

// A syntax version 3 subject (`UNOC:3`) is answered with `CONTRL:D:3:UN`.
assert_eq!(
    wire,
    "UNH+ACK1+CONTRL:D:3:UN'UCI+IC4711+SENDER+RECEIVER+7'UNT+3+ACK1'",
);
# Ok::<(), edifact::EdifactError>(())
```

A clean interchange gets no `UCM` per message: the `UCI` acknowledgement
covers every message inside it (§5.3.4).

---

## Reporting a failed check

`Contrl::from_report` takes the subject interchange, its segments and the
report. The segments turn a finding's byte span into the `UCS` DE 0096 segment
position. Use the **lenient** envelope path: it yields both the interchange and
its faults.

```rust
use edifact::{Contrl, ValidationContext, from_bytes, validate_envelope_lenient};

// `FTX+   ` carries a value of nothing but spaces — ISO 9735-1 §9.3.
let raw = b"UNB+UNOC:3+S+R+260101:0900+IC1'\
            UNH+M1+ORDERS:D:96A:UN'FTX+   'UNT+3+M1'\
            UNZ+1+IC1'";
let segments: Vec<_> = from_bytes(raw).collect::<Result<Vec<_>, _>>()?;

let validated = validate_envelope_lenient(&segments).interchange.unwrap();
let report = ValidationContext::builder()
    .with_envelope_validation()
    .with_syntax_validation()
    .build()
    .validate(&segments);

let wire = Contrl::from_report(&validated, &segments, &report).to_edifact_string()?;

// FTX is segment 2 of the message (UNH is 1); the value is data element
// position 2 (the tag is position 1), component 1.
assert!(wire.contains("UCS+2'"));
assert!(wire.contains("UCD+12+2:1'"));
# Ok::<(), edifact::EdifactError>(())
```

Warnings are reported but do not reject: the interchange above stays
acknowledged (`7`) and still names the value (§5.3.3).

---

## Where a finding lands

The message nests five reporting levels, each naming a part of the subject
interchange:

```text
UCI   the interchange          → UNB / UNZ
 UCF  a group                  → UNG / UNE
  UCM a message or package     → UNH / UNT
   UCS a segment               → by position, UNH = 1
    UCD a data element         → by position within that segment
```

The levels follow the subject: an interchange with `UNG`/`UNE` is reported
through `UCF`, one without gets `UCM` directly under `UCI` (§5.3.1).

A fault is reported on the lowest level that can both locate it and, per
Annex A, carry its code (§5.3.3); the code is not repeated further up.
"Duplicate detected" (`26`), for instance, is not permitted below the message:

```rust
use edifact::contrl::{ReportingLevel, SyntaxError};

// Legal everywhere, so it descends as far as the finding can locate it.
assert!(SyntaxError::InvalidValue.permitted_at(ReportingLevel::DataElement));

// Message level and above only — a UCS could not carry it.
assert!(SyntaxError::DuplicateDetected.permitted_at(ReportingLevel::Message));
assert!(!SyntaxError::DuplicateDetected.permitted_at(ReportingLevel::Segment));

// Segment level only.
assert!(SyntaxError::TooManyGroupRepetitions.permitted_at(ReportingLevel::Segment));
assert!(!SyntaxError::TooManyGroupRepetitions.permitted_at(ReportingLevel::Interchange));
# Ok::<(), edifact::EdifactError>(())
```

### Service segments

A fault in a service segment is reported on that segment's own level, and the
level names it: `UNA`, `UNB` or `UNZ` on the `UCI`, `UNG` or `UNE` on the `UCF`,
`UNH` or `UNT` on the `UCM` — DE 0135 in syntax 4, DE 0013 in syntax 3, the same
position either way. When the erroneous element is known, S011 follows:

| Fault | Reported as |
|---|---|
| `UNT` segment count wrong | `UCM+…+4+29+UNT` |
| `UNZ` reference differs from `UNB` | `UCI+…+4+28+UNZ` |
| `UNE` reference differs from `UNG` | `UCF+…+4+28+UNE` |
| duplicate `UNH` reference | `UCM+…+4+26+UNH` |
| unsupported repertoire in `UNB` S001 | `UCI+…+4+46+UNB+2:1` |

The `UCM` copies the subject's DE 0062 and S009 in full — DE 0057 included,
so `UNH+1+UTILMD:D:11A:UN:S2.1` is named `UCM+1+UTILMD:D:11A:UN:S2.1+…`.

`with_interchange_error` drops a code Annex A forbids at the interchange
level.

### Groups

A fault in a group's own `UNG`/`UNE` rejects that group and everything in it;
the interchange around it stays acknowledged:

```rust
# use edifact::{Contrl, ValidationContext, from_bytes, validate_envelope_lenient};
// The UNE reference does not match its UNG.
let raw = b"UNB+UNOC:3+S+R+260101:0900+IC1'\
            UNG+ORDERS+SND+RCV+260101:0900+GRP1+UN+D:96A'\
            UNH+M1+ORDERS:D:96A:UN'BGM+220'UNT+3+M1'\
            UNE+1+GRP-OTHER'\
            UNZ+1+IC1'";
let segments: Vec<_> = from_bytes(raw).collect::<Result<Vec<_>, _>>()?;
let validated = validate_envelope_lenient(&segments).interchange.unwrap();
let report = ValidationContext::builder()
    .with_envelope_validation()
    .build()
    .validate(&segments);

let wire = Contrl::from_report(&validated, &segments, &report).to_edifact_string()?;

// UCI stays at 7; the UCF carries the rejection, code 28 and the service
// segment at fault (DE 0135 / DE 0013).
assert_eq!(
    wire,
    "UNH+IC1+CONTRL:D:3:UN'UCI+IC1+S+R+7'UCF+GRP1+SND+RCV+4+28+UNE'UNT+4+IC1'",
);
# Ok::<(), edifact::EdifactError>(())
```

---

## Choosing the message identifier

The `CONTRL`'s own `UNH` S009 depends on the syntax version of the interchange
it answers (`UNB` S001 DE 0002):

| Subject syntax version | `UNH` S009 |
|---|---|
| `4` | `CONTRL:4:1:UN` |
| `1`, `2`, `3` | `CONTRL:D:3:UN` |
| empty or unknown | `CONTRL:4:1:UN` — the `CONTRL` is how code `2` "syntax version not supported" is reported, so it is built rather than refused |

Every constructor derives it. A partner agreement that wants something else —
an association assigned code such as EANCOM's `EAN005`, or `4:1:UN` for a
syntax 3 subject — sets `ContrlIdentifier` in `ContrlOptions` or with
`with_identifier`:

```rust
use edifact::contrl::{Contrl, ContrlIdentifier, ContrlOptions};
use edifact::{from_bytes, validate_envelope};

let raw = b"UNB+UNOC:3+SENDER+RECEIVER+260101:0900+IC4711'\
            UNH+MSG1+ORDERS:D:96A:UN'BGM+220+PO-1+9'UNT+3+MSG1'\
            UNZ+1+IC4711'";
let segments: Vec<_> = from_bytes(raw).collect::<Result<Vec<_>, _>>()?;
let validated = validate_envelope(&segments)?;

// Derived: syntax 3 → D:3:UN.
let derived = Contrl::acknowledgement(&validated);
assert_eq!(derived.identifier(), &ContrlIdentifier::syntax_3());

// Explicit: an association assigned code, validated as DE 0057 an..6.
let options = ContrlOptions::new()
    .identifier(ContrlIdentifier::syntax_3().with_association("EX1")?);
let wire = Contrl::acknowledgement_with(&validated, &options)?
    .with_message_reference("ACK1")
    .to_edifact_string()?;
assert!(wire.starts_with("UNH+ACK1+CONTRL:D:3:UN:EX1'"));

// CONTRL:4:1:UN for a syntax 3 subject, if the partner agreement asks for it.
let syntax4 = Contrl::acknowledgement(&validated).with_identifier(ContrlIdentifier::syntax_4());
assert!(syntax4.to_edifact_string()?.starts_with("UNH+IC4711+CONTRL:4:1:UN'"));
# Ok::<(), edifact::EdifactError>(())
```

A component that violates its representation — DE 0052, 0054, 0051 empty or
longer than three characters, DE 0057 empty or longer than six — is
`EdifactError::InvalidContrlIdentifier` naming the data element.

---

## Community rules: action codes and whole-interchange rejection

ISO 9735-4 rejects per level: a faulty message is rejected on its `UCM`, the
`UCI` above stays at `7`. Some communities accept or reject an interchange as a
whole and admit only some action codes per level. `ContrlOptions` expresses
those rules; no community preset ships.

| Option | Default | Effect |
|---|---|---|
| `identifier` | derived | the `UNH` S009, see above |
| `actions` | every code at every level | `ActionPolicy::permit(level, &[…])` restricts DE 0083 on `UCI`, `UCF` or `UCM` |
| `rejection` | `RejectionScope::PerLevel` | `RejectionScope::Interchange`: every level above a rejected one carries `4` without DE 0085, and only rejected messages get a `UCM` |
| `report_warnings` | `true` | `false` leaves warnings out — a guide that admits only `4` on `UCM` cannot express a warning's `7` |

An action the policy forbids is `EdifactError::ActionNotPermitted`, naming the
segment and the code; it is never emitted or rewritten. The
`_with` constructors (`receipt_with`, `acknowledgement_with`,
`from_report_with`, `from_recovered`) take the options and return a `Result`;
the option-less ones keep the defaults and cannot fail.

The BDEW configuration (CONTRL MIG 2.0b) as caller code:

```rust
use edifact::contrl::{
    Action, ActionPolicy, Contrl, ContrlIdentifier, ContrlOptions, RejectionScope, ReportingLevel,
};
use edifact::{EdifactError, ValidationContext, from_bytes, validate_envelope_lenient};

let bdew = ContrlOptions::new()
    .identifier(ContrlIdentifier::syntax_3().with_association("2.0b")?)
    .actions(
        ActionPolicy::new()
            .permit(ReportingLevel::Interchange, &[Action::Acknowledged, Action::Rejected])
            .permit(ReportingLevel::Message, &[Action::Rejected]),
    )
    .rejection(RejectionScope::Interchange)
    .report_warnings(false);

// The UNT segment count is wrong.
let raw = b"UNB+UNOC:3+4012345000023:500+9900357000004:500+261001:0700+IC9'\
            UNH+1+UTILMD:D:11A:UN:S2.1'BGM+E03+X'UNT+9+1'\
            UNZ+1+IC9'";
let segments: Vec<_> = from_bytes(raw).collect::<Result<Vec<_>, _>>()?;
let validated = validate_envelope_lenient(&segments).interchange.unwrap();
let report = ValidationContext::builder()
    .with_envelope_validation()
    .build()
    .validate(&segments);

let wire = Contrl::from_report_with(&validated, &segments, &report, &bdew)?
    .with_message_reference("ACK1")
    .to_edifact_string()?;
assert_eq!(
    wire,
    "UNH+ACK1+CONTRL:D:3:UN:2.0b'\
     UCI+IC9+4012345000023:500+9900357000004:500+4'\
     UCM+1+UTILMD:D:11A:UN:S2.1+4+29+UNT'\
     UNT+4+ACK1'",
);

// A receipt (action 8) is not admitted on the UCI.
assert!(matches!(
    Contrl::receipt_with(&validated.interchange, &bdew),
    Err(EdifactError::ActionNotPermitted { segment_tag: "UCI", action: "8" })
));
# Ok::<(), edifact::EdifactError>(())
```

Under `RejectionScope::Interchange` a fault in the interchange envelope itself
still reads `UCI+…+4+<code>` with nothing below, exactly as per-level; a grouped
subject puts `4` without DE 0085 on the faulty message's `UCF` and on the `UCI`.
Under any options, a `CONTRL` either fails to build or carries only permitted
action codes and passes `validate_envelope`.

---

## Error-code mapping

Every `EdifactError` maps to the narrowest Annex A code that is true of it
(§5.3.3).

| Finding | `CONTRL` code |
|---|---|
| `MessageCountMismatch`, `GroupCountMismatch`, `SegmentCountMismatch` | `29` control count does not match (`GroupCountMismatch` on the group's `UCF`) |
| `QualifierMismatch` (control references) | `28` references do not match |
| `DuplicateReference` | `26` duplicate detected |
| `MissingRequiredElement`, `MissingSegment` | `13` missing |
| `InvalidCodeValue`, `BlankDataElementValue`, `InvalidDateTime` | `12` invalid value |
| `InvalidSegmentForMessage`, `ReservedServiceSegmentTag` | `15` not supported in this position |
| `UnsupportedSyntaxVersion` | `2` syntax version or level not supported, on `UCI` naming `UNB` S001 |
| `InvalidElementCount`, `InvalidComponentCount` | `16` too many constituents |
| `UnrecognisedSyntaxIdentifier`, `UnsupportedCharset` | `46` character set not supported |
| `CharacterNotInRepertoire`, `InvalidText` | `21` invalid character(s) |
| `InvalidUna`, `InvalidDelimiter` | `22` invalid service character(s) |
| `EmptyInterchange`, `EmptyGroup` | `32` lower level empty (`EmptyGroup` on the group's `UCF`) |
| `EmptyMessage` | `13` missing, on the message's `UCM` (Annex A permits `32` only on `UCI`/`UCF`) |
| `PackageNotSupported`, `InteractiveSegmentNotSupported`, `SecuritySegmentNotSupported` | `47` envelope functionality not supported |
| `DataAfterInterchangeTrailer` | `33` invalid occurrence outside message, package or group, on `UCI` |
| anything else | `18` unspecified error |

Where the location changes the code, the location wins: an invalid
segment tag (`E006`) or a stray data token (`E028`) inside a message is `12`
"invalid value" on data element position 1 — the tag is the invalid value —
while outside a message it stays `33`; an invalid `UNA` (`E007`) is `20`
"character invalid as service character" on the `UCI`. A code Annex A forbids
at the lowest level that locates the fault climbs to the next level that
permits it (message → group → interchange), so a `CONTRL` never carries a code
its level may not use.

`SyntaxError::for_issue` maps a `ValidationIssue` by its stable code, so
findings from any validator map. An issue with no stable code, such as a
profile-rule finding, becomes `18`.

---

## Answering an interchange that does not parse

The strict parser stops at the first fault, so an interchange whose messages
do not tokenize yields no validated interchange. A `CONTRL` is still owed,
addressed to the `UNB` parties and pointing at the broken segment.

`RecoveredInterchange::scan` runs the same parser, records a fault where it
refuses a segment, and resumes after that segment's terminator. It reads the `UNB` addressing, the message windows with their `UNH` identity,
and a `ContrlLocation` for every fault — reporting level, message, `UCS`
DE 0096, `UCD` DE 0098 / DE 0104, the service segment and the DE 0085 code:

```rust
use edifact::contrl::ReportingLevel;
use edifact::{RecoveredInterchange, RecoveryConfig};

// The second message's BGM tag is broken: `BGME03` is no segment tag.
let raw = b"UNB+UNOC:3+4012345000023:500+9900357000004:500+261001:0700+IC9'\
            UNH+1+UTILMD:D:11A:UN:S2.1'BGM+E03+X'UNT+3+1'\
            UNH+2+UTILMD:D:11A:UN:S2.1'BGME03+X'UNT+3+2'\
            UNZ+2+IC9'";
let recovered = RecoveredInterchange::scan(raw, &RecoveryConfig::default());

// Who the CONTRL goes back to — read even though the interchange does not parse.
let address = recovered.address().expect("UNB is readable");
assert_eq!(address.sender_id(), "4012345000023");
assert_eq!(address.recipient_id(), "9900357000004");
assert_eq!(address.control_ref(), "IC9");

// Where the fault is: message 2 (DE 0062), segment 2 (UNH = 1), the tag itself
// (data element position 1), code 12 "invalid value".
let location = &recovered.fault_locations()[0];
let message = &recovered.messages()[location.message().unwrap()];
assert_eq!(message.identity().unwrap().message_ref(), "2");
assert_eq!(location.level(), ReportingLevel::DataElement);
assert_eq!(location.segment_position(), Some(2));
assert_eq!(location.element_position(), Some(1));
assert_eq!(location.error().code(), "12");
```

What the scan guarantees:

- **Same parser.** Every segment it yields is the one `from_bytes` yields for
  the same bytes; the first fault carries the strict parser's error. After a
  fault it resumes after the next unreleased segment terminator, searching no
  further than `max_segment_bytes`.
- **Bounded.** Every `ReaderConfig` limit applies — by default 64 KiB per
  segment, 16 MiB per message, 256 MiB in total (faults count as message
  bytes, every consumed byte against `max_input_bytes`) — plus
  `RecoveryConfig::max_faults` (default 1 000).
- **Nothing invented.** A message without `UNT` closes at the next `UNH` or
  `UNZ` with no trailer; an unreadable `UNB` sender, recipient or control
  reference means no address at all. An invalid `UNA`, an unterminated final
  segment, an overlong segment or an exceeded limit ends the scan, and
  `is_complete()` is then `false`.

Errors that carry no position — `MissingRequiredElement`,
`MissingRequiredComponent`, `InvalidFieldValue`, `MissingSegment` — are placed
with `locate(&error, Some(message_index))` on the first segment of that message
with the error's tag, and the location says it is not exact:

```rust
use edifact::{EdifactError, RecoveredInterchange, RecoveryConfig};

let raw = b"UNB+UNOC:3+S+R+260101:0900+IC1'\
            UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+3+1'UNZ+1+IC1'";
let recovered = RecoveredInterchange::scan(raw, &RecoveryConfig::default());

let missing = EdifactError::MissingRequiredElement { tag: "BGM".into(), element_index: 1 };
let location = recovered.locate(&missing, Some(0)).expect("a subject fault");
assert_eq!(location.segment_position(), Some(2));
assert_eq!(location.element_position(), Some(3));
assert_eq!(location.error().code(), "13");
assert!(!location.is_exact());
```

Service-segment faults are placed as in [Service segments](#service-segments)
(`ContrlLocation::service_segment`). Input in a repertoire other than UTF-8 is
read through `decode_interchange` first; locations are counts, so they do not
change.

`Contrl::from_recovered` turns the scan into the `CONTRL` itself: addressed
from the recovered `UNB`, every fault rejecting its level, under the same
options as `from_report_with`. Locations from another layer — `locate` of a
typed-layer error — go in as `extra` and are reported the same way:

```rust
use edifact::contrl::{Contrl, ContrlOptions, RejectionScope};
use edifact::{RecoveredInterchange, RecoveryConfig};

let raw = b"UNB+UNOC:3+4012345000023:500+9900357000004:500+261001:0700+IC9'\
            UNH+1+UTILMD:D:11A:UN:S2.1'BGM+E03+X'UNT+3+1'\
            UNH+2+UTILMD:D:11A:UN:S2.1'BGME03+X'UNT+3+2'\
            UNZ+2+IC9'";
let recovered = RecoveredInterchange::scan(raw, &RecoveryConfig::default());

let options = ContrlOptions::new().rejection(RejectionScope::Interchange);
let wire = Contrl::from_recovered(&recovered, &[], &options)?
    .with_message_reference("ACK1")
    .to_edifact_string()?;

// The whole interchange is rejected; only the broken message is named, down to
// its segment (UCS DE 0096) and the tag (UCD DE 0098 = 1).
assert_eq!(
    wire,
    "UNH+ACK1+CONTRL:D:3:UN'\
     UCI+IC9+4012345000023:500+9900357000004:500+4'\
     UCM+2+UTILMD:D:11A:UN:S2.1+4'UCS+2'UCD+12+1'\
     UNT+6+ACK1'",
);
# Ok::<(), edifact::EdifactError>(())
```

- Without readable `UNB` addressing there is no `CONTRL`
  (`EdifactError::MissingSegment` with tag `UNB`).
- A fault in a message whose `UNH` cannot be read rejects the `UCI`, since no
  `UCM` can name it.
- When the scan ended on a terminal fault, the `UCI` carries `4` whatever the
  scope: the rest of the interchange was never checked.

---

## Sending it

A `CONTRL` travels in an interchange of its own, never inside a group (§5.3).
`to_interchange_bytes` builds that envelope with sender and recipient swapped:

```rust
# use edifact::{Contrl, from_bytes, validate_envelope};
# let raw = b"UNB+UNOC:3+SENDER+RECEIVER+260101:0900+IC4711'UNH+M1+ORDERS:D:96A:UN'BGM+220'UNT+3+M1'UNZ+1+IC4711'";
# let segments: Vec<_> = from_bytes(raw).collect::<Result<Vec<_>, _>>()?;
# let validated = validate_envelope(&segments)?;
let wire = Contrl::acknowledgement(&validated)
    .to_interchange_string("UNOC", "3", "260101", "0930", "ACK-1")?;

assert!(wire.starts_with("UNB+UNOC:3+RECEIVER+SENDER+260101:0930+ACK-1'"));
assert!(wire.ends_with("UNZ+1+ACK-1'"));
# Ok::<(), edifact::EdifactError>(())
```

The reply `UNB` is an `InterchangeHeader` written with
`Writer::begin_interchange`: a subject without a sender or recipient is
`E008`/`E021`, a party the repertoire cannot carry `E038`, and no bytes are
written. Identity values read back as the subject's. `UNT` DE 0074 and `UNZ`
DE 0036 are computed from what was emitted; the bytes are in the declared
repertoire (`to_interchange_string` decodes them).

---

## Parsing an inbound CONTRL

The same layouts serve the read direction. A `CONTRL` is built entirely from
segments in `edifact::service`, so `service::lookup` validates one with no
directory involved:

```rust
use edifact::{DirectoryValidator, ValidationContext, ValidationLayer, from_bytes, service};

let raw = b"UNH+ACK1+CONTRL:4:1:UN'UCI+IC4711+SENDER+RECEIVER+7'UNT+3+ACK1'";
let segments: Vec<_> = from_bytes(raw).collect::<Result<Vec<_>, _>>()?;

// Read the verdict by data element identifier, not by counting to element 3.
assert_eq!(segments[1].value_by_code(&service::UCI, "0083")?, Some("7"));
assert_eq!(segments[1].value_by_code(&service::UCI, "0020")?, Some("IC4711"));

let validator = DirectoryValidator::new(
    "iso-9735-4", service::lookup, |_, _| true, |_, _| None, |_, _| None, None,
);
let report = ValidationContext::builder()
    .with_validator(ValidationLayer::Structure, validator)
    .build()
    .validate(&segments);
assert!(!report.has_errors());
# Ok::<(), edifact::EdifactError>(())
```

---

## Scope

This module generates and reads `CONTRL`. `AUTACK` (part 6) and `KEYMAN`
(part 9) belong to the security parts, which this crate does not cover.

`UNO`…`UNP` **packages** carry binary data this crate does not tokenize: a
subject carrying one raises `E044`, reported as `47` "envelope functionality
not supported".

---

## Further reading

- [Validation](@/docs/validation.md) — producing the report a `CONTRL` reports on
- [Error Reference](@/docs/error-reference.md) — every stable code and what it means
- [Writing](@/docs/writing.md) — the `Writer` that renders the result
