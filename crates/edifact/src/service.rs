//! Segment definitions for the ISO 9735 **service segments**.
//!
//! The batch-EDI service segments — `UNB`, `UNZ`, `UNG`, `UNE`, `UNH`, `UNT`,
//! `UNS`, `UNO`, `UNP`, `UGH`, and `UGT` — are defined by the ISO 9735 service
//! directory, ISO 9735-10:2002 (segments §5.1.5, composites §5.2.5, simple data
//! elements §5.3.5), not by a UN/EDIFACT directory release. They are not
//! licensed directory data, so they ship here. Every position matches the
//! JWG1 publication of syntax 4 release 3 (pinned by the `service_conformance`
//! test).
//!
//! # Why this exists
//!
//! Code-addressed access ([`Segment::value_by_code`],
//! `#[edifact(element = "0020")]`) needs a [`SegmentDefinition`] to resolve
//! against; these definitions make it work on the envelope without
//! hand-authoring `UNB` first.
//!
//! ```
//! use edifact::{from_bytes, service};
//!
//! let segments: Vec<_> =
//!     from_bytes(b"UNB+UNOC:3+SENDER+RECEIVER+260101:0900+IC4711'UNZ+0+IC4711'")
//!         .collect::<Result<Vec<_>, _>>()?;
//!
//! // DE 0020 — interchange control reference — by name, not by counting to 4.
//! assert_eq!(segments[0].value_by_code(&service::UNB, "0020")?, Some("IC4711"));
//! // Reaching into a composite works the same way: S004 component 1.
//! assert_eq!(segments[0].value_by_code(&service::UNB, "0017")?, Some("260101"));
//! # Ok::<(), edifact::EdifactError>(())
//! ```
//!
//! # Scope
//!
//! Service segments only. Message-level segments (`BGM`, `DTM`, `NAD`, `LIN`, …)
//! and their composites (`C002`, `C507`, `C082`, …) belong to a **directory
//! release**, and the UN UNTDID licence does not permit redistributing modified
//! Directory content — which is what transcribing it into `const` tables and
//! publishing it would be. Supply those yourself as `static` tables, or load
//! them at run time with [`DirectoryValidatorBuilder`], and check the result
//! with [`SegmentLayout::audit`].
//!
//! The `CONTRL` reporting segments — `UCI`, `UCF`, `UCM`, `UCS`, `UCD` — ship
//! too. They belong to a *message* rather than to the envelope, but that message
//! is defined by ISO 9735-4, not by a directory release, so the same argument
//! applies: one correct answer, no version to pick. Together with `UNH` and
//! `UNT` they are everything a `CONTRL` is built from, which means
//! [`service::lookup`] alone validates one. See [`contrl`] for
//! generating them.
//!
//! Not here: the interactive-EDI segments `UIB`/`UIH`/`UIR`/`UIT`/`UIZ`
//! (ISO 9735-3) and the security segments `USA`…`USY` (ISO 9735-5/-7/-9) — this
//! crate implements batch EDI. Envelope validation refuses them by name:
//! [`InteractiveSegmentNotSupported`](EdifactError::InteractiveSegmentNotSupported)
//! (`E055`), [`SecuritySegmentNotSupported`](EdifactError::SecuritySegmentNotSupported)
//! (`E056`), and any other tag starting with `U` is
//! [`ReservedServiceSegmentTag`](EdifactError::ReservedServiceSegmentTag)
//! (`E057`, ISO 9735-1:2002 §7.7).
//!
//! # Codes that genuinely repeat
//!
//! Some identifiers appear at more than one position and are therefore not
//! code-addressable — by design, because "which one" has no answer:
//!
//! | Segment | Code | Appears in |
//! |---|---|---|
//! | `UNB` | `0007` | S002 (sender) and S003 (recipient) |
//! | `UNH` | `0051` | S009, S016, S017, and S018 |
//!
//! Resolving one of those returns
//! [`EdifactError::AmbiguousDataElement`].
//! Address the unambiguous neighbour instead — `0004` for the sender, `0010` for
//! the recipient — or read the slot positionally.
//!
//! # `const`, not `static`
//!
//! Every definition here is a `const`, because `#[edifact(layout = …)]` expands
//! into a `const` initialiser and a `const` cannot read a `static`. Pointing the
//! derive straight at `service::UNB` therefore just works, with no alias in
//! between.
//!
//! # Representations
//!
//! Each position carries the representation ISO 9735-10:2002 §5.3.5 states for
//! it. [`validate_envelope`] checks presence, class
//! and length of every `UNB`, `UNZ`, `UNG`, `UNE`, `UNH` and `UNT` position
//! against these tables by default; [`DirectoryValidator`] with [`service::lookup`]
//! applies them to every service segment and adds the `E053` warning for
//! unsuppressed leading zeroes and trailing spaces.
//!
//! ```
//! use edifact::{from_bytes, validate_envelope_lenient};
//!
//! // DE 0036 is n..6: `abc` is no count.
//! let segs: Vec<_> = from_bytes(b"UNB+UNOC:3+S+R+260101:0900+1'UNH+1+X:D:96A:UN'BGM+1'UNT+3+1'UNZ+abc+1'")
//!     .collect::<Result<_, _>>()?;
//! let codes: Vec<_> = validate_envelope_lenient(&segs).errors.iter().map(|e| e.stable_code()).collect();
//! assert_eq!(codes, ["E048"]);
//! # Ok::<(), edifact::EdifactError>(())
//! ```
//!
//! # Syntax versions
//!
//! The definitions follow **syntax version 4**; its extra components are all
//! conditional, so a version 3 interchange is not rejected for lacking them.
//! Where the versions disagree, a position declares both forms
//! ([`ComponentRef::with_repr_by_syntax_version`]):
//!
//! | Data element | Versions 1–3 | Version 4 |
//! |---|---|---|
//! | 0008, 0014 | `an..14` | `an..35` |
//! | 0017 | `n6` (`YYMMDD`) | `n8` (`CCYYMMDD`) |
//! | 0051 | `an..2` | `an..3` |
//! | 0074 | `n..6` | `n..10` |
//! | 0135 | `a3` | `an..3` |
//!
//! Versions 1 and 2 use the version 3 forms. `UNB` S001 DE 0002 selects the
//! form; any value other than `1`–`4` (syntax `X` included) is refused by the
//! envelope with [`UnsupportedSyntaxVersion`](EdifactError::UnsupportedSyntaxVersion)
//! (`E059`), and either form is then accepted.

