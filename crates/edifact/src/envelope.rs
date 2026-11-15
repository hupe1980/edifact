//! EDIFACT envelope validation — UNB / UNG / UNH / UNT / UNE / UNZ.
//!
//! Validates the full interchange structure including optional groups
//! (`UNG`/`UNE`).  The public surface is:
//!
//! - [`validate_envelope`] — fail-fast strict validation
//! - [`validate_envelope_lenient`] — collects all errors
//! - [`parse_unh`] / [`parse_ung`] — zero-copy parse of the header identifier fields
//!
//! # What is checked
//!
//! Every finding is an error; none is a warning.
//!
//! | Rule | Code | Source |
//! |---|---|---|
//! | The interchange opens with `UNB` (`UIB`: interactive, refused) and closes with `UNZ` | `E015`, `E055` | ISO 9735-1:2002 §7.2 |
//! | Nothing but whitespace follows the first `UNZ` outside a message | `E058` | ISO 9735-2:2002 §5 |
//! | It contains at least one message or group | `E042` | ISO 9735-1:2002 §7.2 |
//! | A group opens with `UNG`, closes with `UNE`, holds a message, and is not nested | `E011`, `E015`, `E061` | ISO 9735-1:2002 §7.3 |
//! | A message opens with `UNH`, closes with `UNT`, and has a body | `E011`, `E015`, `E043` | ISO 9735-1:2002 §7.4 |
//! | Every `UNB`/`UNZ`/`UNG`/`UNE`/`UNH`/`UNT` position is present, of its class and length for the syntax version | `E008`, `E021`, `E048`–`E050` | ISO 9735-10:2002 §5.1.5–§5.3.5 |
//! | `UNB` S001 DE 0002 is `1`–`4` | `E059` | ISO 9735-1:2002 §2 |
//! | `UNB`/`UNG` S004 is a calendar date and a 24-hour time | `E060` | ISO 9735-10:2002 §5.3.5, DE 0017/0019 |
//! | `UNB` S001 DE 0001 names a defined character repertoire | `E031`, `E039` | ISO 9735-1:2002 §6 |
//! | `UNZ`/`UNE`/`UNT` references repeat their header's | `E016` | ISO 9735-10:2002 §5.1.5, UNZ/UNE/UNT note 1 |
//! | `UNH` DE 0062 and `UNG` DE 0048 are unique within the interchange | `E032` | ISO 9735-10:2002 §5.1.5 |
//! | The declared counts match what is there | `E004`, `E054`, `E005` | ISO 9735-10:2002 §5.3.5, DE 0036 / 0060 / 0074 |
//! | No interactive, security or other `U` segment, message bodies included | `E055`–`E057` | ISO 9735-1:2002 §7.7 |
//!
//! ```
//! use edifact::{from_bytes, validate_envelope};
//!
//! // Version 3 transfers YYMMDD: an eight-digit date is too long.
//! let segs: Vec<_> = from_bytes(b"UNB+UNOC:3+S+R+20260101:0900+1'UNH+1+X:D:96A:UN'BGM+1'UNT+3+1'UNZ+1+1'")
//!     .collect::<Result<_, _>>()?;
//! assert_eq!(validate_envelope(&segs).unwrap_err().stable_code(), "E049");
//! # Ok::<(), edifact::EdifactError>(())
//! ```
//!
//! # Count semantics
//!
//! - `UNZ` DE 0036 — "the number of messages and packages in an interchange or,
//!   if used, the number of groups in an interchange" ([`EdifactError::MessageCountMismatch`]).
//! - `UNE` DE 0060 — "the number of messages and packages in the group"
//!   ([`EdifactError::GroupCountMismatch`]).
//! - `UNT` DE 0074 — "the number of segments in a message body, plus the message
//!   header segment and message trailer segment" ([`EdifactError::SegmentCountMismatch`]).
//!
//! A count that is not an unsigned integer is `E048` and is not compared.
//!
//! # Packages
//!
//! An interchange may carry packages (`UNO`…`UNP`) instead of, or alongside,
//! messages (ISO 9735-1:2002 §7.10). The object inside a package is arbitrary
//! binary data whose length is declared in `UNO` S022 DE 0810 — it is not
//! EDIFACT-encoded and this crate does not tokenize it. A package therefore
//! reaches this validator as [`EdifactError::PackageNotSupported`] rather than as
//! a misleading complaint about a stray segment.

use crate::error::escape_control;
use crate::{
    error::EdifactError,
    header::InterchangeHeader,
    model::{Segment, Span},
};
use std::collections::HashSet;

// ── Public data types ─────────────────────────────────────────────────────────

/// The interchange envelope: the typed `UNB` header and the `UNZ` count.
///
/// ```
/// let input = b"UNB+UNOA:3+S+R+200101:0900+CTRL++++++1'UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+3+1'UNZ+1+CTRL'";
/// let segs: Vec<_> = edifact::from_bytes(input).collect::<Result<_, _>>()?;
/// let result = edifact::validate_envelope(&segs)?;
/// assert_eq!(result.interchange.header.sender.id, "S");
/// assert!(result.interchange.is_test());
/// # Ok::<(), edifact::EdifactError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct InterchangeEnvelope {
    /// The `UNB` header, as [`InterchangeHeader::parse`] types it.
    pub header: InterchangeHeader,
    /// Interchange unit count declared in `UNZ` DE 0036; `0` when it is not a
    /// count (reported as `E008`/`E048`/`E049`).
    ///
    /// - When no functional groups are present: count of messages (`UNH`/`UNT` pairs).
    /// - When functional groups are present: count of groups (`UNG`/`UNE` pairs).
    pub declared_unit_count: u32,
    /// Actual unit count observed (groups if groups present; messages otherwise).
    pub actual_unit_count: u32,
}

impl InterchangeEnvelope {
    /// Whether the test indicator (`UNB` DE 0035) is set.
    ///
    /// Test interchanges must not be processed as production data.
    #[inline]
    #[must_use]
    pub fn is_test(&self) -> bool {
        self.header.is_test()
    }

    /// Whether the sender requests an acknowledgement (`UNB` DE 0031).
    #[inline]
    #[must_use]
    pub fn ack_requested(&self) -> bool {
        self.header.ack_requested()
    }
}

impl std::fmt::Display for InterchangeEnvelope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{sender} -> {recipient} [{ctrl}] ({syntax}:{ver})",
            sender = escape_control(&self.header.sender.id),
            recipient = escape_control(&self.header.recipient.id),
            ctrl = escape_control(&self.header.control_ref),
            syntax = self.header.charset.syntax_identifier(),
            ver = self.header.syntax_version.number(),
        )
    }
}

/// Extracted data from a single `UNH` / `UNT` message envelope.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MessageEnvelope {
    /// Message reference from `UNH` element 0.
    pub message_ref: String,
    /// EDIFACT message type, e.g. `"ORDERS"`.
    pub message_type: String,
    /// Version number, e.g. `"D"`.
    pub version: String,
    /// Release number, e.g. `"11A"`.
    pub release: String,
    /// Controlling agency code, e.g. `"UN"`.
    pub controlling_agency: String,
    /// Association assigned code (MIG version), e.g. `"FV2510"`.
    pub association_code: String,
    /// Common access reference (UNH DE 0068, element index 2), if present.
    ///
    /// A reference shared across related messages or exchanges on the same network
    /// path.  Used by some EDI network profiles to
    /// correlate messages that belong to a single business transaction.
    /// `None` when element \[2\] is absent or empty.
    pub common_access_ref: Option<String>,
    /// Sequence of transfers (UNH S010 DE 0070, element index 3), if present.
    ///
    /// When a large message is split across multiple interchanges, this is the
    /// 1-based index of this segment within the sequence.  `None` when the message
    /// is not split (element \[3\] absent).
    pub sequence_of_transfers: Option<u32>,
    /// Transfer position indicator (UNH S010 DE 0073, element index 3 comp 1), if present.
    ///
    /// Values per ISO 9735-10:2002 §5.3.5 (DE 0073): `"C"` = continuation, `"F"` = first, `"L"` = last.
    /// `None` when element \[3\] is absent.
    pub transfer_position: Option<String>,
    /// Declared segment count from `UNT` DE 0074; `0` when it is not a count.
    pub declared_segment_count: u64,
    /// Actual segment count between this `UNH` and its `UNT`.
    pub actual_segment_count: u64,
    /// Byte range of this message's `UNH`.
    pub header_span: Span,
    /// Byte range of this message's `UNT`.
    ///
    /// A count mismatch is the trailer's fault, so this is where a diagnostic
    /// should point and what places the finding on this message when a
    /// [`Contrl`][crate::Contrl] is built from the report.
    pub trailer_span: Span,
}

impl std::fmt::Display for MessageEnvelope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{msg_type}:{ver}:{rel} ref={msg_ref} seg={actual}/{declared}",
            msg_type = escape_control(&self.message_type),
            ver = escape_control(&self.version),
            rel = escape_control(&self.release),
            msg_ref = escape_control(&self.message_ref),
            actual = self.actual_segment_count,
            declared = self.declared_segment_count,
        )
    }
}

/// Extracted data from a single `UNG` / `UNE` functional group envelope.
///
/// ISO 9735-1 §7.2 defines optional functional groups that may wrap one or more
/// `UNH`/`UNT` message pairs.  This type carries the parsed fields from both
/// the `UNG` header and its matching `UNE` trailer, plus the validated messages.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FunctionalGroupEnvelope {
    /// Functional group identification (UNG DE 0038), e.g. `"ORDERS"`.
    pub group_id: String,
    /// Application sender's identification (UNG S006 DE 0040).
    pub app_sender: String,
    /// Application sender identification code qualifier (UNG S006 DE 0007).
    ///
    /// Empty string when no qualifier is present.
    pub app_sender_qualifier: String,
    /// Application recipient's identification (UNG S007 DE 0044).
    pub app_recipient: String,
    /// Application recipient identification code qualifier (UNG S007 DE 0007).
    ///
    /// Empty string when no qualifier is present.
    pub app_recipient_qualifier: String,
    /// Date of preparation (UNG S004 DE 0017), e.g. `"200101"` (YYMMDD format).
    pub date: String,
    /// Time of preparation (UNG S004 DE 0019), e.g. `"0900"` (HHMM format), if present.
    pub time: Option<String>,
    /// Functional group reference number (UNG DE 0048). Must match `UNE` DE 0048.
    pub group_ref: String,
    /// Controlling agency, coded (UNG DE 0051), e.g. `"UN"`.
    pub controlling_agency: String,
    /// Message version number, e.g. `"D"`.
    pub version: String,
    /// Message release number, e.g. `"96A"`.
    pub release: String,
    /// Application password (UNG DE 0058), if present.
    ///
    /// A password to the recipient's division, department, or sectional
    /// application system — the group-level counterpart of the interchange's
    /// `recipient_password`.
    pub application_password: Option<String>,
    /// Declared message count from `UNE` DE 0060; `0` when it is not a count.
    pub declared_message_count: u32,
    /// Actual number of `UNH`/`UNT` pairs found within this group.
    pub actual_message_count: u32,
    /// Messages contained within this functional group.
    pub messages: Vec<MessageEnvelope>,
}

/// Fully validated interchange structure returned by [`validate_envelope`].
///
/// Provides both hierarchical (group → message) and flat (all messages) access
/// so that callers who do not care about group boundaries can use
/// [`messages`](Self::messages) directly.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ValidatedInterchange {
    /// Interchange-level envelope data (from `UNB`/`UNZ`).
    pub interchange: InterchangeEnvelope,
    /// Functional groups, when the interchange uses `UNG`/`UNE` wrappers.
    ///
    /// Empty when messages appear directly under the interchange.
    pub functional_groups: Vec<FunctionalGroupEnvelope>,
    /// Flat list of all messages in the interchange.
    ///
    /// When functional groups are present this contains the same messages as
    /// the nested `messages` fields inside each [`FunctionalGroupEnvelope`].
    pub messages: Vec<MessageEnvelope>,
}

impl ValidatedInterchange {
    /// Returns `true` if this interchange uses `UNG`/`UNE` functional group wrappers.
    #[inline]
    #[must_use]
    pub fn has_functional_groups(&self) -> bool {
        !self.functional_groups.is_empty()
    }

    /// Total number of `UNH`/`UNT` message pairs across all groups.
    ///
    /// Equivalent to `self.messages.len()` but communicates intent clearly.
    #[inline]
    #[must_use]
    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    /// Iterate over all messages in the interchange.
    ///
    /// Equivalent to `self.messages.iter()` but communicates intent clearly
    /// and is stable regardless of future internal layout changes.
    #[inline]
    pub fn iter_messages(&self) -> impl Iterator<Item = &MessageEnvelope> {
        self.messages.iter()
    }

    /// Find the first message whose `message_ref` equals `reference`.
    ///
    /// Useful for locating a specific message in an interchange with multiple
    /// messages after calling `validate_envelope`.
    ///
    /// Returns `None` if no message with that reference exists.
    #[inline]
    #[must_use]
    pub fn find_message(&self, reference: &str) -> Option<&MessageEnvelope> {
        self.messages.iter().find(|m| m.message_ref == reference)
    }