use crate::directory_validator::{ComponentRef, ElementRef, Repr, SegmentDefinition, Status};
// Names the module docs link to. Rustdoc versions differ on whether a module's
// inner docs resolve in the module or its parent; importing them here makes the
// links resolve either way.
#[cfg(doc)]
use crate::{
    DirectoryValidator, EdifactError, Segment, contrl,
    directory_validator::{DirectoryValidatorBuilder, SegmentLayout},
    service, validate_envelope,
};

use Status::{Conditional as C, Mandatory as M};

// ── composite data elements ───────────────────────────────────────────────────

/// S001 — syntax identifier (`UNB`). ISO 9735-10:2002 §5.2.5.
pub const S001: &[ComponentRef] = &[
    ComponentRef::new(1, "0001", M).with_repr(Repr::a(4)), // Syntax identifier
    ComponentRef::new(2, "0002", M).with_repr(Repr::an(1)), // Syntax version number
    ComponentRef::new(3, "0080", C).with_repr(Repr::an_up_to(6)), // Service code list directory version number
    ComponentRef::new(4, "0133", C).with_repr(Repr::an_up_to(3)), // Character encoding, coded
    ComponentRef::new(5, "0076", C).with_repr(Repr::an(2)), // Syntax release number (ISO 9735-1:2002 Annex D)
];

/// DE 0008/0014: versions 1–3 `an..14` (address for reverse routing), version 4 `an..35`.
const INTERNAL_ID: (Repr, Repr) = (Repr::an_up_to(14), Repr::an_up_to(35));
/// DE 0051 controlling agency: versions 1–3 `an..2`, version 4 `an..3`.
const AGENCY: (Repr, Repr) = (Repr::an_up_to(2), Repr::an_up_to(3));
/// DE 0135 service segment tag: versions 1–3 `a3` (as DE 0013), version 4 `an..3`.
const SERVICE_TAG: (Repr, Repr) = (Repr::a(3), Repr::an_up_to(3));

/// S002 — interchange sender (`UNB`). ISO 9735-10:2002 §5.2.5.
pub const S002: &[ComponentRef] = &[
    ComponentRef::new(1, "0004", M).with_repr(Repr::an_up_to(35)), // Interchange sender identification
    ComponentRef::new(2, "0007", C).with_repr(Repr::an_up_to(4)),  // Identification code qualifier
    ComponentRef::new(3, "0008", C).with_repr_by_syntax_version(INTERNAL_ID.0, INTERNAL_ID.1), // Interchange sender internal identification
    ComponentRef::new(4, "0042", C).with_repr(Repr::an_up_to(35)), // Interchange sender internal sub-identification
];

/// S003 — interchange recipient (`UNB`). ISO 9735-10:2002 §5.2.5.
pub const S003: &[ComponentRef] = &[
    ComponentRef::new(1, "0010", M).with_repr(Repr::an_up_to(35)), // Interchange recipient identification
    ComponentRef::new(2, "0007", C).with_repr(Repr::an_up_to(4)),  // Identification code qualifier
    ComponentRef::new(3, "0014", C).with_repr_by_syntax_version(INTERNAL_ID.0, INTERNAL_ID.1), // Interchange recipient internal identification
    ComponentRef::new(4, "0046", C).with_repr(Repr::an_up_to(35)), // Interchange recipient internal sub-identification
];

/// S004 — date and time of preparation (`UNB`, `UNG`). ISO 9735-10:2002 §5.2.5.
///
/// Version 3 transfers DE 0017 as `YYMMDD` (`n6`), version 4 as `CCYYMMDD`
/// (`n8`); DE 0019 is `HHMM` (`n4`) in every version. The envelope also checks
/// that both are a real date and time (`E060`).
pub const S004: &[ComponentRef] = &[
    ComponentRef::new(1, "0017", M).with_repr_by_syntax_version(Repr::n(6), Repr::n(8)), // Date
    ComponentRef::new(2, "0019", M).with_repr(Repr::n(4)),                               // Time
];

/// S005 — recipient reference / password details (`UNB`). ISO 9735-10:2002 §5.2.5.
pub const S005: &[ComponentRef] = &[
    ComponentRef::new(1, "0022", M).with_repr(Repr::an_up_to(14)), // Recipient reference/password
    ComponentRef::new(2, "0025", C).with_repr(Repr::an(2)), // Recipient reference/password qualifier
];

/// S006 — application sender identification (`UNG`). ISO 9735-10:2002 §5.2.5.
pub const S006: &[ComponentRef] = &[
    ComponentRef::new(1, "0040", M).with_repr(Repr::an_up_to(35)), // Application sender identification
    ComponentRef::new(2, "0007", C).with_repr(Repr::an_up_to(4)),  // Identification code qualifier
];

/// S007 — application recipient identification (`UNG`). ISO 9735-10:2002 §5.2.5.
pub const S007: &[ComponentRef] = &[
    ComponentRef::new(1, "0044", M).with_repr(Repr::an_up_to(35)), // Application recipient identification
    ComponentRef::new(2, "0007", C).with_repr(Repr::an_up_to(4)),  // Identification code qualifier
];

/// S008 — message version (`UNG`). ISO 9735-10:2002 §5.2.5.
pub const S008: &[ComponentRef] = &[
    ComponentRef::new(1, "0052", M).with_repr(Repr::an_up_to(3)), // Message version number
    ComponentRef::new(2, "0054", M).with_repr(Repr::an_up_to(3)), // Message release number
    ComponentRef::new(3, "0057", C).with_repr(Repr::an_up_to(6)), // Association assigned code
];

/// S009 — message identifier (`UNH`). ISO 9735-10:2002 §5.2.5.
pub const S009: &[ComponentRef] = &[
    ComponentRef::new(1, "0065", M).with_repr(Repr::an_up_to(6)), // Message type
    ComponentRef::new(2, "0052", M).with_repr(Repr::an_up_to(3)), // Message version number
    ComponentRef::new(3, "0054", M).with_repr(Repr::an_up_to(3)), // Message release number
    ComponentRef::new(4, "0051", M).with_repr_by_syntax_version(AGENCY.0, AGENCY.1), // Controlling agency, coded
    ComponentRef::new(5, "0057", C).with_repr(Repr::an_up_to(6)), // Association assigned code
    ComponentRef::new(6, "0110", C).with_repr(Repr::an_up_to(6)), // Code list directory version number
    ComponentRef::new(7, "0113", C).with_repr(Repr::an_up_to(6)), // Message type sub-function identification
];

/// S010 — status of the transfer (`UNH`). ISO 9735-10:2002 §5.2.5.
pub const S010: &[ComponentRef] = &[
    ComponentRef::new(1, "0070", M).with_repr(Repr::n_up_to(2)), // Sequence of transfers
    ComponentRef::new(2, "0073", C).with_repr(Repr::a(1)),       // First and last transfer
];

/// S016 — message subset identification (`UNH`). ISO 9735-10:2002 §5.2.5.
pub const S016: &[ComponentRef] = &[
    ComponentRef::new(1, "0115", M).with_repr(Repr::an_up_to(14)), // Message subset identification
    ComponentRef::new(2, "0116", C).with_repr(Repr::an_up_to(3)),  // Message subset version number
    ComponentRef::new(3, "0118", C).with_repr(Repr::an_up_to(3)),  // Message subset release number
    ComponentRef::new(4, "0051", C).with_repr_by_syntax_version(AGENCY.0, AGENCY.1), // Controlling agency, coded
];

/// S017 — message implementation guideline identification (`UNH`). ISO 9735-10:2002 §5.2.5.
pub const S017: &[ComponentRef] = &[
    ComponentRef::new(1, "0121", M).with_repr(Repr::an_up_to(14)), // Message implementation guideline identification
    ComponentRef::new(2, "0122", C).with_repr(Repr::an_up_to(3)), // Message implementation guideline version number
    ComponentRef::new(3, "0124", C).with_repr(Repr::an_up_to(3)), // Message implementation guideline release number
    ComponentRef::new(4, "0051", C).with_repr_by_syntax_version(AGENCY.0, AGENCY.1), // Controlling agency, coded
];

/// S018 — scenario identification (`UNH`). ISO 9735-10:2002 §5.2.5.
pub const S018: &[ComponentRef] = &[
    ComponentRef::new(1, "0127", M).with_repr(Repr::an_up_to(14)), // Scenario identification
    ComponentRef::new(2, "0128", C).with_repr(Repr::an_up_to(3)),  // Scenario version number
    ComponentRef::new(3, "0130", C).with_repr(Repr::an_up_to(3)),  // Scenario release number
    ComponentRef::new(4, "0051", C).with_repr_by_syntax_version(AGENCY.0, AGENCY.1), // Controlling agency, coded
];

/// S020 — reference identification (`UNO`). ISO 9735-10:2002 §5.2.5.
pub const S020: &[ComponentRef] = &[
    ComponentRef::new(1, "0813", M).with_repr(Repr::an_up_to(3)), // Reference qualifier
    ComponentRef::new(2, "0802", M).with_repr(Repr::an_up_to(35)), // Reference identification number
];