    /// Collect all messages of a given type (e.g. `"ORDERS"`, `"INVOIC"`).
    ///
    /// Returns a `Vec` of references to matching messages in document order.
    /// Returns an empty `Vec` when the interchange contains no messages of
    /// the requested type.
    ///
    /// Prefer [`iter_messages_by_type`](Self::iter_messages_by_type) in tight loops
    /// to avoid the allocation.
    #[must_use]
    pub fn messages_by_type(&self, message_type: &str) -> Vec<&MessageEnvelope> {
        self.messages
            .iter()
            .filter(|m| m.message_type == message_type)
            .collect()
    }

    /// Iterate over all messages of a given type without allocating.
    ///
    /// The lazy alternative to [`messages_by_type`](Self::messages_by_type)
    /// (asserted by the `allocation_budget` test
    /// `messages_by_type_iter_allocates_nothing`).
    ///
    /// The bound `'q: 's` means the `message_type` string reference must outlive the
    /// borrow of `self`, which a string literal always does. For a short-lived
    /// computed string, use [`messages_by_type`](Self::messages_by_type), which
    /// collects eagerly and releases the string reference immediately.
    #[inline]
    pub fn iter_messages_by_type<'s, 'q: 's>(
        &'s self,
        message_type: &'q str,
    ) -> impl Iterator<Item = &'s MessageEnvelope> + 's {
        self.messages
            .iter()
            .filter(move |m| m.message_type == message_type)
    }
}

impl std::fmt::Display for ValidatedInterchange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{ic} messages={n}",
            ic = self.interchange,
            n = self.messages.len(),
        )
    }
}

impl std::fmt::Display for FunctionalGroupEnvelope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{gid} sender={sender} recipient={recip} [{gref}] ({agency}) msgs={actual}/{declared}",
            gid = escape_control(&self.group_id),
            sender = escape_control(&self.app_sender),
            recip = escape_control(&self.app_recipient),
            gref = escape_control(&self.group_ref),
            agency = escape_control(&self.controlling_agency),
            actual = self.actual_message_count,
            declared = self.declared_message_count,
        )
    }
}

/// Parsed identifier fields from a `UNH` segment.
///
/// All string slices borrow from the input bytes so they live as long as the
/// original byte buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MessageIdentifier<'a> {
    /// Message reference number (UNH DE 0062, element index 0).
    pub message_ref: &'a str,
    pub message_type: &'a str,
    pub version: &'a str,
    pub release: &'a str,
    pub controlling_agency: &'a str,
    /// Association assigned code (UNH S009 DE 0057).
    ///
    /// Matches `MessageEnvelope::association_code` for the same message.
    pub association_code: &'a str,
}

impl<'a> MessageIdentifier<'a> {
    /// A message reference and S009 (DE 0065, 0052, 0054, 0051), without an
    /// association assigned code.
    ///
    /// ```
    /// use edifact::{MessageIdentifier, from_bytes, parse_unh};
    ///
    /// let id = MessageIdentifier::new("1", "UTILMD", "D", "11A", "UN").with_association_code("S2.1");
    /// let segments: Vec<edifact::Segment<'static>> =
    ///     from_bytes(b"UNH+1+UTILMD:D:11A:UN:S2.1'").collect::<Result<_, _>>()?;
    /// assert_eq!(parse_unh(&segments[0])?, id);
    /// # Ok::<(), edifact::EdifactError>(())
    /// ```
    #[must_use]
    pub const fn new(
        message_ref: &'a str,
        message_type: &'a str,
        version: &'a str,
        release: &'a str,
        controlling_agency: &'a str,
    ) -> Self {
        Self {
            message_ref,
            message_type,
            version,
            release,
            controlling_agency,
            association_code: "",
        }
    }