/// S021 — object type identification (`UNO`). ISO 9735-10:2002 §5.2.5.
pub const S021: &[ComponentRef] = &[
    ComponentRef::new(1, "0805", M).with_repr(Repr::an_up_to(3)), // Object type qualifier
    ComponentRef::new(2, "0809", C).with_repr(Repr::an_up_to(256)), // Object type attribute identification
    ComponentRef::new(3, "0808", C).with_repr(Repr::an_up_to(256)), // Object type attribute
    ComponentRef::new(4, "0051", C).with_repr_by_syntax_version(AGENCY.0, AGENCY.1), // Controlling agency, coded
];

/// S022 — status of the object (`UNO`). ISO 9735-10:2002 §5.2.5.
pub const S022: &[ComponentRef] = &[
    ComponentRef::new(1, "0810", M).with_repr(Repr::n_up_to(18)), // Length of object in octets of bits
    ComponentRef::new(2, "0814", C).with_repr(Repr::n_up_to(3)), // Number of segments before object
    ComponentRef::new(3, "0070", C).with_repr(Repr::n_up_to(2)), // Sequence of transfers
    ComponentRef::new(4, "0073", C).with_repr(Repr::a(1)),       // First and last transfer
];

/// S011 — data element identification (`UCI`, `UCF`, `UCM`, `UCD`).
/// ISO 9735-10:2002 §5.2.5.
///
/// Pinpoints the erroneous data element inside the segment a `CONTRL` reporting
/// level names. Both counts are **one-based**: DE 0098 counts the segment tag as
/// position 1, and DE 0104 counts every component position the composite
/// declares.
pub const S011: &[ComponentRef] = &[
    ComponentRef::new(1, "0098", M).with_repr(Repr::n_up_to(3)), // Erroneous data element position in segment
    ComponentRef::new(2, "0104", C).with_repr(Repr::n_up_to(3)), // Erroneous component data element position
    ComponentRef::new(3, "0136", C).with_repr(Repr::n_up_to(6)), // Erroneous data element occurrence
];

// ── segment definitions ───────────────────────────────────────────────────────

const UNB_ELEMENTS: &[ElementRef] = &[
    ElementRef::composite(1, "S001", M, 1, S001),
    ElementRef::composite(2, "S002", M, 1, S002),
    ElementRef::composite(3, "S003", M, 1, S003),
    ElementRef::composite(4, "S004", M, 1, S004),
    ElementRef::new(5, "0020", M, 1).with_repr(Repr::an_up_to(14)), // Interchange control reference
    ElementRef::composite(6, "S005", C, 1, S005),
    ElementRef::new(7, "0026", C, 1).with_repr(Repr::an_up_to(14)), // Application reference
    ElementRef::new(8, "0029", C, 1).with_repr(Repr::a(1)),         // Processing priority code
    ElementRef::new(9, "0031", C, 1).with_repr(Repr::n(1)),         // Acknowledgement request
    ElementRef::new(10, "0032", C, 1).with_repr(Repr::an_up_to(35)), // Interchange agreement identifier
    ElementRef::new(11, "0035", C, 1).with_repr(Repr::n(1)),         // Test indicator
];

/// `UNB` — interchange header. ISO 9735-10:2002 §5.1.5; ISO 9735-1:2002 §7.2.
pub const UNB: SegmentDefinition =
    SegmentDefinition::new("UNB", "Interchange header", UNB_ELEMENTS);

const UNZ_ELEMENTS: &[ElementRef] = &[
    ElementRef::new(1, "0036", M, 1).with_repr(Repr::n_up_to(6)), // Interchange control count
    ElementRef::new(2, "0020", M, 1).with_repr(Repr::an_up_to(14)), // Interchange control reference
];

/// `UNZ` — interchange trailer. ISO 9735-10:2002 §5.1.5; ISO 9735-1:2002 §7.2.
pub const UNZ: SegmentDefinition =
    SegmentDefinition::new("UNZ", "Interchange trailer", UNZ_ELEMENTS);

// Every element of `UNG` except the group reference number is conditional
// (ISO 9735-10:2002 §5.1.5, UNG); dependency note D2 ties 0038/0051/S008
// together as "all or none".
const UNG_ELEMENTS: &[ElementRef] = &[
    ElementRef::new(1, "0038", C, 1).with_repr(Repr::an_up_to(6)), // Message group identification
    ElementRef::composite(2, "S006", C, 1, S006),
    ElementRef::composite(3, "S007", C, 1, S007),
    ElementRef::composite(4, "S004", C, 1, S004),
    ElementRef::new(5, "0048", M, 1).with_repr(Repr::an_up_to(14)), // Group reference number
    ElementRef::new(6, "0051", C, 1).with_repr_by_syntax_version(AGENCY.0, AGENCY.1), // Controlling agency, coded
    ElementRef::composite(7, "S008", C, 1, S008),
    ElementRef::new(8, "0058", C, 1).with_repr(Repr::an_up_to(14)), // Application password
];

/// `UNG` — group header. ISO 9735-10:2002 §5.1.5; ISO 9735-1:2002 §7.3.
pub const UNG: SegmentDefinition = SegmentDefinition::new("UNG", "Group header", UNG_ELEMENTS);

const UNE_ELEMENTS: &[ElementRef] = &[
    ElementRef::new(1, "0060", M, 1).with_repr(Repr::n_up_to(6)), // Group control count
    ElementRef::new(2, "0048", M, 1).with_repr(Repr::an_up_to(14)), // Group reference number
];

/// `UNE` — group trailer. ISO 9735-10:2002 §5.1.5; ISO 9735-1:2002 §7.3.
pub const UNE: SegmentDefinition = SegmentDefinition::new("UNE", "Group trailer", UNE_ELEMENTS);

const UNH_ELEMENTS: &[ElementRef] = &[
    ElementRef::new(1, "0062", M, 1).with_repr(Repr::an_up_to(14)), // Message reference number
    ElementRef::composite(2, "S009", M, 1, S009),
    ElementRef::new(3, "0068", C, 1).with_repr(Repr::an_up_to(35)), // Common access reference
    ElementRef::composite(4, "S010", C, 1, S010),
    ElementRef::composite(5, "S016", C, 1, S016),
    ElementRef::composite(6, "S017", C, 1, S017),
    ElementRef::composite(7, "S018", C, 1, S018),
];

/// `UNH` — message header. ISO 9735-10:2002 §5.1.5; ISO 9735-1:2002 §7.4.
pub const UNH: SegmentDefinition = SegmentDefinition::new("UNH", "Message header", UNH_ELEMENTS);

const UNT_ELEMENTS: &[ElementRef] = &[
    ElementRef::new(1, "0074", M, 1)
        .with_repr_by_syntax_version(Repr::n_up_to(6), Repr::n_up_to(10)), // Number of segments in the message
    ElementRef::new(2, "0062", M, 1).with_repr(Repr::an_up_to(14)), // Message reference number
];

/// `UNT` — message trailer. ISO 9735-10:2002 §5.1.5; ISO 9735-1:2002 §7.4.
pub const UNT: SegmentDefinition = SegmentDefinition::new("UNT", "Message trailer", UNT_ELEMENTS);

const UNS_ELEMENTS: &[ElementRef] = &[
    ElementRef::new(1, "0081", M, 1).with_repr(Repr::a(1)), // Section identification
];

/// `UNS` — section control. ISO 9735-10:2002 §5.1.5; ISO 9735-1:2002 §7.4.
pub const UNS: SegmentDefinition = SegmentDefinition::new("UNS", "Section control", UNS_ELEMENTS);

const UNO_ELEMENTS: &[ElementRef] = &[
    ElementRef::new(1, "0800", M, 1).with_repr(Repr::an_up_to(35)), // Package reference number
    ElementRef::composite(2, "S020", M, 99, S020),
    ElementRef::composite(3, "S021", M, 99, S021),
    ElementRef::composite(4, "S022", M, 1, S022),
    // S302, S301, S300 and 0035 are interactive-EDI only (ISO 9735-10:2002 UNO note 4);
    // they are legal in a batch `UNO` only in the sense that they are absent.
];

/// `UNO` — object header, the opening segment of a package.
/// ISO 9735-10:2002 §5.1.5; ISO 9735-1:2002 §7.10.
///
/// A package carries an arbitrary object — a PDF, an image, an encrypted blob —
/// through an interchange alongside or instead of messages. The object itself is *not* EDIFACT-encoded and is not
/// tokenized by this crate; `UNO` S022 DE 0810 states its length in octets so a
/// receiver can lift it out of the byte stream.
pub const UNO: SegmentDefinition = SegmentDefinition::new("UNO", "Object header", UNO_ELEMENTS);

const UNP_ELEMENTS: &[ElementRef] = &[
    ElementRef::new(1, "0810", M, 1).with_repr(Repr::n_up_to(18)), // Length of object in octets of bits
    ElementRef::new(2, "0800", M, 1).with_repr(Repr::an_up_to(35)), // Package reference number
];

/// `UNP` — object trailer, closing the package opened by [`UNO`].
/// ISO 9735-10:2002 §5.1.5; ISO 9735-1:2002 §7.10.
pub const UNP: SegmentDefinition = SegmentDefinition::new("UNP", "Object trailer", UNP_ELEMENTS);

const UGH_ELEMENTS: &[ElementRef] = &[
    ElementRef::new(1, "0087", M, 1).with_repr(Repr::an_up_to(4)), // Anti-collision segment group identification
];

/// `UGH` — anti-collision segment group header.
/// ISO 9735-10:2002 §5.1.5; ISO 9735-1:2002 §12 and Annex C.
///
/// Wraps a segment group whose trigger segment would otherwise be ambiguous with
/// a neighbouring one, so a receiver can tell which group a segment belongs to
/// from the tag alone.
pub const UGH: SegmentDefinition =
    SegmentDefinition::new("UGH", "Anti-collision segment group header", UGH_ELEMENTS);

const UGT_ELEMENTS: &[ElementRef] = &[
    ElementRef::new(1, "0087", M, 1).with_repr(Repr::an_up_to(4)), // Anti-collision segment group identification
];

/// `UGT` — anti-collision segment group trailer; DE 0087 repeats the [`UGH`] value.
/// ISO 9735-10:2002 §5.1.5; ISO 9735-1:2002 §12 and Annex C.
pub const UGT: SegmentDefinition =
    SegmentDefinition::new("UGT", "Anti-collision segment group trailer", UGT_ELEMENTS);