    /// Set the association assigned code (S009 DE 0057), e.g. `S2.1`.
    #[must_use]
    pub const fn with_association_code(mut self, code: &'a str) -> Self {
        self.association_code = code;
        self
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Extract identifier fields from a `UNH` segment.
///
/// Every field borrows from `unh`; a successful call allocates nothing
/// (asserted by the `allocation_budget` test `parse_unh_allocates_nothing`).
pub fn parse_unh<'a>(unh: &'a Segment<'_>) -> Result<MessageIdentifier<'a>, EdifactError> {
    // Element [0]: DE 0062 — message reference number (required, simple DE)
    let message_ref = unh
        .get_element(0)
        .and_then(|e| e.get_component(0))
        .filter(|s| !s.is_empty())
        .ok_or_else(|| EdifactError::MissingRequiredComponent {
            tag: "UNH".to_owned(),
            element_index: 0,
            component_index: 0,
        })?;
    // Element [1]: S009 composite — message type, version, release, agency, association
    let elem = unh
        .get_element(1)
        .ok_or_else(|| EdifactError::MissingRequiredElement {
            tag: "UNH".to_owned(),
            element_index: 1,
        })?;
    let message_type =
        elem.get_component(0)
            .ok_or_else(|| EdifactError::MissingRequiredComponent {
                tag: "UNH".to_owned(),
                element_index: 1,
                component_index: 0,
            })?;
    Ok(MessageIdentifier {
        message_ref,
        message_type,
        version: elem.get_component(1).unwrap_or(""),
        release: elem.get_component(2).unwrap_or(""),
        controlling_agency: elem.get_component(3).unwrap_or(""),
        association_code: elem.get_component(4).unwrap_or(""),
    })
}

/// Parsed identifier fields from a `UNG` segment.
///
/// All string slices borrow from the input bytes so they live as long as the
/// original byte buffer.  Use this to route groups in streaming scenarios where
/// you need to inspect group identity without full validation.
///
/// # UNG element positions (ISO 9735-10:2002 §5.1.5, UNG; 0-indexed)
///
/// ```text
/// [0] DE 0038  message group identification
/// [1] S006     application sender id + qualifier (comp 0 / comp 1)
/// [2] S007     application recipient id + qualifier (comp 0 / comp 1)
/// [3] S004     date + time (comp 0 / comp 1)
/// [4] DE 0048  group reference number     ← the only mandatory one
/// [5] DE 0051  controlling agency
/// [6] S008     version + release (comp 0 / comp 1)
/// [7] DE 0058  application password
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GroupIdentifier<'a> {
    /// Functional group identification (UNG DE 0038), e.g. `"ORDERS"`.
    pub group_id: &'a str,
    /// Application sender identification (UNG S006 DE 0040).
    pub app_sender: &'a str,
    /// Application sender identification code qualifier (UNG S006 DE 0007).
    pub app_sender_qualifier: &'a str,
    /// Application recipient identification (UNG S007 DE 0044).
    pub app_recipient: &'a str,
    /// Application recipient identification code qualifier (UNG S007 DE 0007).
    pub app_recipient_qualifier: &'a str,
    /// Group reference number (UNG DE 0048).
    pub group_ref: &'a str,
    /// Controlling agency (UNG DE 0051), e.g. `"UN"`.
    pub controlling_agency: &'a str,
    /// Message version number from S008 (UNG DE 0052), e.g. `"D"`.
    pub version: &'a str,
    /// Message release number from S008 (UNG DE 0054), e.g. `"96A"`.
    pub release: &'a str,
    /// Application password (UNG DE 0058); empty when absent.
    pub application_password: &'a str,
}

/// Extract identifier fields from a `UNG` segment.
///
/// Every field borrows from `ung`; a successful call allocates nothing
/// (asserted by the `allocation_budget` test `parse_ung_allocates_nothing`).
///
/// The symmetric counterpart to [`parse_unh`] for streaming scenarios that need
/// to inspect or route functional groups before full validation.
pub fn parse_ung<'a>(ung: &'a Segment<'_>) -> Result<GroupIdentifier<'a>, EdifactError> {
    let group_id = ung
        .get_element(0)
        .and_then(|e| e.get_component(0))
        .unwrap_or("");
    let app_sender = ung
        .get_element(1)
        .and_then(|e| e.get_component(0))
        .unwrap_or("");
    let app_sender_qualifier = ung
        .get_element(1)
        .and_then(|e| e.get_component(1))
        .unwrap_or("");
    let app_recipient = ung
        .get_element(2)
        .and_then(|e| e.get_component(0))
        .unwrap_or("");
    let app_recipient_qualifier = ung
        .get_element(2)
        .and_then(|e| e.get_component(1))
        .unwrap_or("");
    let group_ref = ung
        .get_element(4)
        .and_then(|e| e.get_component(0))
        .ok_or_else(|| EdifactError::MissingRequiredComponent {
            tag: "UNG".to_owned(),
            element_index: 4,
            component_index: 0,
        })?;
    let controlling_agency = ung
        .get_element(5)
        .and_then(|e| e.get_component(0))
        .unwrap_or("");
    let s008 = ung.get_element(6);
    let version = s008.as_ref().and_then(|e| e.get_component(0)).unwrap_or("");
    let release = s008.as_ref().and_then(|e| e.get_component(1)).unwrap_or("");
    let application_password = ung
        .get_element(7)
        .and_then(|e| e.get_component(0))
        .unwrap_or("");
    Ok(GroupIdentifier {
        group_id,
        app_sender,
        app_sender_qualifier,
        app_recipient,
        app_recipient_qualifier,
        group_ref,
        controlling_agency,
        version,
        release,
        application_password,
    })
}

/// Validate the EDIFACT interchange envelope, failing at the first violation.
///
/// Supports direct-message interchanges and functional-group interchanges, and
/// checks every service-segment value against ISO 9735-10:2002 §5. Returns
/// [`ValidatedInterchange`] on success.
///
/// Accepts segments from either parsing path — `&[OwnedSegment]` coerces to
/// `&[Segment<'_>]`.
///
/// # Errors
///
/// The first violation found, in discovery order.  Use
/// [`validate_envelope_lenient`] to collect all of them.
pub fn validate_envelope(segments: &[Segment<'_>]) -> Result<ValidatedInterchange, EdifactError> {
    validate_envelope_impl(segments)
}

/// Result of a lenient envelope validation — carries both a (possibly partial)
/// interchange and the full list of collected errors.
///
/// Returned by [`validate_envelope_lenient`].
///
/// # Semantics
///
/// | Condition | `interchange` | `errors` |
/// |-----------|---------------|----------|
/// | Structurally valid, all counts correct | `Some(result)` | empty |
/// | Structurally parseable but count violations | `Some(partial)` | non-empty |
/// | Missing `UNB`/`UNZ`, stray segments, etc. | `None` | non-empty |
#[derive(Debug)]
#[non_exhaustive]
pub struct LenientResult {
    /// The parsed interchange, if extraction was structurally possible.
    pub interchange: Option<ValidatedInterchange>,
    /// All errors collected during validation, in discovery order.
    pub errors: Vec<EdifactError>,
}

impl LenientResult {
    /// Returns `true` if no errors were detected and the interchange is fully valid.
    #[inline]
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    /// Returns `true` if one or more errors were collected.
    ///
    /// The readable inverse of [`is_valid`](Self::is_valid).
    /// A partial interchange may still be present even when `has_errors()` returns `true`.
    #[inline]
    #[must_use]
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Convert into a `Result`, returning the interchange on success or the errors on failure.
    ///
    /// The partial interchange (when present alongside errors) is discarded on the
    /// `Err` path.  Use the fields directly when you need both simultaneously.
    ///
    /// This conversion is total.  Because [`errors`](Self::errors) is a public
    /// field, callers may legitimately filter out violations they tolerate before
    /// converting; if that leaves no errors but also no interchange, the result is
    /// an empty `Err` rather than a panic.
    pub fn into_strict(self) -> Result<ValidatedInterchange, Vec<EdifactError>> {
        match self.interchange {
            Some(interchange) if self.errors.is_empty() => Ok(interchange),
            _ => Err(self.errors),
        }
    }
}

/// Validate the EDIFACT envelope and collect **all** errors rather than stopping
/// at the first failure (borrowed-segment path).
///
/// Returns a [`LenientResult`] whose `interchange` field is:
///
/// - `Some(result)` with empty `errors` when fully valid.
/// - `Some(partial)` with non-empty `errors` when only count violations were found —
///   lets diagnostic tooling display the actual interchange structure.
/// - `None` when the interchange is structurally broken beyond recovery
///   (missing `UNB`/`UNZ`, stray segments, etc.).
pub fn validate_envelope_lenient(segments: &[Segment<'_>]) -> LenientResult {
    validate_envelope_lenient_impl(segments)
}

// ── Core implementation ───────────────────────────────────────────────────────

/// Collector for **recoverable** envelope violations.
///
/// Extraction distinguishes two error classes:
///
/// * *Recoverable* — the violation is recorded here and extraction continues,
///   so later checks still run: representation and presence findings, control
///   reference and count mismatches, refused segments.
/// * *Fatal* — the structure cannot be interpreted at all (no `UNB`/`UNZ`, a
///   stray segment outside any message, an unterminated message).  These are
///   returned via `Err` and abort extraction.
///
/// Strict and lenient validation share one implementation over this sink, which
/// is what keeps them from diverging: strict reports the first error the sink
/// collected, lenient reports all of them.
#[derive(Default)]
struct ErrorSink {
    errors: Vec<EdifactError>,
    /// Index of the segment under check, when the check is about one segment.
    at: Option<usize>,
    /// The segment index each error was found at, filled by [`Self::attribute`].
    located: Vec<Option<usize>>,
}

impl ErrorSink {
    /// Attribute the errors collected so far to the segment under check, then
    /// check the segment at `index` next.
    fn attribute(&mut self, index: Option<usize>) {
        self.located.resize(self.errors.len(), self.at);
        self.at = index;
    }

    #[inline]
    fn push(&mut self, error: EdifactError) {
        self.errors.push(error);
    }

    /// Record a recoverable failure and continue with `fallback`.
    #[inline]
    fn recover<T>(&mut self, result: Result<T, EdifactError>, fallback: T) -> T {
        match result {
            Ok(value) => value,
            Err(error) => {
                self.errors.push(error);
                fallback
            }
        }
    }
}

/// The value at `(element, component)`, or `""` when absent.
///
/// Presence is reported once, by the service-segment check; extraction only
/// reads.
#[inline]
fn text(seg: &Segment<'_>, element: usize, component: usize) -> String {
    seg.component_str(element, component)
        .unwrap_or("")
        .to_owned()
}

/// Shared extraction used by both the strict and the lenient entry points.
///
/// Returns the interchange when the structure was interpretable at all, plus
/// every violation found in discovery order.
fn validate_envelope_collecting(
    segments: &[Segment<'_>],
) -> (Option<ValidatedInterchange>, ErrorSink) {
    let single = |error| ErrorSink {
        errors: vec![error],
        at: None,
        located: vec![None],
    };
    match segments.first() {
        Some(first) if first.tag() == "UNB" => {}
        // An interactive interchange opens with `UIB`: name it rather than
        // reporting a missing `UNB`.
        Some(first) => {
            let error = classify_u_tag(first.tag(), first.span).unwrap_or_else(missing_unb);
            return (None, single(error));
        }
        None => return (None, single(missing_unb())),
    }
    let Some(end) = interchange_end(segments) else {
        return (
            None,
            single(EdifactError::MissingSegment {
                tag: "UNZ".to_owned(),
                expected_position: "last segment of interchange".to_owned(),
            }),
        );
    };

    let mut sink = ErrorSink::default();
    let interchange = &segments[..=end];
    let result = validate_interchange(interchange, &mut sink);
    // ISO 9735-2:2002 §5 scopes a `UNA` to the interchange it precedes; what
    // follows the trailer is no part of this one, and is reported once.
    if let Some(trailing) = segments.get(end + 1) {
        sink.push(EdifactError::DataAfterInterchangeTrailer {
            tag: trailing.tag().to_owned(),
            span: trailing.span,
        });
    }
    sink.attribute(None);
    (result, sink)
}

fn missing_unb() -> EdifactError {
    EdifactError::MissingSegment {
        tag: "UNB".to_owned(),
        expected_position: "first segment of interchange".to_owned(),
    }
}

/// Index of the `UNZ` that ends the interchange: the first `UNZ` outside a
/// message. `None` when there is none.
///
/// A `UNZ` inside a message is no trailer; when it is the last segment it is
/// taken as one, so extraction reports it as nested (`E011`) instead of the
/// trailer as missing.
fn interchange_end(segments: &[Segment<'_>]) -> Option<usize> {
    let mut in_message = false;
    for (index, seg) in segments.iter().enumerate().skip(1) {
        match seg.tag() {
            "UNH" => in_message = true,
            "UNT" => in_message = false,
            "UNZ" if !in_message => return Some(index),
            _ => {}
        }
    }
    (segments.len() > 1 && segments.last().is_some_and(|s| s.tag() == "UNZ"))
        .then(|| segments.len() - 1)
}

/// Validate `interchange`, which runs from its `UNB` to its `UNZ`.
fn validate_interchange(
    interchange: &[Segment<'_>],
    sink: &mut ErrorSink,
) -> Option<ValidatedInterchange> {
    let version = check_service_segments(interchange, sink);
    let interchange_env = extract_interchange(interchange, sink);

    let inner = &interchange[1..interchange.len() - 1];
    let (functional_groups, messages) = match extract_content(inner, version, sink) {
        Ok(pair) => pair,
        Err(fatal) => {
            sink.push(fatal);
            return None;
        }
    };
    let control_ref = interchange[0].component_str(4, 0).unwrap_or("").to_owned();

    // ISO 9735-1 §7.1: an interchange "shall contain at least one group, or one
    // message or one package".  An empty one is a delivery that says nothing —
    // and `UNZ+0` makes the counts agree, so nothing else would catch it.
    if functional_groups.is_empty() && messages.is_empty() {
        sink.push(EdifactError::EmptyInterchange { control_ref });
    }

    // `UNZ` DE 0036 counts "the number of messages and packages in an interchange
    // or, if used, the number of groups in an interchange" (ISO 9735-10:2002
    // §5.3.5, DE 0036).
    let actual_unit_count = if functional_groups.is_empty() {
        messages.len()
    } else {
        functional_groups.len()
    };
    let actual_unit_count = sink.recover(
        u32::try_from(actual_unit_count).map_err(|_| EdifactError::InterchangeTooLarge {
            count: actual_unit_count as u64,
        }),
        u32::MAX,
    );

    let unz = &interchange[interchange.len() - 1];
    let mut declared_unit_count = 0;
    // n..6: always fits `u32`.
    if let Some(declared) = control_count(unz, 6, sink).and_then(|d| u32::try_from(d).ok()) {
        declared_unit_count = declared;
        if declared != actual_unit_count {
            sink.push(EdifactError::MessageCountMismatch {
                expected: declared,
                actual: actual_unit_count,
                span: unz.span,
            });
        }
    }

    let mut interchange_env = interchange_env?;
    interchange_env.declared_unit_count = declared_unit_count;
    interchange_env.actual_unit_count = actual_unit_count;
    Some(ValidatedInterchange {
        interchange: interchange_env,
        functional_groups,
        messages,
    })
}

fn validate_envelope_impl(segments: &[Segment<'_>]) -> Result<ValidatedInterchange, EdifactError> {
    match validate_envelope_collecting(segments) {
        (Some(result), sink) if sink.errors.is_empty() => Ok(result),
        (_, sink) => Err(sink.errors.into_iter().next().unwrap_or_else(missing_unb)),
    }
}

fn validate_envelope_lenient_impl(segments: &[Segment<'_>]) -> LenientResult {
    validate_envelope_located(segments).0
}

/// [`validate_envelope_lenient`], plus the index into `segments` of the
/// segment each error was found at, when a per-segment check found it.
pub(crate) fn validate_envelope_located(
    segments: &[Segment<'_>],
) -> (LenientResult, Vec<Option<usize>>) {
    let (interchange, sink) = validate_envelope_collecting(segments);
    (
        LenientResult {
            interchange,
            errors: sink.errors,
        },
        sink.located,
    )
}

// ── Service segment checks ────────────────────────────────────────────────────

/// Interactive-EDI segments, ISO 9735-3:2002.
const INTERACTIVE_TAGS: [&str; 5] = ["UIB", "UIH", "UIR", "UIT", "UIZ"];

/// Security segments, ISO 9735-5/-7/-9:2002.
const SECURITY_TAGS: [&str; 13] = [
    "USA", "USB", "USC", "USD", "USE", "USF", "USH", "USL", "USR", "UST", "USU", "USX", "USY",
];

/// The `U` tag of a segment this crate refuses, as the finding naming it.
///
/// `E055` for interactive, `E056` for security segments, `E057` for any other
/// tag starting with `U` that no batch service segment
/// ([`service::lookup`](crate::service::lookup)) has (ISO 9735-1:2002 §7.7).
/// `None` for every other tag, and for `UNA`, which the tokenizer reports.
pub(crate) fn classify_u_tag(tag: &str, span: Span) -> Option<EdifactError> {
    if !tag.starts_with('U') || tag == "UNA" || crate::service::lookup(tag).is_some() {
        return None;
    }
    let tag_owned = tag.to_owned();
    Some(if INTERACTIVE_TAGS.contains(&tag) {
        EdifactError::InteractiveSegmentNotSupported {
            tag: tag_owned,
            span,
        }
    } else if SECURITY_TAGS.contains(&tag) {
        EdifactError::SecuritySegmentNotSupported {
            tag: tag_owned,
            span,
        }
    } else {
        EdifactError::ReservedServiceSegmentTag {
            tag: tag_owned,
            span,
        }
    })
}

/// Whether `tag` is refused by [`classify_u_tag`]; such a segment is reported
/// there and skipped by extraction.
fn is_refused(seg: &Segment<'_>) -> bool {
    classify_u_tag(seg.tag(), seg.span).is_some()
}

/// The syntax version `UNB` S001 DE 0002 declares, for selecting
/// representations.
///
/// `Ok(Some(1..=4))`; `Ok(None)` when DE 0002 is absent (its presence is the
/// service check's finding); `Err(E059)` for any other value, syntax `X`
/// included.
pub(crate) fn syntax_version(unb: &Segment<'_>) -> Result<Option<u8>, EdifactError> {
    let value = unb.component_str(0, 1).unwrap_or("");
    if value.is_empty() {
        return Ok(None);
    }
    match crate::directory_validator::supported_syntax_version(value) {
        Some(version) => Ok(Some(version)),
        None => Err(EdifactError::UnsupportedSyntaxVersion {
            value: value.to_owned(),
            span: unb
                .get_element(0)
                .and_then(|e| e.component_span(1))
                .unwrap_or(unb.span),
        }),
    }
}

/// Run the service-segment checks over `interchange` and return the syntax
/// version that selects representations.
///
/// `E059` once for an unsupported DE 0002; presence, class and length of every
/// `UNB`/`UNZ`/`UNG`/`UNE`/`UNH`/`UNT` position (ISO 9735-10:2002 §5); S004
/// calendar validity; `E055`–`E057` for refused tags wherever they appear.
fn check_service_segments(interchange: &[Segment<'_>], sink: &mut ErrorSink) -> Option<u8> {
    let version = match syntax_version(&interchange[0]) {
        Ok(version) => version,
        Err(error) => {
            sink.push(error);
            None
        }
    };
    for (index, seg) in interchange.iter().enumerate() {
        sink.attribute(Some(index));
        match seg.tag() {
            "UNB" | "UNG" => {
                crate::directory_validator::check_service_segment(seg, version, &mut sink.errors);
                check_s004(seg, version, &mut sink.errors);
                if seg.tag() == "UNB" {
                    crate::header::check_flags(seg, &mut sink.errors);
                }
            }
            "UNZ" | "UNE" | "UNH" | "UNT" => {
                crate::directory_validator::check_service_segment(seg, version, &mut sink.errors);
            }
            tag => {
                if let Some(error) = classify_u_tag(tag, seg.span) {
                    sink.push(error);
                }
            }
        }
    }
    sink.attribute(None);
    version
}

/// Calendar validity of S004 (element 4) in `UNB` or `UNG` (`E060`).
///
/// Checked only for a value that passed its representation check — the
/// version's digit count — so a malformed value is reported once, as
/// `E048`/`E049`/`E050`.
pub(crate) fn check_s004(seg: &Segment<'_>, version: Option<u8>, out: &mut Vec<EdifactError>) {
    const S004: usize = 3;
    let Some(element) = seg.get_element(S004) else {
        return;
    };
    // A value of the wrong length or class is the representation check's
    // finding; only a well-formed one is read as a date or a time.
    let digits = |v: &str, len: usize| v.len() == len && v.bytes().all(|b| b.is_ascii_digit());
    let date_valid = |v: &str| match version {
        Some(4) => !digits(v, 8) || is_valid_ccyymmdd(v),
        Some(_) => !digits(v, 6) || is_valid_yymmdd(v),
        None => (!digits(v, 6) || is_valid_yymmdd(v)) && (!digits(v, 8) || is_valid_ccyymmdd(v)),
    };
    let time_valid = |v: &str| !digits(v, 4) || is_valid_hhmm(v);
    for (component, valid) in [(0, &date_valid as &dyn Fn(&str) -> bool), (1, &time_valid)] {
        let Some(value) = element.get_component(component) else {
            continue;
        };
        if !valid(value) {
            out.push(EdifactError::InvalidDateTime {
                tag: seg.tag().to_owned(),
                element_index: S004,
                component_index: component,
                value: value.to_owned(),
                span: element.component_span(component).unwrap_or(seg.span),
            });
        }
    }
}

/// Whether `year` is a Gregorian leap year.
pub(crate) const fn is_leap_year(year: u16) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

/// Days in `month` (1–12), `0` for any other month.
pub(crate) const fn days_in_month(month: u8, leap: bool) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    }
}

/// Whether `hour:minute` is a time on the 24-hour clock (00:00–23:59).
pub(crate) const fn is_valid_time(hour: u16, minute: u16) -> bool {
    hour < 24 && minute < 60
}

/// The value of `digits` (ASCII digits only), or `None`.
fn decimal(digits: &[u8]) -> Option<u16> {
    let mut value: u16 = 0;
    for &b in digits {
        if !b.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add(u16::from(b - b'0'))?;
    }
    Some(value)
}

/// `MMDD` of a year with leap status `leap`.
fn is_valid_month_day(mmdd: &[u8], leap: bool) -> bool {
    match (decimal(&mmdd[..2]), decimal(&mmdd[2..])) {
        (Some(month), Some(day)) => {
            day >= 1 && day <= u16::from(days_in_month(month as u8, leap)) && month <= 12
        }
        _ => false,
    }
}

/// `YYMMDD` (DE 0017, syntax versions 1–3). The century is not transferred, so
/// 29 February is valid in every year divisible by 4.
pub(crate) fn is_valid_yymmdd(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() == 6 && decimal(&b[..2]).is_some_and(|yy| is_valid_month_day(&b[2..], yy % 4 == 0))
}

/// `CCYYMMDD` (DE 0017, syntax version 4), Gregorian leap rule.
pub(crate) fn is_valid_ccyymmdd(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() == 8 && decimal(&b[..4]).is_some_and(|y| is_valid_month_day(&b[4..], is_leap_year(y)))
}

/// `HHMM` (DE 0019), 0000–2359.
pub(crate) fn is_valid_hhmm(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() == 4
        && match (decimal(&b[..2]), decimal(&b[2..])) {
            (Some(h), Some(m)) => is_valid_time(h, m),
            _ => false,
        }
}

/// A control count (`UNZ` 0036, `UNE` 0060, `UNT` 0074, element 1) of at most
/// `max_digits` digits to compare, or `None` to skip the comparison.
///
/// The comparison is skipped only when the value is already reported: absent
/// (`E008`), not numeric (`E048`) or too long (`E049`). A §10 numeric value that
/// is no unsigned integer (`-1`, `1.5`, `1E2`) passes the representation, so it
/// is reported here as `E048`. The count is exact: `max_digits` is at most 10.
fn control_count(seg: &Segment<'_>, max_digits: usize, sink: &mut ErrorSink) -> Option<u64> {
    let value = seg.component_str(0, 0).unwrap_or("");
    if value.is_empty() {
        return None;
    }
    if value.bytes().all(|b| b.is_ascii_digit()) {
        return (value.len() <= max_digits)
            .then(|| value.parse().ok())
            .flatten();
    }
    if crate::directory_validator::Repr::n_up_to(6).permits_characters(value) {
        sink.push(EdifactError::InvalidCharacterType {
            tag: seg.tag().to_owned(),
            element_index: 0,
            component_index: 0,
            repr: format!("n..{max_digits} (an unsigned count)"),
            value: value.to_owned(),
            span: seg
                .get_element(0)
                .and_then(|e| e.component_span(0))
                .unwrap_or(seg.span),
        });
    }
    None
}

// ── Interchange extraction ────────────────────────────────────────────────────

/// The `UNB`/`UNZ` data of `interchange` (from its `UNB` to its `UNZ`).
///
/// `None` when the typed header cannot be built; the reasons are in `sink`.
fn extract_interchange(
    interchange: &[Segment<'_>],
    sink: &mut ErrorSink,
) -> Option<InterchangeEnvelope> {
    let unb = &interchange[0];
    let unz = &interchange[interchange.len() - 1];

    // DE 0001 names a character repertoire, so `Charset` is the single source
    // of which values exist.
    let syntax_identifier = unb.component_str(0, 0).unwrap_or("");
    if !syntax_identifier.is_empty()
        && let Err(error) = crate::Charset::from_syntax_identifier(syntax_identifier)
    {
        sink.push(error);
    }

    let control_ref = unb.component_str(4, 0).unwrap_or("");
    let unz_control_ref = unz.component_str(1, 0).unwrap_or("");
    // An absent reference is already reported as missing.
    if !unz_control_ref.is_empty() && !control_ref.is_empty() && unz_control_ref != control_ref {
        sink.push(EdifactError::QualifierMismatch {
            tag: "UNZ".to_owned(),
            actual: unz_control_ref.to_owned(),
            expected: control_ref.to_owned(),
            span: unz.span,
        });
    }

    let header = InterchangeHeader::from_unb_lenient(unb);
    if header.is_none() && sink.errors.is_empty() {
        // Every reason the header cannot be typed is a finding of the checks
        // above; this keeps `None` from ever arriving without one.
        sink.push(InterchangeHeader::parse(unb).err()?);
    }
    Some(InterchangeEnvelope {
        header: header?,
        declared_unit_count: 0,
        actual_unit_count: 0,
    })
}

// ── Content extraction ────────────────────────────────────────────────────────

fn extract_content(
    inner: &[Segment<'_>],
    version: Option<u8>,
    sink: &mut ErrorSink,
) -> Result<(Vec<FunctionalGroupEnvelope>, Vec<MessageEnvelope>), EdifactError> {
    // A UNG as the first inner segment means the interchange uses functional
    // groups; refused segments in front of it are already reported.
    if inner
        .iter()
        .find(|s| !is_refused(s))
        .is_some_and(|s| s.tag == "UNG")
    {
        let groups = extract_with_groups(inner, version, sink)?;
        let messages = groups
            .iter()
            .flat_map(|g| g.messages.iter().cloned())
            .collect();
        Ok((groups, messages))
    } else {
        let mut seen_refs = HashSet::new();
        let messages = extract_messages_flat(inner, version, sink, &mut seen_refs)?;
        Ok((vec![], messages))
    }
}

/// Find the index of the `UNE` that closes the `UNG` opened just before `start`.
fn find_matching_une(segments: &[Segment<'_>], start: usize) -> Result<usize, EdifactError> {
    for (offset, seg) in segments[start..].iter().enumerate() {
        match seg.tag() {
            "UNE" => return Ok(start + offset),
            "UNG" => {
                return Err(EdifactError::InvalidSegmentForMessage {
                    tag: "UNG".to_owned(),
                    message_type: "ENVELOPE".to_owned(),
                    span: seg.span,
                });
            }
            _ => {}
        }
    }
    Err(EdifactError::MissingSegment {
        tag: "UNE".to_owned(),
        expected_position: "end of functional group".to_owned(),
    })
}

fn extract_with_groups(
    inner: &[Segment<'_>],
    version: Option<u8>,
    sink: &mut ErrorSink,
) -> Result<Vec<FunctionalGroupEnvelope>, EdifactError> {
    let mut groups: Vec<FunctionalGroupEnvelope> = Vec::new();
    // DE 0048 must be unique within the interchange (ISO 9735-1 §7.3); DE 0062
    // must be unique across the whole interchange, so the set spans all groups.
    let mut seen_group_refs: HashSet<String> = HashSet::new();
    let mut seen_message_refs: HashSet<String> = HashSet::new();
    let mut i = 0;

    while i < inner.len() {
        let seg = &inner[i];
        match seg.tag() {
            "UNG" => {
                let ung_idx = i;
                let une_idx = find_matching_une(inner, ung_idx + 1)?;

                let ung = &inner[ung_idx];
                // UNG DE 0048 — group reference number, the only mandatory element.
                let group_ref = text(ung, 4, 0);
                if !group_ref.is_empty() && !seen_group_refs.insert(group_ref.clone()) {
                    sink.push(EdifactError::DuplicateReference {
                        tag: "UNG".to_owned(),
                        reference: group_ref.clone(),
                        span: ung.span,
                    });
                }

                let une = &inner[une_idx];
                let une_ref = text(une, 1, 0);
                if !une_ref.is_empty() && !group_ref.is_empty() && une_ref != group_ref {
                    sink.push(EdifactError::QualifierMismatch {
                        tag: "UNE".to_owned(),
                        actual: une_ref,
                        expected: group_ref.clone(),
                        span: une.span,
                    });
                }

                let group_content = &inner[ung_idx + 1..une_idx];
                let messages =
                    extract_messages_flat(group_content, version, sink, &mut seen_message_refs)?;
                let actual_message_count = sink.recover(
                    u32::try_from(messages.len()).map_err(|_| EdifactError::InterchangeTooLarge {
                        count: messages.len() as u64,
                    }),
                    u32::MAX,
                );

                // ISO 9735-1:2002 §7.3: a group "shall contain at least one
                // message or package".
                if messages.is_empty() {
                    sink.push(EdifactError::EmptyGroup {
                        group_ref: group_ref.clone(),
                        span: ung.span,
                    });
                }

                // n..6: always fits `u32`.
                let declared = control_count(une, 6, sink).and_then(|d| u32::try_from(d).ok());
                if let Some(declared) = declared
                    && declared != actual_message_count
                {
                    sink.push(EdifactError::GroupCountMismatch {
                        group_ref: group_ref.clone(),
                        expected: declared,
                        actual: actual_message_count,
                        span: une.span,
                    });
                }

                let optional = |element: usize| {
                    ung.component_str(element, 0)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                };
                groups.push(FunctionalGroupEnvelope {
                    group_id: text(ung, 0, 0),
                    app_sender: text(ung, 1, 0),
                    app_sender_qualifier: text(ung, 1, 1),
                    app_recipient: text(ung, 2, 0),
                    app_recipient_qualifier: text(ung, 2, 1),
                    date: text(ung, 3, 0),
                    time: ung
                        .component_str(3, 1)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned),
                    group_ref,
                    controlling_agency: text(ung, 5, 0),
                    // UNG S008 — version (DE 0052) + release (DE 0054).
                    version: text(ung, 6, 0),
                    release: text(ung, 6, 1),
                    // UNG DE 0058 — application password.
                    application_password: optional(7),
                    declared_message_count: declared.unwrap_or(0),
                    actual_message_count,
                    messages,
                });
                i = une_idx + 1;
            }
            "UNE" => {
                return Err(EdifactError::InvalidSegmentForMessage {
                    tag: "UNE".to_owned(),
                    message_type: "ENVELOPE".to_owned(),
                    span: seg.span,
                });
            }
            "UNH" => {
                // §7.1: an interchange holds groups or ungrouped messages, never
                // both — a message outside any group has no group to be counted
                // in.  `CONTRL` has a code for exactly this (30), and §5.3.3
                // asks for the precise one over the general.
                return Err(EdifactError::GroupsAndMessagesMixed { span: seg.span });
            }
            "UNO" | "UNP" => {
                return Err(EdifactError::PackageNotSupported {
                    tag: seg.tag.clone().into_owned(),
                    span: seg.span,
                });
            }
            // Already reported by its tag class.
            _ if is_refused(seg) => i += 1,
            _ => {
                return Err(EdifactError::InvalidSegmentForMessage {
                    tag: seg.tag.clone().into_owned(),
                    message_type: "ENVELOPE".to_owned(),
                    span: seg.span,
                });
            }
        }
    }

    Ok(groups)
}

/// Extract `UNH`/`UNT` message pairs from a flat slice (no UNB/UNZ/UNG/UNE expected).
fn extract_messages_flat(
    segments: &[Segment<'_>],
    version: Option<u8>,
    sink: &mut ErrorSink,
    seen_refs: &mut HashSet<String>,
) -> Result<Vec<MessageEnvelope>, EdifactError> {
    let mut messages: Vec<MessageEnvelope> = Vec::new();
    // Index of the `UNH` that opened the message currently being read.  A single
    // `Option` carries the whole "are we inside a message" state, so the index
    // cannot be read while absent.
    let mut unh_idx: Option<usize> = None;
    // DE 0074 is n..6 in syntax versions 1–3 and n..10 in version 4.
    let segment_count_digits = match version {
        Some(1..=3) => 6,
        _ => 10,
    };

    for (i, seg) in segments.iter().enumerate() {
        match seg.tag() {
            "UNH" => {
                if unh_idx.is_some() {
                    return Err(EdifactError::InvalidSegmentForMessage {
                        tag: "UNH".to_owned(),
                        message_type: "ENVELOPE".to_owned(),
                        span: seg.span,
                    });
                }
                unh_idx = Some(i);
            }
            "UNT" if unh_idx.is_some() => {
                let msg_start_idx = unh_idx.take().expect("guarded by the match arm");
                let unh = &segments[msg_start_idx];

                let message_ref = text(unh, 0, 0);
                if !message_ref.is_empty() && !seen_refs.insert(message_ref.clone()) {
                    sink.push(EdifactError::DuplicateReference {
                        tag: "UNH".to_owned(),
                        reference: message_ref.clone(),
                        span: unh.span,
                    });
                }

                let unt_ref = text(seg, 1, 0);
                if !unt_ref.is_empty() && !message_ref.is_empty() && unt_ref != message_ref {
                    sink.push(EdifactError::QualifierMismatch {
                        tag: "UNT".to_owned(),
                        actual: unt_ref,
                        expected: message_ref.clone(),
                        span: seg.span,
                    });
                }

                let segment_span = i - msg_start_idx + 1;
                let actual_segment_count = segment_span as u64;
                let declared = control_count(seg, segment_count_digits, sink);
                if let Some(declared) = declared.filter(|d| *d != actual_segment_count) {
                    sink.push(EdifactError::SegmentCountMismatch {
                        expected: declared,
                        actual: actual_segment_count,
                        message_ref: message_ref.clone(),
                        span: seg.span,
                    });
                }

                // ISO 9735-1 §7.4: a message "shall contain at least one
                // additional segment" beyond its header and trailer.  A bare
                // `UNH'UNT+2+…'` is a well-formed envelope around nothing, and
                // its own segment count confirms it — so only this rule sees it.
                if segment_span < 3 {
                    sink.push(EdifactError::EmptyMessage {
                        message_ref: message_ref.clone(),
                        span: unh.span,
                    });
                }

                messages.push(MessageEnvelope {
                    message_ref,
                    message_type: text(unh, 1, 0),
                    version: text(unh, 1, 1),
                    release: text(unh, 1, 2),
                    controlling_agency: text(unh, 1, 3),
                    association_code: text(unh, 1, 4),
                    // UNH element [2]: DE 0068 — common access reference
                    common_access_ref: unh
                        .component_str(2, 0)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned),
                    // UNH element [3]: S010 — sequence of transfers (DE 0070)
                    // and first/last transfer (DE 0073)
                    sequence_of_transfers: unh
                        .component_str(3, 0)
                        .filter(|s| !s.is_empty())
                        .and_then(|s| s.parse::<u32>().ok()),
                    transfer_position: unh
                        .component_str(3, 1)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned),
                    declared_segment_count: declared.unwrap_or(0),
                    actual_segment_count,
                    header_span: unh.span,
                    trailer_span: seg.span,
                });
            }
            "UNT" => {
                return Err(EdifactError::InvalidSegmentForMessage {
                    tag: "UNT".to_owned(),
                    message_type: "ENVELOPE".to_owned(),
                    span: seg.span,
                });
            }
            // A package is a legal member of an interchange (ISO 9735-1 §7.10),
            // so reporting it as a stray segment would send the reader looking
            // for a corruption that is not there.  The object between UNO and
            // UNP is arbitrary binary and is not EDIFACT-encoded, which is the
            // reason this crate declines it rather than the reason it is invalid.
            "UNO" | "UNP" => {
                return Err(EdifactError::PackageNotSupported {
                    tag: seg.tag.clone().into_owned(),
                    span: seg.span,
                });
            }
            "UNB" | "UNZ" | "UNG" | "UNE" if unh_idx.is_some() => {
                return Err(EdifactError::InvalidSegmentForMessage {
                    tag: seg.tag.clone().into_owned(),
                    message_type: "ENVELOPE".to_owned(),
                    span: seg.span,
                });
            }
            // Already reported by its tag class.
            _ if unh_idx.is_none() && is_refused(seg) => {}
            _ if unh_idx.is_none() => {
                return Err(EdifactError::InvalidSegmentForMessage {
                    tag: seg.tag.clone().into_owned(),
                    message_type: "ENVELOPE".to_owned(),
                    span: seg.span,
                });
            }
            _ => {}
        }
    }

    if unh_idx.is_some() {
        return Err(EdifactError::MissingSegment {
            tag: "UNT".to_owned(),
            expected_position: "end of message group".to_owned(),
        });
    }

    Ok(messages)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &[u8]) -> Vec<crate::OwnedSegment> {
        crate::from_reader(std::io::Cursor::new(input))
            .collect::<Result<Vec<_>, _>>()
            .expect("parse failed")
    }

    fn parse_and_validate(input: &[u8]) -> Result<ValidatedInterchange, EdifactError> {
        validate_envelope(&parse(input))
    }

    fn parse_and_validate_lenient(input: &[u8]) -> LenientResult {
        validate_envelope_lenient(&parse(input))
    }

    #[test]
    fn lenient_collects_every_violation_not_just_the_first() {
        // Two independent violations: a UNZ control-reference mismatch and a
        // UNT segment-count mismatch.  Lenient validation must report both.
        let input = b"UNB+UNOA:1+S+R+200101:0900+CTRL1'\
                      UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+99+1'\
                      UNZ+1+CTRL2'";
        let result = parse_and_validate_lenient(input);
        assert!(
            result.errors.len() >= 2,
            "expected both violations, got {:?}",
            result.errors
        );
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, EdifactError::QualifierMismatch { tag, .. } if tag == "UNZ")),
            "missing UNZ control-ref mismatch in {:?}",
            result.errors
        );
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, EdifactError::SegmentCountMismatch { .. })),
            "missing UNT segment-count mismatch in {:?}",
            result.errors
        );
    }

    #[test]
    fn strict_reports_the_first_error_lenient_collects() {
        // Strict and lenient share one implementation, so strict's error must
        // always be the head of the lenient error list.
        for input in [
            &b"UNB+UNOA:1+S+R+200101:0900+C1'UNH+1+ORDERS:D:96A:UN'UNT+2+1'UNZ+9+C1'"[..],
            &b"UNB+UNOA:1+S+R+200101:0900+C1'UNH+1+ORDERS:D:96A:UN'UNT+99+1'UNZ+1+C1'"[..],
            &b"UNB+UNOA:1+S+R+200101:0900+C1'UNH+1+ORDERS:D:96A:UN'UNT+2+9'UNZ+1+C1'"[..],
        ] {
            let strict = parse_and_validate(input);
            let lenient = parse_and_validate_lenient(input);
            match strict {
                Err(e) => assert_eq!(
                    Some(&e),
                    lenient.errors.first(),
                    "strict/lenient diverged for {:?}",
                    std::str::from_utf8(input).unwrap()
                ),
                Ok(_) => assert!(lenient.errors.is_empty()),
            }
        }
    }

    #[test]
    fn duplicate_message_references_are_rejected() {
        let input = b"UNB+UNOA:1+S+R+200101:0900+C1'\
                      UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+3+1'\
                      UNH+1+ORDERS:D:96A:UN'BGM+221'UNT+3+1'\
                      UNZ+2+C1'";
        let err = parse_and_validate(input).expect_err("duplicate UNH refs must fail");
        assert!(
            matches!(&err, EdifactError::DuplicateReference { tag, reference, .. }
                     if tag == "UNH" && reference == "1"),
            "got {err:?}"
        );
    }

    #[test]
    fn distinct_message_references_are_accepted() {
        let input = b"UNB+UNOA:1+S+R+200101:0900+C1'\
                      UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+3+1'\
                      UNH+2+ORDERS:D:96A:UN'BGM+221'UNT+3+2'\
                      UNZ+2+C1'";
        parse_and_validate(input).expect("distinct refs must validate");
    }

    #[test]
    fn into_strict_is_total_after_the_caller_filters_errors() {
        // `errors` is a public field, so filtering tolerated violations before
        // converting must not panic.
        let input = b"UNB+UNOA:1+S+R+200101:0900+C1'UNZ+1+C2'";
        let mut lenient = parse_and_validate_lenient(input);
        lenient.errors.clear();
        // Either outcome is acceptable; the contract is only that it does not panic.
        let _ = lenient.into_strict();
    }

    fn parse_and_validate_owned(input: &[u8]) -> Result<ValidatedInterchange, EdifactError> {
        validate_envelope(&parse(input))
    }

    const VALID_INTERCHANGE: &[u8] =
        b"UNA:+.? 'UNB+UNOA:3+SENDER::293+RECEIVER::293+230401:0900+00001'UNH+00001+ORDERS:D:11A:UN:EAN010'BGM+220+PO-4711+9'DTM+137:20230401:102'UNT+4+00001'UNZ+1+00001'";

    #[test]
    fn valid_envelope_parses_ok() {
        let result = parse_and_validate(VALID_INTERCHANGE).expect("envelope should be valid");
        assert_eq!(result.interchange.header.sender.id, "SENDER");
        assert_eq!(
            result
                .interchange
                .header
                .sender
                .qualifier
                .as_deref()
                .unwrap_or(""),
            ""
        ); // no qualifier in fixture
        assert_eq!(result.interchange.header.recipient.id, "RECEIVER");
        assert_eq!(
            result
                .interchange
                .header
                .recipient
                .qualifier
                .as_deref()
                .unwrap_or(""),
            ""
        );
        assert_eq!(
            result.interchange.header.charset.syntax_identifier(),
            "UNOA"
        );
        assert_eq!(
            result
                .interchange
                .header
                .syntax_version
                .number()
                .to_string(),
            "3"
        );
        assert_eq!(result.interchange.header.control_ref, "00001");
        assert_eq!(result.interchange.declared_unit_count, 1);
        assert_eq!(result.interchange.actual_unit_count, 1);
        assert!(!result.interchange.is_test());
        assert!(!result.has_functional_groups());
        assert_eq!(result.message_count(), 1);
        assert_eq!(result.messages[0].message_type, "ORDERS");
        assert_eq!(result.messages[0].association_code, "EAN010");
        assert_eq!(result.messages[0].declared_segment_count, 4);
        assert_eq!(result.messages[0].actual_segment_count, 4);
    }

    #[test]
    fn valid_envelope_parses_ok_owned_path() {
        let result = parse_and_validate_owned(VALID_INTERCHANGE).expect("envelope should be valid");
        assert_eq!(result.interchange.header.sender.id, "SENDER");
        assert_eq!(result.interchange.actual_unit_count, 1);
        assert_eq!(result.messages[0].declared_segment_count, 4);
    }

    #[test]
    fn unt_count_mismatch_returns_err() {
        let input = b"UNB+UNOA:3+S+R+200101:0900+1'UNH+1+ORDERS:D:11A:UN:EAN010'BGM+220+PO-1+9'DTM+137:20200101:102'UNT+99+1'UNZ+1+1'";
        let result = parse_and_validate(input);
        assert!(
            matches!(
                result,
                Err(EdifactError::SegmentCountMismatch { expected: 99, .. })
            ),
            "expected SegmentCountMismatch, got {result:?}"
        );
    }

    #[test]
    fn unz_count_mismatch_returns_err() {
        let input = b"UNB+UNOA:3+S+R+200101:0900+1'UNH+1+ORDERS:D:11A:UN:EAN010'BGM+220+PO-1+9'UNT+3+1'UNZ+2+1'";
        let result = parse_and_validate(input);
        assert!(
            matches!(
                result,
                Err(EdifactError::MessageCountMismatch {
                    expected: 2,
                    actual: 1,
                    ..
                })
            ),
            "expected MessageCountMismatch(2,1), got {result:?}"
        );
    }

    #[test]
    fn missing_unb_returns_err() {
        let input = b"UNH+1+ORDERS:D:11A:UN:EAN010'BGM+220+PO-1+9'UNT+3+1'UNZ+1+1'";
        assert!(parse_and_validate(input).is_err());
    }

    #[test]
    fn extracts_una_interchange_correctly() {
        let result = parse_and_validate(VALID_INTERCHANGE).unwrap();
        assert_eq!(
            result.interchange.header.charset.syntax_identifier(),
            "UNOA"
        );
        assert_eq!(
            result
                .interchange
                .header
                .syntax_version
                .number()
                .to_string(),
            "3"
        );
        assert_eq!(
            result.interchange.header.prepared,
            crate::PreparationDateTime::from_two_digit_year(23, 4, 1, 9, 0).unwrap()
        );
    }

    #[test]
    fn sender_and_recipient_qualifiers_extracted() {
        // GLN-qualified partners: 1234567890123:14 — qualifier at S002 comp 1
        let input = b"UNB+UNOA:3+1234567890123:14+9876543210987:14+200101:0900+1'\
                      UNH+1+ORDERS:D:96A:UN'\
                      BGM+220+PO-001+9'\
                      UNT+3+1'\
                      UNZ+1+1'";
        let r = parse_and_validate(input).expect("GLN-qualified UNB must parse ok");
        assert_eq!(r.interchange.header.sender.id, "1234567890123");
        assert_eq!(
            r.interchange
                .header
                .sender
                .qualifier
                .as_deref()
                .unwrap_or(""),
            "14"
        );
        assert_eq!(r.interchange.header.recipient.id, "9876543210987");
        assert_eq!(
            r.interchange
                .header
                .recipient
                .qualifier
                .as_deref()
                .unwrap_or(""),
            "14"
        );
    }

    // UNB DE 0026 (app_ref) is at element index 6 (ISO 9735-10:2002 §5.1.5, UNB):
    // [4]=control_ref [5]=S005/password [6]=0026/app_ref [7]=0029 [8]=0031/ack [9]=0032/comms [10]=0035/test

    #[test]
    fn test_indicator_parsed_from_unb() {
        // DE 0035 (test indicator) is at element index 10 (ISO 9735-1).
        // UNB+...+1 (ctrl) + (S005) + (0026) + (0029) + (0031) + (0032) + 1 (0035)
        //                    [5]       [6]       [7]       [8]       [9]     [10]
        let input = b"UNB+UNOA:3+S+R+200101:0900+1++++++1'\
                      UNH+1+ORDERS:D:96A:UN'\
                      BGM+220+PO-001+9'\
                      UNT+3+1'\
                      UNZ+1+1'";
        let r = parse_and_validate(input).expect("test-flagged UNB must parse ok");
        assert!(
            r.interchange.header.test_indicator,
            "test_indicator should be true"
        );
        assert!(r.interchange.is_test(), "is_test() convenience must agree");
    }

    #[test]
    fn no_test_indicator_defaults_false() {
        let r = parse_and_validate(VALID_INTERCHANGE).unwrap();
        assert!(!r.interchange.header.test_indicator);
        assert!(!r.interchange.is_test());
    }

    #[test]
    fn app_ref_extracted_when_present() {
        // DE 0026 (app_ref) is at element index 6; element [5] (S005 password) is empty.
        let input = b"UNB+UNOA:3+S+R+200101:0900+1++MYAPP'\
                      UNH+1+ORDERS:D:96A:UN'\
                      BGM+220+PO-001+9'\
                      UNT+3+1'\
                      UNZ+1+1'";
        let r = parse_and_validate(input).expect("UNB with app_ref must parse ok");
        assert_eq!(
            r.interchange.header.application_ref.as_deref(),
            Some("MYAPP")
        );
    }

    #[test]
    fn app_ref_is_none_when_absent() {
        let r = parse_and_validate(VALID_INTERCHANGE).unwrap();
        assert!(r.interchange.header.application_ref.is_none());
    }

    #[test]
    fn recipient_password_extracted_when_present() {
        // DE 0022 (recipient password) is at element index 5 (S005 comp 0).
        let input = b"UNB+UNOA:3+S+R+200101:0900+1+MYPASS'\
                      UNH+1+ORDERS:D:96A:UN'\
                      BGM+220+PO-001+9'\
                      UNT+3+1'\
                      UNZ+1+1'";
        let r = parse_and_validate(input).expect("UNB with password must parse ok");
        assert_eq!(
            r.interchange.header.recipient_password.as_deref(),
            Some("MYPASS")
        );
    }

    #[test]
    fn recipient_password_is_none_when_absent() {
        let r = parse_and_validate(VALID_INTERCHANGE).unwrap();
        assert!(r.interchange.header.recipient_password.is_none());
    }

    #[test]
    fn acknowledgement_request_flag_parsed() {
        // DE 0031 (ack request) at element index 8; set to "1".
        // Elements: [5]S005="" [6]0026="" [7]0029="" [8]0031="1"
        let input = b"UNB+UNOA:3+S+R+200101:0900+1++++1'\
                      UNH+1+ORDERS:D:96A:UN'\
                      BGM+220+PO-001+9'\
                      UNT+3+1'\
                      UNZ+1+1'";
        let r = parse_and_validate(input).expect("UNB with ack-request must parse ok");
        assert!(r.interchange.header.acknowledgement_requested);
        assert!(r.interchange.ack_requested());
    }

    #[test]
    fn acknowledgement_request_defaults_false() {
        let r = parse_and_validate(VALID_INTERCHANGE).unwrap();
        assert!(!r.interchange.header.acknowledgement_requested);
        assert!(!r.interchange.ack_requested());
    }

    #[test]
    fn communications_agreement_id_extracted() {
        // DE 0032 at element index 9; elements [5]-[8] empty.
        let input = b"UNB+UNOA:3+S+R+200101:0900+1+++++AGREEMENT-1'\
                      UNH+1+ORDERS:D:96A:UN'\
                      BGM+220+PO-001+9'\
                      UNT+3+1'\
                      UNZ+1+1'";
        let r = parse_and_validate(input).expect("UNB with comms-agreement must parse ok");
        assert_eq!(
            r.interchange.header.communications_agreement_id.as_deref(),
            Some("AGREEMENT-1")
        );
    }

    #[test]
    fn dangling_unh_without_unt_returns_err() {
        let input =
            b"UNB+UNOA:3+S+R+200101:0900+1'UNH+1+ORDERS:D:11A:UN:EAN010'BGM+220+PO-1+9'UNZ+1+1'";
        let result = parse_and_validate(input);
        assert!(
            matches!(result, Err(EdifactError::MissingSegment { ref tag, .. }) if tag == "UNT")
        );
    }

    #[test]
    fn stray_segment_outside_message_returns_err() {
        let input = b"UNB+UNOA:3+S+R+200101:0900+1'UNH+1+ORDERS:D:11A:UN:EAN010'BGM+220+PO-1+9'UNT+3+1'BGM+999+PO-2+9'UNZ+1+1'";
        assert!(matches!(
            parse_and_validate(input),
            Err(EdifactError::InvalidSegmentForMessage { .. })
        ));
    }

    #[test]
    fn missing_unb_sender_component_returns_err() {
        let input = b"UNB+UNOA:3++R+200101:0900+1'UNH+1+ORDERS:D:11A:UN:EAN010'BGM+220+PO-1+9'UNT+3+1'UNZ+1+1'";
        let result = parse_and_validate(input);
        assert!(
            matches!(result, Err(EdifactError::MissingRequiredElement { ref tag, element_index: 1 }) if tag == "UNB"),
            "expected MissingRequiredElement for empty S002, got: {result:?}"
        );
    }

    #[test]
    fn nested_unh_without_closing_previous_message_returns_err() {
        let input = b"UNB+UNOA:3+S+R+200101:0900+1'UNH+1+ORDERS:D:11A:UN:EAN010'BGM+220+PO-1+9'UNH+2+ORDERS:D:11A:UN:EAN010'UNT+3+2'UNZ+1+1'";
        let result = parse_and_validate(input);
        assert!(
            matches!(result, Err(EdifactError::InvalidSegmentForMessage { ref tag, .. }) if tag == "UNH"),
            "expected InvalidSegmentForMessage(UNH), got {result:?}"
        );
    }

    #[test]
    fn unt_message_reference_must_match_unh() {
        let input = b"UNB+UNOA:3+S+R+200101:0900+1'UNH+1+ORDERS:D:11A:UN:EAN010'BGM+220+PO-1+9'UNT+3+999'UNZ+1+1'";
        let result = parse_and_validate(input);
        assert!(
            matches!(result, Err(EdifactError::QualifierMismatch { ref tag, .. }) if tag == "UNT")
        );
    }

    #[test]
    fn unz_control_reference_must_match_unb() {
        let input = b"UNB+UNOA:3+S+R+200101:0900+1'UNH+1+ORDERS:D:11A:UN:EAN010'BGM+220+PO-1+9'UNT+3+1'UNZ+1+999'";
        let result = parse_and_validate(input);
        assert!(
            matches!(result, Err(EdifactError::QualifierMismatch { ref tag, .. }) if tag == "UNZ")
        );
    }

    #[test]
    fn missing_unh_message_type_components_return_err() {
        let input =
            b"UNB+UNOA:3+S+R+200101:0900+1'UNH+1+ORDERS:D:11A'BGM+220+PO-1+9'UNT+3+1'UNZ+1+1'";
        let result = parse_and_validate(input);
        assert!(
            matches!(result, Err(EdifactError::MissingRequiredComponent { ref tag, element_index: 1, component_index: 3 }) if tag == "UNH"),
            "got: {result:?}"
        );
    }

    #[test]
    fn nested_unz_inside_message_returns_err() {
        let input =
            b"UNB+UNOA:3+S+R+200101:0900+1'UNH+1+ORDERS:D:11A:UN:EAN010'UNZ+1+1'UNT+2+1'UNZ+1+1'";
        let result = parse_and_validate(input);
        assert!(
            matches!(result, Err(EdifactError::InvalidSegmentForMessage { ref tag, .. }) if tag == "UNZ")
        );
    }

    #[test]
    fn lenient_returns_partial_result_on_count_mismatch() {
        // UNZ says 2 but only 1 message — lenient mode must return Some(partial)
        // along with the error, not None.
        let input = b"UNB+UNOA:3+S+R+200101:0900+1'UNH+1+ORDERS:D:11A:UN:EAN010'BGM+220+PO-1+9'UNT+3+1'UNZ+2+1'";
        let owned = parse(input);
        let lenient = validate_envelope_lenient(&owned);
        let result = lenient.interchange;
        let errors = lenient.errors;
        assert!(
            result.is_some(),
            "lenient mode must return Some even on count mismatch"
        );
        assert_eq!(errors.len(), 1);
        assert!(
            matches!(
                &errors[0],
                EdifactError::MessageCountMismatch {
                    expected: 2,
                    actual: 1,
                    ..
                }
            ),
            "expected MessageCountMismatch(2,1), got {:?}",
            errors[0]
        );
        let partial = result.unwrap();
        assert_eq!(partial.messages.len(), 1);
        assert_eq!(partial.interchange.actual_unit_count, 1);
        assert_eq!(partial.interchange.declared_unit_count, 2);
    }

    #[test]
    fn lenient_returns_none_on_structural_error() {
        // Missing UNB — no structure at all, expect None
        let input = b"UNH+1+ORDERS:D:11A:UN:EAN010'BGM+220+PO-1+9'UNT+3+1'UNZ+1+1'";
        let owned = parse(input);
        let lenient = validate_envelope_lenient(&owned);
        let result = lenient.interchange;
        let errors = lenient.errors;
        assert!(result.is_none(), "missing UNB must yield None");
        assert!(!errors.is_empty());
    }

    #[test]
    fn message_count_convenience_method() {
        let r = parse_and_validate(VALID_INTERCHANGE).unwrap();
        assert_eq!(r.message_count(), r.messages.len());
        assert_eq!(r.message_count(), 1);
    }

    #[test]
    fn interchange_with_single_functional_group_parses_ok() {
        let input = b"UNB+UNOA:3+S+R+200101:0900+1'\
                      UNG+ORDERS+S+R+200101:0900+1+UN+D:96A'\
                      UNH+1+ORDERS:D:96A:UN'\
                      BGM+220+PO-001+9'\
                      UNT+3+1'\
                      UNE+1+1'\
                      UNZ+1+1'";
        let result = parse_and_validate(input).expect("single-group interchange must parse ok");
        assert!(result.has_functional_groups());
        assert_eq!(result.functional_groups.len(), 1);
        let g = &result.functional_groups[0];
        assert_eq!(g.group_id, "ORDERS");
        assert_eq!(g.group_ref, "1");
        assert_eq!(g.controlling_agency, "UN");
        assert_eq!(g.declared_message_count, 1);
        assert_eq!(g.actual_message_count, 1);
        assert_eq!(result.messages.len(), 1);
        assert_eq!(result.messages[0].message_type, "ORDERS");
        assert_eq!(result.interchange.declared_unit_count, 1);
        assert_eq!(result.interchange.actual_unit_count, 1);
    }

    #[test]
    fn interchange_with_multi_message_group_parses_ok() {
        // One group containing 2 messages — UNZ = 1 group, UNE = 2 messages.
        // This is the key case that strip_functional_group_segments breaks.
        let input = b"UNB+UNOA:3+S+R+200101:0900+1'\
                      UNG+ORDERS+S+R+200101:0900+1+UN+D:96A'\
                      UNH+1+ORDERS:D:96A:UN'\
                      BGM+220+PO-001+9'\
                      UNT+3+1'\
                      UNH+2+ORDERS:D:96A:UN'\
                      BGM+220+PO-002+9'\
                      UNT+3+2'\
                      UNE+2+1'\
                      UNZ+1+1'";
        let result = parse_and_validate(input).expect("multi-message group must parse ok");
        assert!(result.has_functional_groups());
        assert_eq!(result.functional_groups.len(), 1);
        assert_eq!(result.functional_groups[0].actual_message_count, 2);
        assert_eq!(result.messages.len(), 2);
        // UNZ = 1 group (not 2 messages): ISO 9735-10:2002 §5.3.5 (DE 0036)
        assert_eq!(result.interchange.actual_unit_count, 1);
        assert_eq!(result.interchange.declared_unit_count, 1);
    }

    #[test]
    fn interchange_with_multiple_groups_parses_ok() {
        let input = b"UNB+UNOA:3+S+R+200101:0900+2'\
                      UNG+ORDERS+S+R+200101:0900+1+UN+D:96A'\
                      UNH+1+ORDERS:D:96A:UN'\
                      BGM+220+PO-001+9'\
                      UNT+3+1'\
                      UNE+1+1'\
                      UNG+INVOIC+S+R+200101:0900+2+UN+D:96A'\
                      UNH+2+INVOIC:D:96A:UN'\
                      BGM+380+INV-001+9'\
                      UNT+3+2'\
                      UNE+1+2'\
                      UNZ+2+2'";
        let result = parse_and_validate(input).expect("multi-group interchange must parse ok");
        assert_eq!(result.functional_groups.len(), 2);
        assert_eq!(result.functional_groups[0].group_id, "ORDERS");
        assert_eq!(result.functional_groups[1].group_id, "INVOIC");
        assert_eq!(result.messages.len(), 2);
        assert_eq!(result.interchange.declared_unit_count, 2);
        assert_eq!(result.interchange.actual_unit_count, 2);
    }

    #[test]
    fn ung_une_count_mismatch_returns_err() {
        let input = b"UNB+UNOA:3+S+R+200101:0900+1'\
                      UNG+ORDERS+S+R+200101:0900+1+UN+D:96A'\
                      UNH+1+ORDERS:D:96A:UN'\
                      BGM+220+PO-001+9'\
                      UNT+3+1'\
                      UNE+2+1'\
                      UNZ+1+1'";
        let result = parse_and_validate(input);
        assert!(
            matches!(
                &result,
                Err(EdifactError::GroupCountMismatch {
                    group_ref,
                    expected: 2,
                    actual: 1,
                    ..
                }) if group_ref == "1"
            ),
            "expected GroupCountMismatch(1, 2, 1), got {result:?}"
        );
    }

    #[test]
    fn une_without_ung_returns_err() {
        let input = b"UNB+UNOA:3+S+R+200101:0900+1'\
                      UNE+1+1'\
                      UNH+1+ORDERS:D:96A:UN'\
                      BGM+220+PO-001+9'\
                      UNT+3+1'\
                      UNZ+1+1'";
        let result = parse_and_validate(input);
        assert!(
            matches!(result, Err(EdifactError::InvalidSegmentForMessage { ref tag, .. }) if tag == "UNE"),
            "expected InvalidSegmentForMessage(UNE), got {result:?}"
        );
    }

    #[test]
    fn ung_without_une_returns_err() {
        let input = b"UNB+UNOA:3+S+R+200101:0900+1'\
                      UNG+ORDERS+S+R+200101:0900+1+UN+D:96A'\
                      UNH+1+ORDERS:D:96A:UN'\
                      BGM+220+PO-001+9'\
                      UNT+3+1'\
                      UNZ+1+1'";
        let result = parse_and_validate(input);
        assert!(
            matches!(result, Err(EdifactError::MissingSegment { ref tag, .. }) if tag == "UNE"),
            "expected MissingSegment(UNE), got {result:?}"
        );
    }

    #[test]
    fn une_group_ref_must_match_ung() {
        let input = b"UNB+UNOA:3+S+R+200101:0900+1'\
                      UNG+ORDERS+S+R+200101:0900+1+UN+D:96A'\
                      UNH+1+ORDERS:D:96A:UN'\
                      BGM+220+PO-001+9'\
                      UNT+3+1'\
                      UNE+1+999'\
                      UNZ+1+1'";
        let result = parse_and_validate(input);
        assert!(
            matches!(result, Err(EdifactError::QualifierMismatch { ref tag, .. }) if tag == "UNE"),
            "expected QualifierMismatch(UNE), got {result:?}"
        );
    }

    // ── New-field tests (ISO 9735-1 completeness) ─────────────────────────────

    #[test]
    fn processing_priority_extracted_when_present() {
        // DE 0029 at element index 7: [5]=S005="" [6]=0026="" [7]=0029="A"
        let input = b"UNB+UNOA:3+S+R+200101:0900+1+++A'\
                      UNH+1+ORDERS:D:96A:UN'\
                      BGM+220+PO-001+9'\
                      UNT+3+1'\
                      UNZ+1+1'";
        let r = parse_and_validate(input).expect("UNB with processing_priority must parse ok");
        assert_eq!(
            r.interchange.header.processing_priority.as_deref(),
            Some("A")
        );
    }

    #[test]
    fn processing_priority_is_none_when_absent() {
        let r = parse_and_validate(VALID_INTERCHANGE).unwrap();
        assert!(r.interchange.header.processing_priority.is_none());
    }

    #[test]
    fn sender_routing_address_extracted_when_present() {
        // S002: sender_id:qualifier:routing  → comp[2] = routing address
        let input = b"UNB+UNOA:3+SENDER:14:ROUTEA+RECIP+200101:0900+1'\
                      UNH+1+ORDERS:D:96A:UN'\
                      BGM+220+PO-001+9'\
                      UNT+3+1'\
                      UNZ+1+1'";
        let r = parse_and_validate(input).expect("UNB with sender routing must parse ok");
        assert_eq!(
            r.interchange.header.sender.internal_id.as_deref(),
            Some("ROUTEA")
        );
        assert!(r.interchange.header.recipient.internal_id.is_none());
    }

    #[test]
    fn recipient_routing_address_extracted_when_present() {
        // S003: recipient_id:qualifier:routing → comp[2] = routing address
        let input = b"UNB+UNOA:3+SENDER+RECIP:14:ROUTEB+200101:0900+1'\
                      UNH+1+ORDERS:D:96A:UN'\
                      BGM+220+PO-001+9'\
                      UNT+3+1'\
                      UNZ+1+1'";
        let r = parse_and_validate(input).expect("UNB with recipient routing must parse ok");
        assert!(r.interchange.header.sender.internal_id.is_none());
        assert_eq!(
            r.interchange.header.recipient.internal_id.as_deref(),
            Some("ROUTEB")
        );
    }

    #[test]
    fn routing_address_is_none_when_absent() {
        // Plain S+R without sub-components — no routing addresses in S002/S003
        let input = b"UNB+UNOA:3+S+R+200101:0900+1'\
                      UNH+1+ORDERS:D:96A:UN'\
                      BGM+220+PO-001+9'\
                      UNT+3+1'\
                      UNZ+1+1'";
        let r = parse_and_validate(input).unwrap();
        assert!(r.interchange.header.sender.internal_id.is_none());
        assert!(r.interchange.header.recipient.internal_id.is_none());
    }

    #[test]
    fn common_access_ref_extracted_when_present() {
        // UNH element [2] (DE 0068): common access reference
        let input = b"UNB+UNOA:3+S+R+200101:0900+1'\
                      UNH+1+ORDERS:D:96A:UN+COMREF1'\
                      BGM+220+PO-001+9'\
                      UNT+3+1'\
                      UNZ+1+1'";
        let r = parse_and_validate(input).expect("UNH with common_access_ref must parse ok");
        assert_eq!(r.messages[0].common_access_ref.as_deref(), Some("COMREF1"));
    }

    #[test]
    fn common_access_ref_is_none_when_absent() {
        let r = parse_and_validate(VALID_INTERCHANGE).unwrap();
        assert!(r.messages[0].common_access_ref.is_none());
    }

    #[test]
    fn parse_unh_includes_message_ref() {
        let input = b"UNB+UNOA:3+S+R+200101:0900+1'\
                      UNH+REF42+ORDERS:D:96A:UN'\
                      BGM+220+PO-001+9'\
                      UNT+3+REF42'\
                      UNZ+1+1'";
        // Validate the high-level API which exercises parse_unh internally;
        // the message_ref field on MessageEnvelope should equal the UNH DE 0062 value.
        let r = parse_and_validate(input).expect("must parse ok");
        assert_eq!(r.messages[0].message_ref, "REF42");
        assert_eq!(r.messages[0].message_type, "ORDERS");
    }

    #[test]
    fn iter_messages_by_type_returns_matching_messages() {
        let input = b"UNB+UNOA:3+S+R+200101:0900+CTRL2'\
                      UNH+1+ORDERS:D:96A:UN'\
                      BGM+220+PO-001+9'\
                      UNT+3+1'\
                      UNH+2+INVOIC:D:96A:UN'\
                      BGM+380+INV-001+9'\
                      UNT+3+2'\
                      UNZ+2+CTRL2'";
        let r = parse_and_validate(input).expect("two-message interchange must parse ok");
        let orders: Vec<_> = r.iter_messages_by_type("ORDERS").collect();
        assert_eq!(orders.len(), 1);
        assert_eq!(orders[0].message_ref, "1");
        let invoices: Vec<_> = r.iter_messages_by_type("INVOIC").collect();
        assert_eq!(invoices.len(), 1);
        assert_eq!(invoices[0].message_ref, "2");
        let none: Vec<_> = r.iter_messages_by_type("DESADV").collect();
        assert!(none.is_empty());
    }

    #[test]
    fn display_interchange_envelope() {
        let r = parse_and_validate(VALID_INTERCHANGE).unwrap();
        let s = r.interchange.to_string();
        assert!(s.contains("SENDER"), "Display must include sender_id");
        assert!(s.contains("UNOA"), "Display must include syntax_identifier");
    }

    #[test]
    fn display_message_envelope() {
        let r = parse_and_validate(VALID_INTERCHANGE).unwrap();
        let s = r.messages[0].to_string();
        assert!(s.contains("ORDERS"), "Display must include message_type");
        assert!(s.contains("ref="), "Display must include message ref label");
    }

    #[test]
    fn display_validated_interchange() {
        let r = parse_and_validate(VALID_INTERCHANGE).unwrap();
        let s = r.to_string();
        assert!(
            s.contains("messages="),
            "Display must include message count label"
        );
    }

    // ── DE 0001 syntax identifier validation ─────────────────────────────────

    #[test]
    fn unrecognised_syntax_identifier_returns_err() {
        // DE 0001 is `UN` plus a two-character repertoire code; "XXXX" names none.
        let input = b"UNB+XXXX:3+S+R+200101:0900+1'\
                      UNH+1+ORDERS:D:96A:UN'\
                      BGM+220+PO-001+9'\
                      UNT+3+1'\
                      UNZ+1+1'";
        let result = parse_and_validate(input);
        assert!(
            matches!(result, Err(EdifactError::UnrecognisedSyntaxIdentifier(ref id)) if id == "XXXX"),
            "expected UnrecognisedSyntaxIdentifier(\"XXXX\"), got {result:?}"
        );
    }

    #[test]
    fn every_repertoire_the_crate_decodes_is_accepted_in_the_unb() {
        // `Charset` is the single source of which repertoires exist, so every
        // one it decodes must also be accepted in a `UNB`.
        for id in [
            "UNOA", "UNOB", "UNOC", "UNOD", "UNOE", "UNOF", "UNOG", "UNOH", "UNOI", "UNOJ", "UNOK",
            "UNOY",
        ] {
            let input = format!(
                "UNB+{id}:3+S+R+200101:0900+1'UNH+1+ORDERS:D:96A:UN'BGM+220+PO-001+9'UNT+3+1'UNZ+1+1'"
            );
            let result = parse_and_validate(input.as_bytes());
            assert!(
                result.is_ok(),
                "syntax id '{id}' should be accepted, got {result:?}"
            );
        }
    }

    #[test]
    fn a_defined_but_undecodable_repertoire_says_so() {
        // `UNOX` (an ISO 9735 repertoire) and `KECA` (a Korean identifier outside
        // ISO 9735) are known identifiers this crate cannot decode.  "Unsupported" and "unrecognised" are different claims:
        // one is about this crate, the other about the sender.
        for id in ["UNOX", "KECA"] {
            let input = format!(
                "UNB+{id}:3+S+R+200101:0900+1'UNH+1+ORDERS:D:96A:UN'BGM+220+PO-001+9'UNT+3+1'UNZ+1+1'"
            );
            let result = parse_and_validate(input.as_bytes());
            assert!(
                matches!(result, Err(EdifactError::UnsupportedCharset { ref syntax_identifier }) if syntax_identifier == id),
                "expected UnsupportedCharset for '{id}', got {result:?}"
            );
        }
    }

    #[test]
    fn an_interchange_with_no_content_is_rejected() {
        // ISO 9735-1 §7.1.  `UNZ+0` makes the control count agree, so this is
        // invisible to every other check.
        let result = parse_and_validate(b"UNB+UNOC:3+S+R+260101:0900+IC1'UNZ+0+IC1'");
        assert!(
            matches!(result, Err(EdifactError::EmptyInterchange { ref control_ref }) if control_ref == "IC1"),
            "expected EmptyInterchange, got {result:?}"
        );
    }

    #[test]
    fn a_message_with_no_body_is_rejected() {
        // ISO 9735-1 §7.3: a message "shall contain at least one additional
        // segment".  `UNT+2` is self-consistent, so only this rule sees it.
        let result = parse_and_validate(
            b"UNB+UNOC:3+S+R+260101:0900+IC1'UNH+1+ORDERS:D:96A:UN'UNT+2+1'UNZ+1+IC1'",
        );
        assert!(
            matches!(result, Err(EdifactError::EmptyMessage { ref message_ref, .. }) if message_ref == "1"),
            "expected EmptyMessage, got {result:?}"
        );
    }

    #[test]
    fn a_package_is_reported_as_a_package() {
        // Not as a stray segment: `UNO` is a legal member of an interchange
        // (ISO 9735-1 §7.10), and saying "invalid segment for message type ENVELOPE" sends
        // the reader hunting for a corruption that is not there.
        let result = parse_and_validate(
            b"UNB+UNOC:4+S+R+20260101:0900+IC1'UNO+PKG1+AAF:1+ZZZ+1024'UNZ+1+IC1'",
        );
        assert!(
            matches!(result, Err(EdifactError::PackageNotSupported { ref tag, .. }) if tag == "UNO"),
            "expected PackageNotSupported, got {result:?}"
        );
    }

    // ── UNB S005 password qualifier ───────────────────────────────────────────

    #[test]
    fn recipient_password_qualifier_extracted_when_present() {
        // S005: MYPASS:AA — comp[0]=password, comp[1]=qualifier (DE 0025)
        let input = b"UNB+UNOA:3+S+R+200101:0900+1+MYPASS:AA'\
                      UNH+1+ORDERS:D:96A:UN'\
                      BGM+220+PO-001+9'\
                      UNT+3+1'\
                      UNZ+1+1'";
        let r = parse_and_validate(input).expect("UNB with password+qualifier must parse ok");
        assert_eq!(
            r.interchange.header.recipient_password.as_deref(),
            Some("MYPASS")
        );
        assert_eq!(
            r.interchange.header.recipient_password_qualifier.as_deref(),
            Some("AA")
        );
    }

    #[test]
    fn recipient_password_qualifier_is_none_when_absent() {
        let r = parse_and_validate(VALID_INTERCHANGE).unwrap();
        assert!(r.interchange.header.recipient_password_qualifier.is_none());
    }

    // ── UNH S010 sequence-of-transfers ───────────────────────────────────────

    #[test]
    fn sequence_of_transfers_extracted_when_present() {
        // UNH element [2] = common access ref, element [3] = S010 (seq:position)
        let input = b"UNB+UNOA:3+S+R+200101:0900+1'\
                      UNH+1+ORDERS:D:96A:UN++2:C'\
                      BGM+220+PO-001+9'\
                      UNT+3+1'\
                      UNZ+1+1'";
        let r = parse_and_validate(input).expect("UNH with S010 must parse ok");
        assert_eq!(r.messages[0].sequence_of_transfers, Some(2));
        assert_eq!(r.messages[0].transfer_position.as_deref(), Some("C"));
    }

    #[test]
    fn sequence_of_transfers_none_when_absent() {
        let r = parse_and_validate(VALID_INTERCHANGE).unwrap();
        assert!(r.messages[0].sequence_of_transfers.is_none());
        assert!(r.messages[0].transfer_position.is_none());
    }

    // ── UNG S006/S007 application qualifiers ─────────────────────────────────

    #[test]
    fn ung_app_sender_and_recipient_qualifiers_extracted() {
        // UNG: group_id + S006(app_sender:qualifier) + S007(app_recip:qualifier) + ...
        let input = b"UNB+UNOA:3+S+R+200101:0900+1'\
                      UNG+ORDERS+APPSEND:ZZZ+APPRECV:14+200101:0900+1+UN+D:96A'\
                      UNH+1+ORDERS:D:96A:UN'\
                      BGM+220+PO-001+9'\
                      UNT+3+1'\
                      UNE+1+1'\
                      UNZ+1+1'";
        let r = parse_and_validate(input).expect("UNG with qualifiers must parse ok");
        let g = &r.functional_groups[0];
        assert_eq!(g.app_sender, "APPSEND");
        assert_eq!(g.app_sender_qualifier, "ZZZ");
        assert_eq!(g.app_recipient, "APPRECV");
        assert_eq!(g.app_recipient_qualifier, "14");
    }

    #[test]
    fn ung_qualifiers_empty_when_absent() {
        let input = b"UNB+UNOA:3+S+R+200101:0900+1'\
                      UNG+ORDERS+S+R+200101:0900+1+UN+D:96A'\
                      UNH+1+ORDERS:D:96A:UN'\
                      BGM+220+PO-001+9'\
                      UNT+3+1'\
                      UNE+1+1'\
                      UNZ+1+1'";
        let r = parse_and_validate(input).unwrap();
        let g = &r.functional_groups[0];
        assert_eq!(g.app_sender_qualifier, "");
        assert_eq!(g.app_recipient_qualifier, "");
    }

    // ── LenientResult methods ─────────────────────────────────────────────────

    #[test]
    fn lenient_result_is_valid_true_on_clean_interchange() {
        let owned = parse(VALID_INTERCHANGE);
        let r = validate_envelope_lenient(&owned);
        assert!(r.is_valid());
        assert!(r.errors.is_empty());
        assert!(r.interchange.is_some());
    }

    #[test]
    fn lenient_result_into_strict_ok_path() {
        let owned = parse(VALID_INTERCHANGE);
        let r = validate_envelope_lenient(&owned);
        let strict = r.into_strict();
        assert!(
            strict.is_ok(),
            "into_strict() should succeed for valid interchange"
        );
        assert_eq!(strict.unwrap().messages.len(), 1);
    }

    #[test]
    fn lenient_result_into_strict_err_path() {
        // Count mismatch → into_strict() returns Err with the error
        let input = b"UNB+UNOA:3+S+R+200101:0900+1'UNH+1+ORDERS:D:11A:UN:EAN010'BGM+220+PO-1+9'UNT+3+1'UNZ+2+1'";
        let owned = parse(input);
        let r = validate_envelope_lenient(&owned);
        assert!(!r.is_valid());
        let strict = r.into_strict();
        assert!(strict.is_err());
        let errors = strict.unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(matches!(
            &errors[0],
            EdifactError::MessageCountMismatch {
                expected: 2,
                actual: 1,
                ..
            }
        ));
    }

    // ── Direct parse_unh / parse_ung API ─────────────────────────────────────

    #[test]
    fn parse_unh_direct_extracts_all_s009_fields() {
        // parse_unh is called internally by extract_messages_flat; all S009 fields
        // it extracts surface in the resulting MessageEnvelope.
        let input = b"UNB+UNOA:3+S+R+200101:0900+1'\
                      UNH+REF99+ORDERS:D:96A:UN:EAN010'\
                      BGM+220+PO-001+9'\
                      UNT+3+REF99'\
                      UNZ+1+1'";
        let r = parse_and_validate(input).expect("must parse ok");
        let msg = &r.messages[0];
        assert_eq!(msg.message_ref, "REF99");
        assert_eq!(msg.message_type, "ORDERS");
        assert_eq!(msg.version, "D");
        assert_eq!(msg.release, "96A");
        assert_eq!(msg.controlling_agency, "UN");
        assert_eq!(msg.association_code, "EAN010");
    }

    #[test]
    fn parse_ung_direct_extracts_identifier_fields() {
        // parse_ung fields surface via the FunctionalGroupEnvelope returned by validation.
        let input = b"UNB+UNOA:3+S+R+200101:0900+1'\
                      UNG+ORDERS+APPSEND:ZZZ+APPRECV:14+200101:0900+GRP01+UN+D:96A'\
                      UNH+1+ORDERS:D:96A:UN'\
                      BGM+220+PO-001+9'\
                      UNT+3+1'\
                      UNE+1+GRP01'\
                      UNZ+1+1'";
        let r = parse_and_validate(input).expect("must parse ok");
        let g = &r.functional_groups[0];
        assert_eq!(g.group_id, "ORDERS");
        assert_eq!(g.app_sender, "APPSEND");
        assert_eq!(g.app_sender_qualifier, "ZZZ");
        assert_eq!(g.app_recipient, "APPRECV");
        assert_eq!(g.app_recipient_qualifier, "14");
        assert_eq!(g.group_ref, "GRP01");
        assert_eq!(g.controlling_agency, "UN");
        assert_eq!(g.version, "D");
        assert_eq!(g.release, "96A");
    }

    // ── find_message convenience method ──────────────────────────────────────

    #[test]
    fn find_message_returns_correct_message_by_ref() {
        let input = b"UNB+UNOA:3+S+R+200101:0900+CTRL2'\
                      UNH+REF-A+ORDERS:D:96A:UN'\
                      BGM+220+PO-001+9'\
                      UNT+3+REF-A'\
                      UNH+REF-B+INVOIC:D:96A:UN'\
                      BGM+380+INV-001+9'\
                      UNT+3+REF-B'\
                      UNZ+2+CTRL2'";
        let r = parse_and_validate(input).expect("two-message interchange must parse ok");
        let msg_a = r.find_message("REF-A");
        assert!(msg_a.is_some());
        assert_eq!(msg_a.unwrap().message_type, "ORDERS");
        let msg_b = r.find_message("REF-B");
        assert!(msg_b.is_some());
        assert_eq!(msg_b.unwrap().message_type, "INVOIC");
        assert!(r.find_message("MISSING").is_none());
    }

    // ── UNG missing mandatory group_ref ──────────────────────────────────────

    #[test]
    fn ung_missing_group_ref_returns_err() {
        // UNG with element [4] (group ref) empty — must error, not silently use "".
        // DE 0048 is a simple element, so its absence is the element's.
        let input = b"UNB+UNOA:3+S+R+200101:0900+1'\
                      UNG+ORDERS+S+R+200101:0900++UN+D:96A'\
                      UNH+1+ORDERS:D:96A:UN'\
                      BGM+220+PO-001+9'\
                      UNT+3+1'\
                      UNE+1+'\
                      UNZ+1+1'";
        let result = parse_and_validate(input);
        assert!(
            matches!(result,
                Err(EdifactError::MissingRequiredElement { ref tag, element_index: 4 })
                if tag == "UNG"
            ),
            "expected MissingRequiredElement for empty UNG group_ref, got {result:?}"
        );
    }

    // ── Edge case and ergonomics tests ────────────────────────────────────────

    #[test]
    fn empty_segment_list_returns_missing_unb() {
        // Contract: validate_envelope(&[]) must return MissingSegment{UNB},
        // not panic or return Ok.
        let result = validate_envelope(&[]);
        assert!(
            matches!(result, Err(EdifactError::MissingSegment { ref tag, .. }) if tag == "UNB"),
            "expected MissingSegment(UNB) for empty input, got {result:?}"
        );
    }

    #[test]
    fn single_segment_only_unb_returns_missing_unz() {
        // Only UNB, no UNZ — should fail with MissingSegment{UNZ}.
        let input = b"UNB+UNOA:3+S+R+200101:0900+1'";
        let owned = parse(input);
        let result = validate_envelope(&owned);
        assert!(
            matches!(result, Err(EdifactError::MissingSegment { ref tag, .. }) if tag == "UNZ"),
            "expected MissingSegment(UNZ) for UNB-only input, got {result:?}"
        );
    }

    #[test]
    fn display_validated_interchange_includes_count() {
        let r = parse_and_validate(VALID_INTERCHANGE).unwrap();
        let s = r.to_string();
        // Must include the numeric count, not just the label
        assert!(
            s.contains("messages=1"),
            "Display must include count '1': {s}"
        );
    }

    #[test]
    fn display_interchange_envelope_contains_arrow() {
        let r = parse_and_validate(VALID_INTERCHANGE).unwrap();
        let s = r.interchange.to_string();
        // Must use ASCII arrow, not Unicode →
        assert!(s.contains("->"), "Display must use ASCII '->' arrow: {s}");
        assert!(
            !s.contains('\u{2192}'),
            "Display must not use Unicode → arrow: {s}"
        );
    }

    #[test]
    fn display_functional_group_envelope() {
        let input = b"UNB+UNOA:3+S+R+200101:0900+1'\
                      UNG+ORDERS+APPSEND+APPRECV+200101:0900+GRP01+UN+D:96A'\
                      UNH+1+ORDERS:D:96A:UN'\
                      BGM+220+PO-001+9'\
                      UNT+3+1'\
                      UNE+1+GRP01'\
                      UNZ+1+1'";
        let r = parse_and_validate(input).expect("must parse ok");
        let g = &r.functional_groups[0];
        let s = g.to_string();
        assert!(s.contains("ORDERS"), "Display must include group_id: {s}");
        assert!(
            s.contains("APPSEND"),
            "Display must include app_sender: {s}"
        );
        assert!(
            s.contains("APPRECV"),
            "Display must include app_recipient: {s}"
        );
        assert!(s.contains("GRP01"), "Display must include group_ref: {s}");
        assert!(
            s.contains("msgs="),
            "Display must include message count label: {s}"
        );
        assert!(
            s.contains("1/1"),
            "Display must include actual/declared counts: {s}"
        );
    }

    #[test]
    fn lenient_has_errors_is_inverse_of_is_valid() {
        let owned = parse(VALID_INTERCHANGE);
        let valid = validate_envelope_lenient(&owned);
        assert!(valid.is_valid());
        assert!(!valid.has_errors());

        // Count mismatch: is_valid() == false, has_errors() == true
        let input = b"UNB+UNOA:3+S+R+200101:0900+1'UNH+1+ORDERS:D:11A:UN:EAN010'BGM+220+PO-1+9'UNT+3+1'UNZ+2+1'";
        let owned2 = parse(input);
        let invalid = validate_envelope_lenient(&owned2);
        assert!(!invalid.is_valid());
        assert!(invalid.has_errors());
    }

    // ── S004 calendar ─────────────────────────────────────────────────────────

    #[test]
    fn calendar_helpers_follow_the_standard() {
        // YYMMDD: the century is not transferred, so every YY % 4 == 0 is leap.
        assert!(is_valid_yymmdd("000229"));
        assert!(is_valid_yymmdd("960229"));
        assert!(!is_valid_yymmdd("010229"));
        assert!(!is_valid_yymmdd("261399"));
        assert!(!is_valid_yymmdd("260100"));
        assert!(!is_valid_yymmdd("260431"));
        // CCYYMMDD: Gregorian.
        assert!(is_valid_ccyymmdd("20000229"));
        assert!(!is_valid_ccyymmdd("19000229"));
        assert!(is_valid_ccyymmdd("20240229"));
        assert!(!is_valid_ccyymmdd("20261301"));
        // HHMM.
        assert!(is_valid_hhmm("0000"));
        assert!(is_valid_hhmm("2359"));
        assert!(!is_valid_hhmm("2400"));
        assert!(!is_valid_hhmm("1260"));
        // Malformed input is invalid, never a panic.
        for value in [
            "",
            "1",
            "2026010",
            "99999999999999999999",
            "ab0101",
            "ü1234",
        ] {
            assert!(!is_valid_yymmdd(value) && !is_valid_ccyymmdd(value) && !is_valid_hhmm(value));
        }
    }

    #[test]
    fn s004_is_checked_only_after_its_representation_passed() {
        let check = |input: &[u8]| {
            let segments = parse(input);
            let mut out = Vec::new();
            check_s004(
                &segments[0],
                syntax_version(&segments[0]).ok().flatten(),
                &mut out,
            );
            out.iter()
                .map(EdifactError::stable_code)
                .collect::<Vec<_>>()
        };
        assert_eq!(check(b"UNB+UNOC:3+S+R+261399:2599+1'"), ["E060", "E060"]);
        // Too long for n6 / n4: the representation check reports it, not this.
        assert!(check(b"UNB+UNOC:3+S+R+20261399:25999+1'").is_empty());
        // Unknown version: either length is a date.
        assert_eq!(check(b"UNB+UNOC:9+S+R+20261399:0900+1'"), ["E060"]);
    }

    #[test]
    fn the_syntax_version_is_one_to_four() {
        let version = |input: &[u8]| syntax_version(&parse(input)[0]);
        assert_eq!(version(b"UNB+UNOC:1+S+R+260101:0900+1'"), Ok(Some(1)));
        assert_eq!(version(b"UNB+UNOC:4+S+R+20260101:0900+1'"), Ok(Some(4)));
        assert_eq!(version(b"UNB+UNOC+S+R+260101:0900+1'"), Ok(None));
        assert_eq!(
            version(b"UNB+UNOC:X+S+R+260101:0900+1'"),
            Err(EdifactError::UnsupportedSyntaxVersion {
                value: "X".into(),
                span: Span::new(9, 10),
            })
        );
    }

    #[test]
    fn u_tags_are_classified() {
        let code = |tag: &str| classify_u_tag(tag, Span::new(0, 3)).map(|e| e.stable_code());
        assert_eq!(code("UIB"), Some("E055"));
        assert_eq!(code("USH"), Some("E056"));
        assert_eq!(code("UXX"), Some("E057"));
        for tag in ["UNA", "UNB", "UNS", "UGH", "UCI", "BGM", "XYZ"] {
            assert_eq!(code(tag), None, "{tag}");
        }
    }
}