// ── CONTRL reporting segments (ISO 9735-4) ────────────────────────────────────

const UCI_ELEMENTS: &[ElementRef] = &[
    ElementRef::new(1, "0020", M, 1).with_repr(Repr::an_up_to(14)), // Interchange control reference
    ElementRef::composite(2, "S002", M, 1, S002),
    ElementRef::composite(3, "S003", M, 1, S003),
    ElementRef::new(4, "0083", M, 1).with_repr(Repr::an_up_to(3)), // Action, coded
    ElementRef::new(5, "0085", C, 1).with_repr(Repr::an_up_to(3)), // Syntax error, coded
    ElementRef::new(6, "0135", C, 1).with_repr_by_syntax_version(SERVICE_TAG.0, SERVICE_TAG.1), // Service segment tag, coded
    ElementRef::composite(7, "S011", C, 1, S011),
    ElementRef::new(8, "0534", C, 1).with_repr(Repr::an_up_to(14)), // Security reference number
    ElementRef::new(9, "0138", C, 1).with_repr(Repr::n_up_to(6)),   // Security segment position
];

/// `UCI` — interchange response: the `CONTRL` reporting level for `UNB`/`UNZ`.
/// ISO 9735-10:2002 §5.1.5; ISO 9735-4:2002 §5.
pub const UCI: SegmentDefinition =
    SegmentDefinition::new("UCI", "Interchange response", UCI_ELEMENTS);

const UCF_ELEMENTS: &[ElementRef] = &[
    ElementRef::new(1, "0048", M, 1).with_repr(Repr::an_up_to(14)), // Group reference number
    ElementRef::composite(2, "S006", C, 1, S006),
    ElementRef::composite(3, "S007", C, 1, S007),
    ElementRef::new(4, "0083", M, 1).with_repr(Repr::an_up_to(3)), // Action, coded
    ElementRef::new(5, "0085", C, 1).with_repr(Repr::an_up_to(3)), // Syntax error, coded
    ElementRef::new(6, "0135", C, 1).with_repr_by_syntax_version(SERVICE_TAG.0, SERVICE_TAG.1), // Service segment tag, coded
    ElementRef::composite(7, "S011", C, 1, S011),
    ElementRef::new(8, "0534", C, 1).with_repr(Repr::an_up_to(14)), // Security reference number
    ElementRef::new(9, "0138", C, 1).with_repr(Repr::n_up_to(6)),   // Security segment position
];

/// `UCF` — group response: the `CONTRL` reporting level for `UNG`/`UNE`.
/// ISO 9735-10:2002 §5.1.5; ISO 9735-4:2002 §5.
pub const UCF: SegmentDefinition = SegmentDefinition::new("UCF", "Group response", UCF_ELEMENTS);

const UCM_ELEMENTS: &[ElementRef] = &[
    ElementRef::new(1, "0062", C, 1).with_repr(Repr::an_up_to(14)), // Message reference number
    ElementRef::composite(2, "S009", C, 1, S009),
    ElementRef::new(3, "0083", M, 1).with_repr(Repr::an_up_to(3)), // Action, coded
    ElementRef::new(4, "0085", C, 1).with_repr(Repr::an_up_to(3)), // Syntax error, coded
    ElementRef::new(5, "0135", C, 1).with_repr_by_syntax_version(SERVICE_TAG.0, SERVICE_TAG.1), // Service segment tag, coded
    ElementRef::composite(6, "S011", C, 1, S011),
    ElementRef::new(7, "0800", C, 1).with_repr(Repr::an_up_to(35)), // Package reference number
    ElementRef::composite(8, "S020", C, 99, S020),
    ElementRef::new(9, "0534", C, 1).with_repr(Repr::an_up_to(14)), // Security reference number
    ElementRef::new(10, "0138", C, 1).with_repr(Repr::n_up_to(6)),  // Security segment position
];

/// `UCM` — message/package response: the `CONTRL` reporting level for
/// `UNH`/`UNT` and `UNO`/`UNP`. ISO 9735-10:2002 §5.1.5; ISO 9735-4:2002 §5.
///
/// DE 0062 and DE 0800 are mutually exclusive (dependency note D1): a `UCM`
/// reports on a message *or* a package, never both.
pub const UCM: SegmentDefinition =
    SegmentDefinition::new("UCM", "Message/package response", UCM_ELEMENTS);

const UCS_ELEMENTS: &[ElementRef] = &[
    ElementRef::new(1, "0096", M, 1).with_repr(Repr::n_up_to(6)), // Segment position in message body
    ElementRef::new(2, "0085", C, 1).with_repr(Repr::an_up_to(3)), // Syntax error, coded
];

/// `UCS` — segment error indication: the `CONTRL` reporting level for one
/// segment of a message body. ISO 9735-10:2002 §5.1.5; ISO 9735-4:2002 §5.
///
/// DE 0096 counts from the `UNH`, which is segment 1. A *missing* segment is
/// reported at the position of the last segment processed before it was
/// expected.
pub const UCS: SegmentDefinition =
    SegmentDefinition::new("UCS", "Segment error indication", UCS_ELEMENTS);

const UCD_ELEMENTS: &[ElementRef] = &[
    ElementRef::new(1, "0085", M, 1).with_repr(Repr::an_up_to(3)), // Syntax error, coded
    ElementRef::composite(2, "S011", M, 1, S011),
];

/// `UCD` — data element error indication: the deepest `CONTRL` reporting level,
/// naming one erroneous data element inside the segment its `UCS` identifies.
/// ISO 9735-10:2002 §5.1.5; ISO 9735-4:2002 §5.
pub const UCD: SegmentDefinition =
    SegmentDefinition::new("UCD", "Data element error indication", UCD_ELEMENTS);

// ── lookup ────────────────────────────────────────────────────────────────────

/// The service-segment definition for `tag`, if there is one.
///
/// Plugs straight into
/// [`DirectoryValidator::new`][crate::DirectoryValidator::new] as the
/// `segment_lookup` hook, or chains ahead of a directory-specific lookup so the
/// envelope is validated without duplicating it in every directory table.
///
/// # Example
///
/// ```
/// use edifact::service;
///
/// assert_eq!(service::lookup("UNB").map(|d| d.tag), Some("UNB"));
/// assert!(service::lookup("BGM").is_none()); // a directory segment, not a service one
/// ```
#[must_use]
pub fn lookup(tag: &str) -> Option<&'static SegmentDefinition> {
    Some(match tag {
        "UNB" => &UNB,
        "UNZ" => &UNZ,
        "UNG" => &UNG,
        "UNE" => &UNE,
        "UNH" => &UNH,
        "UNT" => &UNT,
        "UNS" => &UNS,
        "UNO" => &UNO,
        "UNP" => &UNP,
        "UGH" => &UGH,
        "UGT" => &UGT,
        "UCI" => &UCI,
        "UCF" => &UCF,
        "UCM" => &UCM,
        "UCS" => &UCS,
        "UCD" => &UCD,
        _ => return None,
    })
}

/// The envelope, package, and anti-collision service segments, in envelope order.
///
/// # Example
///
/// ```
/// use edifact::service;
///
/// assert_eq!(
///     service::ENVELOPE.iter().map(|d| d.tag).collect::<Vec<_>>(),
///     ["UNB", "UNG", "UNH", "UNT", "UNE", "UNZ", "UNS", "UNO", "UNP", "UGH", "UGT"],
/// );
/// ```
pub const ENVELOPE: &[&SegmentDefinition] = &[
    &UNB, &UNG, &UNH, &UNT, &UNE, &UNZ, &UNS, &UNO, &UNP, &UGH, &UGT,
];

/// The `CONTRL` reporting segments, outermost level first (ISO 9735-4).
///
/// # Example
///
/// ```
/// use edifact::service;
///
/// assert_eq!(
///     service::CONTRL.iter().map(|d| d.tag).collect::<Vec<_>>(),
///     ["UCI", "UCF", "UCM", "UCS", "UCD"],
/// );
/// ```
pub const CONTRL: &[&SegmentDefinition] = &[&UCI, &UCF, &UCM, &UCS, &UCD];

/// Every service-segment definition this module ships: [`ENVELOPE`] then [`CONTRL`].
///
/// A `CONTRL` message is built entirely from segments in this list — `UNH`,
/// `UCI`, `UCM`, `UCS`, `UCD`, `UNT` — so [`lookup`] alone is enough to validate
/// one against the standard, with no directory involved.
///
/// # Example
///
/// ```
/// use edifact::service;
///
/// assert_eq!(service::ALL.len(), service::ENVELOPE.len() + service::CONTRL.len());
/// assert!(service::ALL.iter().any(|d| d.tag == "UCI"));
/// ```
pub const ALL: &[&SegmentDefinition] = &[
    &UNB, &UNG, &UNH, &UNT, &UNE, &UNZ, &UNS, &UNO, &UNP, &UGH, &UGT, &UCI, &UCF, &UCM, &UCS, &UCD,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directory_validator::SegmentLayout;

    #[test]
    fn every_definition_numbers_its_positions_consecutively_from_one() {
        for definition in ALL {
            for (index, element) in definition.elements.iter().enumerate() {
                assert_eq!(
                    element.position() as usize,
                    index + 1,
                    "{}: element positions must be consecutive and one-based",
                    definition.tag,
                );
                for (component_index, component) in element.components().iter().enumerate() {
                    assert_eq!(
                        component.position() as usize,
                        component_index + 1,
                        "{} {}: component positions must be consecutive and one-based",
                        definition.tag,
                        element.data_element(),
                    );
                }
            }
        }
    }

    #[test]
    fn the_lookup_and_the_list_agree() {
        for definition in ALL {
            assert_eq!(
                lookup(definition.tag).map(|d| d.tag),
                Some(definition.tag),
                "{} is in ALL but not in lookup()",
                definition.tag,
            );
        }
        assert_eq!(ALL.len(), ENVELOPE.len() + CONTRL.len());
        assert_eq!(ALL.len(), 16);
    }

    #[test]
    fn ung_matches_the_part_10_statuses() {
        use crate::directory_validator::Status;

        // ISO 9735-10 UNG: only the group reference number is mandatory.  Everything
        // else is conditional — including the pieces dependency note D2 ties
        // together as "all or none" (0038, 0051, S008).
        let statuses: Vec<_> = UNG
            .elements
            .iter()
            .map(|e| (e.data_element(), e.status()))
            .collect();
        assert_eq!(
            statuses,
            vec![
                ("0038", Status::Conditional),
                ("S006", Status::Conditional),
                ("S007", Status::Conditional),
                ("S004", Status::Conditional),
                ("0048", Status::Mandatory),
                ("0051", Status::Conditional),
                ("S008", Status::Conditional),
                ("0058", Status::Conditional),
            ],
        );
    }

    #[test]
    fn package_and_anti_collision_segments_resolve_their_codes() {
        assert_eq!(UNO.resolve_code("0800").unwrap().element, 0);
        // S022 component 1 — the object's length in octets.
        let length = UNO.resolve_code("0810").unwrap();
        assert_eq!((length.element, length.component), (3, Some(0)));
        assert_eq!(UNP.resolve_code("0800").unwrap().element, 1);
        assert_eq!(UGH.resolve_code("0087").unwrap().element, 0);
        assert_eq!(UGT.resolve_code("0087").unwrap().element, 0);
    }

    #[test]
    fn envelope_positions_match_the_ones_the_envelope_parser_assumes() {
        // `envelope.rs` reads these slots by hand-written index.  If the two ever
        // disagree, one of them is wrong — so pin them against each other.
        let cases: [(&SegmentDefinition, &str, usize, Option<usize>); 9] = [
            (&UNB, "0001", 0, Some(0)), // S001 comp 1 — syntax identifier
            (&UNB, "0004", 1, Some(0)), // S002 comp 1 — sender id
            (&UNB, "0010", 2, Some(0)), // S003 comp 1 — recipient id
            (&UNB, "0017", 3, Some(0)), // S004 comp 1 — date
            (&UNB, "0020", 4, None),    // interchange control reference
            (&UNB, "0035", 10, None),   // test indicator
            (&UNZ, "0036", 0, None),    // interchange control count
            (&UNH, "0062", 0, None),    // message reference number
            (&UNH, "0065", 1, Some(0)), // S009 comp 1 — message type
        ];
        for (definition, code, element, component) in cases {
            let path = definition
                .resolve_code(code)
                .unwrap_or_else(|e| panic!("{} {code}: {e}", definition.tag));
            assert_eq!(
                (path.element, path.component),
                (element, component),
                "{} {code}",
                definition.tag,
            );
        }
    }

    #[test]
    fn codes_that_genuinely_repeat_are_reported_as_ambiguous() {
        // Documented in the module header: these have no single right answer.
        assert!(
            UNB.resolve_code("0007").is_err(),
            "UNB 0007 (S002 and S003)"
        );
        assert!(
            UNH.resolve_code("0051").is_err(),
            "UNH 0051 (S009/S016/S017/S018)"
        );
        // And the neighbours that disambiguate them do resolve.
        assert!(UNB.resolve_code("0004").is_ok());
        assert!(UNB.resolve_code("0010").is_ok());
    }

    #[test]
    fn a_real_interchange_resolves_through_the_shipped_layouts() {
        let input = b"UNB+UNOC:3+SENDER:14+RECEIVER:14+260101:0900+IC4711++ORDERS'\
                      UNH+MSG1+ORDERS:D:96A:UN:EAN008'\
                      UNT+2+MSG1'UNZ+1+IC4711'";
        let segments: Vec<_> = crate::from_bytes(input)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(
            segments[0].value_by_code(&UNB, "0020").unwrap(),
            Some("IC4711")
        );
        assert_eq!(
            segments[0].value_by_code(&UNB, "0004").unwrap(),
            Some("SENDER")
        );
        assert_eq!(
            segments[0].value_by_code(&UNB, "0010").unwrap(),
            Some("RECEIVER")
        );
        assert_eq!(
            segments[0].value_by_code(&UNB, "0019").unwrap(),
            Some("0900")
        );
        assert_eq!(
            segments[0].value_by_code(&UNB, "0026").unwrap(),
            Some("ORDERS")
        );
        assert_eq!(
            segments[1].value_by_code(&UNH, "0062").unwrap(),
            Some("MSG1")
        );
        assert_eq!(
            segments[1].value_by_code(&UNH, "0057").unwrap(),
            Some("EAN008")
        );
        assert_eq!(segments[2].value_by_code(&UNT, "0074").unwrap(), Some("2"));
        assert_eq!(segments[3].value_by_code(&UNZ, "0036").unwrap(), Some("1"));

        // A layout applied to the wrong segment is refused, not silently misread.
        assert!(segments[0].value_by_code(&UNH, "0062").is_err());
    }

    #[test]
    fn the_shipped_layouts_drive_the_directory_validator() {
        use crate::{ValidationContext, ValidationLayer};

        let input = b"UNB+UNOC:3+S+R+260101:0900+IC1'UNZ+1+IC1'";
        let segments: Vec<_> = crate::from_bytes(input)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        let validator = crate::DirectoryValidator::new(
            "iso-9735-service",
            lookup,
            |_, _| true,
            |_, _| None,
            |_, _| None,
            None,
        );
        let report = ValidationContext::builder()
            .with_validator(ValidationLayer::Structure, validator)
            .build()
            .validate(&segments);

        assert!(!report.has_errors(), "{:#?}", report.errors());
    }
}
