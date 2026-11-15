use crate::model::Span;
use std::fmt;
use thiserror::Error;

/// `text` for diagnostic output: every control character (Unicode `Cc`),
/// U+2028/U+2029 and every Unicode `Bidi_Control` character written as an
/// escape (`\n`, `\r`, `\t`, `\0`, otherwise `\u{1b}` style), everything
/// else unchanged.
///
/// One diagnostic stays one line, no input byte drives a terminal, and no
/// value reorders the text around it. The
/// escape is idempotent: its output contains no character it would escape.
pub(crate) fn escape_control(text: &str) -> EscapeControl<'_> {
    EscapeControl(text)
}

/// [`Display`](fmt::Display) of [`escape_control`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct EscapeControl<'a>(&'a str);

/// Whether [`escape_control`] escapes `c`.
pub(crate) fn needs_escape(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{2028}'
                | '\u{2029}'
                // Bidi_Control
                | '\u{061C}'
                | '\u{200E}'
                | '\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2066}'..='\u{2069}'
        )
}

impl fmt::Display for EscapeControl<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut rest = self.0;
        while let Some((at, c)) = rest.char_indices().find(|&(_, c)| needs_escape(c)) {
            f.write_str(&rest[..at])?;
            match c {
                '\0' => f.write_str("\\0")?,
                '\t' => f.write_str("\\t")?,
                '\n' => f.write_str("\\n")?,
                '\r' => f.write_str("\\r")?,
                _ => write!(f, "\\u{{{:x}}}", u32::from(c))?,
            }
            rest = &rest[at + c.len_utf8()..];
        }
        f.write_str(rest)
    }
}

/// Wrapper around [`std::io::Error`] that implements [`PartialEq`] by comparing [`std::io::ErrorKind`].
///
/// This allows `EdifactError` to derive `PartialEq` without requiring `std::io::Error: PartialEq`.
#[derive(Debug)]
pub struct IoError(pub(crate) std::io::Error);

impl IoError {
    /// Returns a reference to the underlying [`std::io::Error`].
    pub fn inner(&self) -> &std::io::Error {
        &self.0
    }
}

impl PartialEq for IoError {
    /// Equality is determined by [`std::io::ErrorKind`] only.
    ///
    /// Two `IoError` values with the same kind but different OS-level error codes
    /// (or different messages) will compare as equal.  This is a deliberate
    /// limitation: `std::io::Error` is not `PartialEq`, so kind-based comparison
    /// is the only practical option that lets `EdifactError` derive `PartialEq`.
    fn eq(&self, other: &Self) -> bool {
        self.0.kind() == other.0.kind()
    }
}

impl std::fmt::Display for IoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for IoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

impl From<std::io::Error> for IoError {
    fn from(e: std::io::Error) -> Self {
        Self(e)
    }
}

/// Which rule of ISO 9735-1 §9.1 a value violates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Insignificant {
    /// A variable-length numeric value carries leading zeroes.
    ///
    /// "Nevertheless, a single zero before a decimal mark is allowed", so `0.5`
    /// is correct and `00.5` is not.
    LeadingZeroes,
    /// A variable-length alphabetic or alphanumeric value carries trailing
    /// spaces.
    TrailingSpaces,
}

impl std::fmt::Display for Insignificant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::LeadingZeroes => "leading zeroes are not suppressed",
            Self::TrailingSpaces => "trailing spaces are not suppressed",
        })
    }
}

/// All errors produced by `edifact`.
///
/// # Positional data
///
/// Two kinds of position information appear in this enum, and the distinction is
/// deliberate:
///
/// * **`offset: usize`** — a single byte position in the input stream.  Used by
///   the lexical variants (`UnexpectedEof`, `InvalidDelimiter`, `InvalidText`,
///   `InvalidReleaseSequence`, `SegmentTooLong`, `UnexpectedDataToken`) and by
///   `LimitExceeded`, where the fault is a *point* in the byte stream and no
///   meaningful end position exists.
/// * **`span: Span`** — a half-open byte range.  Used by every variant produced
///   while validating an already-parsed [`Segment`][crate::Segment], where the
///   exact source range is known.  A [`ValidationIssue`][crate::ValidationIssue]
///   built from such an error carries the full range, so `miette` and LSP
///   tooling can underline the offending segment rather than place a
///   zero-width caret.
///
/// Use `span.start` when only the start position is needed.
///
/// # Text from the input
///
/// `Display` (and, with `diagnostics`, the `miette` help) writes control
/// characters, U+2028/U+2029 and bidirectional formatting characters
/// (`Bidi_Control`) from input values as escapes, so one error is one line,
/// no input byte reaches a terminal raw and no value reorders the line. The fields hold the raw
/// values.
///
/// ```
/// use edifact::{EdifactError, Span};
///
/// let err = EdifactError::DuplicateReference {
///     tag: "UNH".into(),
///     reference: "A\r\nFAKE".into(),
///     span: Span::new(5, 9),
/// };
/// assert_eq!(err.to_string(), r"duplicate UNH reference 'A\r\nFAKE' at bytes 5..9");
/// ```
#[derive(Debug, Error, PartialEq)]
#[non_exhaustive]
pub enum EdifactError {
    /// Unexpected end of input while parsing.
    ///
    /// Raised when a segment terminator or expected delimiter is not found
    /// before the end of the input stream.
    #[error("unexpected end of input at byte offset {offset}")]
    UnexpectedEof {
        /// Byte offset where the parser exhausted input.
        offset: usize,
    },

    /// Invalid byte encountered in a delimiter context.
    ///
    /// Delimiters must be precisely ASCII characters from the UNA service string advice.
    /// Any other byte is invalid in delimiter position.
    #[error("invalid delimiter byte 0x{byte:02X} at offset {offset}")]
    InvalidDelimiter {
        /// Unexpected delimiter byte.
        byte: u8,
        /// Byte offset where the delimiter was observed.
        offset: usize,
    },

    /// Invalid UTF-8 sequence in parsed text.
    ///
    /// While EDIFACT operates on bytes, segments and elements are expected to contain
    /// valid UTF-8 text. Non-UTF-8 sequences are rejected at parse time.
    #[error("invalid EDIFACT text at byte offset {offset}")]
    InvalidText {
        /// Byte offset where invalid UTF-8 text starts.
        offset: usize,
    },

    /// Invalid release-character escape sequence in parsed text.
    ///
    /// The release character (`?` by default) must be followed by one escaped byte.
    /// A trailing release character without a following byte is malformed.
    #[error("invalid release sequence at byte offset {offset}: dangling release character")]
    InvalidReleaseSequence {
        /// Byte offset of the dangling release character.
        offset: usize,
    },

    /// `UNZ` DE 0036 does not match the interchange's content.
    ///
    /// DE 0036 counts the messages and packages of the interchange or, when
    /// groups are used, its groups (ISO 9735-10:2002 §5.3.5, DE 0036). A group's
    /// own count is [`GroupCountMismatch`](Self::GroupCountMismatch).
    #[error("UNZ DE 0036 declares {expected} unit(s), the interchange holds {actual}")]
    MessageCountMismatch {
        /// Count declared in `UNZ` DE 0036.
        expected: u32,
        /// Messages (or groups, when the interchange uses them) found.
        actual: u32,
        /// Byte range of the `UNZ`.
        span: Span,
    },

    /// UNT segment count does not match the actual number of segments in the message.
    ///
    /// The `UNT` segment declares the number of segments in the message (including `UNH`/`UNT`),
    /// but the actual count differs.
    #[error(
        "segment count mismatch in message {message_ref}: UNT declared {expected}, found {actual}",
        message_ref = escape_control(.message_ref)
    )]
    SegmentCountMismatch {
        /// Segment count declared in the UNT segment (DE 0074, up to 10 digits).
        expected: u64,
        /// Actual number of segments observed.
        actual: u64,
        /// Message reference from the UNH segment.
        message_ref: String,
        /// Byte range of the offending `UNT`.
        ///
        /// This is what places the finding on the *message* rather than on the
        /// interchange — a `CONTRL` built from the report reports it on that
        /// message's `UCM`, and a rendered diagnostic underlines the trailer
        /// that got the count wrong.
        span: Span,
    },

    /// Invalid or malformed segment tag.
    ///
    /// Segment tags must be exactly 3 ASCII uppercase letters.
    #[error("invalid segment tag {0:?}")]
    InvalidSegmentTag(String),

    /// Invalid UNA service string advice.
    ///
    /// If present, the UNA segment must be exactly 9 bytes: `"UNA"` followed by
    /// 6 service characters.  The **active** ones — component separator, element
    /// separator, release character, segment terminator, and the repetition
    /// separator when it is not the "not used" space — must all be mutually
    /// distinct and printable, non-alphanumeric ASCII.
    ///
    /// The **decimal mark is not checked**: ISO 9735-1 Annex B says the character
    /// in that position "shall be ignored by the recipient", and it is the one
    /// position where the standard permits a space.
    #[error("invalid UNA service string advice")]
    InvalidUna,

    /// Missing required element in a segment.
    ///
    /// Certain segments require specific elements to be present. This error indicates
    /// a mandatory element was not found.
    #[error("missing required element {element_index} in segment {tag}", tag = escape_control(.tag))]
    MissingRequiredElement {
        /// Segment tag containing the missing element.
        tag: String,
        /// Zero-based required element index.
        element_index: usize,
    },

    /// Missing required component in a composite element.
    ///
    /// The element is present, but the required component at the given index is absent or empty.
    #[error(
        "missing required component {component_index} in element {element_index} of segment {tag}",
        tag = escape_control(.tag)
    )]
    MissingRequiredComponent {
        /// Segment tag containing the composite element.
        tag: String,
        /// Zero-based element index of the composite.
        element_index: usize,
        /// Zero-based component index that was absent.
        component_index: usize,
    },

    /// Output serialization produced invalid UTF-8.
    ///
    /// This is an internal consistency error; the writer should never produce non-UTF-8 output.
    /// If this occurs, it indicates a bug in the serialization logic.
    #[error("serialized output contains invalid UTF-8")]
    InvalidUtf8,

    /// I/O error from reading or writing.
    #[error(transparent)]
    Io(#[from] IoError),

    // ── validation variants (E010–E020) ────────────────────────────────────
    /// Segment is not valid for the current message type.
    ///
    /// Structural validation found a segment that should not appear in this message.
    #[error("segment {tag} is not valid for message type {message_type}", tag = escape_control(.tag), message_type = escape_control(.message_type))]
    InvalidSegmentForMessage {
        /// Segment tag that is not allowed for the message type.
        tag: String,
        /// Message type used for structural validation.
        message_type: String,
        /// Byte range of the offending segment tag.
        span: Span,
    },

    /// Element count in segment exceeds or falls short of directory definition.
    ///
    /// Validation against directory metadata found an element count mismatch.
    #[error("segment {tag} has {actual} elements, expected between {min} and {max}", tag = escape_control(.tag))]
    InvalidElementCount {
        /// Segment tag with wrong arity.
        tag: String,
        /// Minimum allowed element count.
        min: usize,
        /// Maximum allowed element count.
        max: usize,
        /// Actual element count found.
        actual: usize,
        /// Byte range of the offending segment.
        span: Span,
    },

    /// Component count in a composite element is invalid.
    ///
    /// A composite data element does not have the expected number of components.
    #[error("segment {tag} element {element_index} has {actual} components, expected {expected}", tag = escape_control(.tag))]
    InvalidComponentCount {
        /// Segment tag containing the composite.
        tag: String,
        /// Zero-based element index of the composite.
        element_index: usize,
        /// Expected component count.
        expected: u8,
        /// Actual component count found.
        actual: u8,
        /// Byte range of the offending composite element.
        span: Span,
    },

    /// Code-list value is not valid.
    ///
    /// The value appears in a field that should contain a code from a specific code list,
    /// but the value is not in that code list.
    #[error(
        "segment {tag} element {element_index}: '{value}' is not a valid code (code list {code_list})",
        tag = escape_control(.tag), value = escape_control(.value), code_list = escape_control(.code_list)
    )]
    InvalidCodeValue {
        /// Segment tag containing the invalid value.
        tag: String,
        /// Zero-based element index containing the invalid code.
        element_index: usize,
        /// Invalid code value observed.
        value: String,
        /// Data element code list identifier.
        code_list: String,
        /// Byte range of the offending value.
        span: Span,
        /// Optional remediation suggestion from the code-list lookup function.
        suggestion: Option<&'static str>,
    },

    /// A required segment is missing from the message.
    ///
    /// Structural validation found that a mandatory segment is absent.
    #[error("required segment {tag} is missing from message (position {expected_position})", tag = escape_control(.tag), expected_position = escape_control(.expected_position))]
    MissingSegment {
        /// Missing segment tag.
        tag: String,
        /// Human-readable position hint.
        expected_position: String,
    },

    /// Qualifier does not match expected value for segment.
    ///
    /// A qualified segment (e.g., NAD+MS) has a qualifier that does not match expected.
    #[error("segment {tag} has qualifier '{actual}', expected '{expected}'", tag = escape_control(.tag), actual = escape_control(.actual), expected = escape_control(.expected))]
    QualifierMismatch {
        /// Segment tag whose qualifier mismatched.
        tag: String,
        /// Actual qualifier found.
        actual: String,
        /// Expected qualifier value.
        expected: String,
        /// Byte range of the offending segment.
        span: Span,
    },

    /// Conditional requirement not met.
    ///
    /// A segment or element is conditionally required based on another element's value,
    /// but the condition was not satisfied.
    #[error("segment {tag} element {element_index}: conditional requirement not met ({condition})", tag = escape_control(.tag), condition = escape_control(.condition))]
    ConditionalRequirementNotMet {
        /// Segment tag that violated a conditional rule.
        tag: String,
        /// Zero-based element index governed by the condition.
        element_index: usize,
        /// Condition text describing the rule.
        condition: String,
        /// Byte range of the offending segment.
        span: Span,
    },

    /// Validation failed and the full [`ValidationReport`] is preserved.
    ///
    /// Returned by validation helpers when errors are found.  Provides programmatic
    /// access to all issues, warnings, and infos.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// match my_fn() {
    ///     Err(EdifactError::ValidationErrors { report, .. }) => {
    ///         for issue in report.errors() {
    ///             eprintln!("{}", issue);
    ///         }
    ///     }
    ///     other => { /* ... */ }
    /// }
    /// ```
    #[error("validation failed with {error_count} error(s)")]
    ValidationErrors {
        /// Number of error-severity issues in the report.
        error_count: usize,
        /// Full report with all errors, warnings, and infos.
        report: Box<ValidationReport>,
    },

    /// Segment exceeded the configured maximum byte length.
    ///
    /// Returned by reader-based parsers when an unterminated segment accumulates more
    /// bytes than the configured `max_segment_bytes` limit in [`ReaderConfig`].  This
    /// prevents resource exhaustion on adversarially crafted or truncated input that
    /// never emits a segment terminator.
    ///
    /// [`ReaderConfig`]: crate::ReaderConfig
    #[error("segment starting at byte offset {offset} exceeded maximum length of {limit} bytes")]
    SegmentTooLong {
        /// Byte offset where the overlong segment started.
        offset: usize,
        /// Configured maximum segment byte length.
        limit: usize,
    },

    /// An interchange or message contains more segments or messages than can be
    /// represented in a `u32` counter (> 4 294 967 295).
    ///
    /// Returned rather than silently saturating or wrapping the counter.
    #[error("interchange too large: count {count} exceeds u32::MAX")]
    InterchangeTooLarge {
        /// The count that could not be represented as `u32`.
        count: u64,
    },

    /// Events or writer calls in an invalid sequence.
    ///
    /// This indicates a programming error in the caller's code: for example,
    /// emitting an [`crate::EdifactEvent::Element`] without a prior
    /// [`crate::EdifactEvent::StartSegment`], emitting
    /// [`crate::EdifactEvent::ComponentElement`] without a preceding
    /// [`crate::EdifactEvent::Element`], or a raw `UNB`/`UNZ` inside an
    /// interchange opened by [`crate::Writer::begin_interchange`].
    #[error("invalid event sequence: {message}")]
    InvalidEventSequence {
        /// Description of the protocol violation.
        message: &'static str,
    },

    /// An [`crate::OwnedElementRef`] has `position = 0`, which is never valid.
    ///
    /// Element positions are one-based: position 1 refers to the first element
    /// slot.  Position 0 is reserved and invalid.  Use [`crate::OwnedElementRef::try_new`]
    /// to get a `Result` instead of a panic.
    #[error("element definition contains invalid position 0; positions must be >= 1 (one-based)")]
    InvalidElementPosition,

    /// Two [`crate::ProfileRulePack`] values with incompatible release scopes were composed.
    ///
    /// When composing packs via [`crate::ProfileRulePack::extend_from`] or
    /// [`crate::ProfileRulePack::merge_with_override`], both packs must either
    /// share the same release scope or at most one may carry a scope.
    #[error("incompatible release scopes: cannot compose {current:?} with {incoming:?}")]
    IncompatibleReleaseScopes {
        /// Release scope of the pack being composed into.
        current: String,
        /// Release scope of the pack being composed in.
        incoming: String,
    },

    /// A field value failed semantic validation (e.g. wrong format, out-of-range).
    ///
    /// Distinct from [`InvalidCodeValue`][Self::InvalidCodeValue] which is for
    /// code-list membership checks.  Use this variant when a free-text or numeric
    /// field contains a value that is structurally invalid for its purpose.
    #[error("segment {tag} element {element_index}: invalid field value {value:?}", tag = escape_control(.tag))]
    InvalidFieldValue {
        /// Segment tag that contains the invalid field.
        tag: String,
        /// Zero-based element index of the invalid field.
        element_index: usize,
        /// The invalid value that was observed.
        value: String,
    },

    /// A data or component element token appeared before the first segment tag.
    ///
    /// EDIFACT syntax requires that every data element follows a segment tag.
    /// A data element token encountered before any tag (e.g. after a stray
    /// separator at the start of the stream) is a protocol violation.
    ///
    /// Like a segment terminator where a tag belongs (`E002`), stray data
    /// tokens indicate encoding corruption or a partial write.
    #[error("unexpected data token at byte offset {offset}: data element before segment tag")]
    UnexpectedDataToken {
        /// Byte offset of the stray token.
        offset: usize,
    },

    /// The interchange syntax identifier (`UNB` S001 DE 0001) names no
    /// repertoire this crate recognises.
    ///
    /// DE 0001 is `UN` plus a repertoire code from the ISO 9735-10 code list.
    /// This crate recognises `UNOA`–`UNOK` and `UNOY`; `UNOX` and `KECA` (a
    /// Korean identifier outside ISO 9735) are refused as
    /// [`UnsupportedCharset`][Self::UnsupportedCharset]; anything else is
    /// reported here.
    #[error("unrecognised syntax identifier '{}': expected UNOA-UNOK or UNOY", escape_control(.0))]
    UnrecognisedSyntaxIdentifier(String),

    /// A control reference was reused within the scope that requires it to be unique.
    ///
    /// ISO 9735-1 requires the message reference number (`UNH` DE 0062) to be
    /// unique within an interchange, and the group reference number (`UNG`
    /// DE 0048) to be unique within an interchange.  Duplicates make a message
    /// unaddressable: a receiver keying on the reference silently processes one
    /// occurrence and drops the rest.
    #[error("duplicate {tag} reference '{reference}' at bytes {span}", tag = escape_control(.tag), reference = escape_control(.reference))]
    DuplicateReference {
        /// Segment tag that carries the duplicated reference (`UNH` or `UNG`).
        tag: String,
        /// The reference value that appeared more than once.
        reference: String,
        /// Byte range of the duplicate occurrence.
        span: Span,
    },

    /// A UN/EDIFACT data element code was not found in the segment definition.
    ///
    /// Produced by the code-addressed accessors
    /// ([`Segment::value_by_code`][crate::Segment::value_by_code] and friends)
    /// when the requested data element identifier does not appear anywhere in
    /// the supplied [`SegmentLayout`][crate::SegmentLayout].  This is the error
    /// that turns a mistyped or stale DE reference into a loud failure instead
    /// of a silent off-by-one read of the wrong element.
    #[error("segment {tag} has no data element {data_element} in its definition", tag = escape_control(.tag), data_element = escape_control(.data_element))]
    UnknownDataElement {
        /// Segment tag whose definition was searched.
        tag: String,
        /// The data element identifier that was not found.
        data_element: String,
    },

    /// A UN/EDIFACT data element code appears more than once in a segment definition.
    ///
    /// Code-addressed access requires an unambiguous target.  When a directory
    /// genuinely repeats a code (e.g. the same DE used at two positions), address
    /// it positionally with [`Segment::element_str`][crate::Segment::element_str]
    /// or split the definition.
    #[error("segment {tag} defines data element {data_element} at more than one position", tag = escape_control(.tag), data_element = escape_control(.data_element))]
    AmbiguousDataElement {
        /// Segment tag whose definition was searched.
        tag: String,
        /// The data element identifier that resolved to multiple positions.
        data_element: String,
    },

    /// A [`ReaderConfig`][crate::ReaderConfig] resource limit or the repertoire
    /// sniff window was exceeded.
    ///
    /// Raised by the parsing iterators when the input carries more segments,
    /// messages, or bytes than the caller allowed, and by the repertoire sniff
    /// when the `UNB` syntax identifier lies outside its fixed window.  The limit is reported rather
    /// than silently applied: a budget that ends the iterator without an error is
    /// indistinguishable from a clean end of input, so the caller would accept a
    /// **truncated** interchange as complete.
    ///
    /// [`SegmentTooLong`][Self::SegmentTooLong] covers the per-segment size
    /// guard; this variant covers the count and byte budgets.
    ///
    /// The variant is `#[non_exhaustive]`: match it as
    /// `LimitExceeded { limit, max, .. }`.
    #[error("input exceeded the {limit} limit of {max} at byte offset {offset}")]
    #[non_exhaustive]
    LimitExceeded {
        /// Name of the limit that tripped: `"max_segments"`, `"max_messages"`,
        /// `"max_message_bytes"`, `"max_input_bytes"`, or — for the
        /// recovering scan's fault budget,
        /// [`RecoveryConfig::max_faults`][crate::RecoveryConfig::max_faults] —
        /// `"max_faults"`, or — for the repertoire sniff, see
        /// [`sniff_charset`][crate::sniff_charset] — `"sniff_window"`, or — for
        /// [`InterchangeHeader::read_from`][crate::InterchangeHeader::read_from]
        /// — `"header_read"`.
        limit: &'static str,
        /// The configured ceiling.
        max: u64,
        /// Where the budget ran out.
        ///
        /// For a count or per-message budget (`max_segments`, `max_messages`,
        /// `max_message_bytes`, `max_faults`): the start of the segment
        /// that would have exceeded it. For `max_input_bytes`: the budget
        /// itself — the first byte outside it — saturated to `usize::MAX`. For
        /// `sniff_window`: the first byte of the header the window cannot hold.
        offset: usize,
    },

    /// A repeating data element was written under a service string advice that
    /// declares no repetition separator.
    ///
    /// ISO 9735-1 §8.7 repetitions can only be expressed when `UNA` position 7
    /// carries a real separator.  With the space "not used" sentinel there is no
    /// byte to write between occurrences, and joining them anyway would emit
    /// output that reads back as a single occurrence — silent data corruption.
    /// Construct the writer with [`Writer::with_una`][crate::Writer::with_una]
    /// and a service string advice whose `repetition_sep` is set.
    #[error(
        "cannot write a repeating data element: the active service string advice declares no repetition separator"
    )]
    RepetitionSeparatorNotDeclared,

    /// A non-finite float was handed to a numeric serializer.
    ///
    /// ISO 9735-1 §10 admits the ISO 6093 numeric representations — digits, an
    /// optional minus sign, a decimal mark, an exponent — and nothing else.
    /// There is no representation for `NaN` or infinity, and `Display` would
    /// emit `NaN` / `inf`, text no receiver can parse and that the crate's own
    /// reader would hand back as a string.
    #[error("non-finite number {value} has no EDIFACT representation", value = escape_control(.value))]
    NonFiniteNumber {
        /// The offending value, formatted for the message.
        value: String,
    },

    /// A character cannot be represented in the interchange's declared repertoire.
    ///
    /// The `UNB` S001 DE 0001 syntax identifier names the character repertoire the
    /// payload is written in (`UNOA`, `UNOC`, …).  Writing a character outside it
    /// produces bytes the receiver decodes as something else — or as nothing at
    /// all — so the writer refuses instead.
    ///
    /// Also raised by [`Charset::encode`][crate::Charset::encode].
    #[error(
        "character {character:?} at offset {offset} is not in the {charset} character repertoire"
    )]
    CharacterNotInRepertoire {
        /// The syntax identifier of the repertoire that rejected the character.
        charset: &'static str,
        /// The offending character.
        character: char,
        /// Byte offset of the character within the value it appeared in.
        offset: usize,
    },

    /// An interchange header declares a repertoire other than the writer's own.
    ///
    /// A header that names `UNOA` while the body goes out as ISO 8859-1 is
    /// unreadable at the far end in exactly the way that is hardest to diagnose,
    /// so [`Writer::begin_interchange`][crate::Writer::begin_interchange] refuses
    /// the combination rather than emitting it.
    #[error("UNB declares repertoire {declared}, but the writer encodes {writer}", declared = escape_control(.declared))]
    CharacterRepertoireMismatch {
        /// Syntax identifier of the header passed to `begin_interchange`.
        declared: String,
        /// Repertoire the writer was bound to with `Writer::with_charset`.
        writer: &'static str,
    },

    /// The interchange declares a character repertoire this crate cannot decode.
    ///
    /// `UNOX` (ISO/IEC 2022 code extension) and `KECA` (a Korean identifier
    /// outside ISO 9735) are stateful or multi-byte in ways that break the byte-level delimiter scanning every
    /// other repertoire allows.  They are reported rather than silently
    /// mis-decoded.
    #[error("character repertoire '{syntax_identifier}' is not supported", syntax_identifier = escape_control(.syntax_identifier))]
    UnsupportedCharset {
        /// The `UNB` S001 DE 0001 value that could not be handled.
        syntax_identifier: String,
    },

    /// An interchange carries neither a message nor a group.
    ///
    /// ISO 9735-1 §7.1: an interchange "shall contain at least one group, or one
    /// message or one package".  A bare `UNB`/`UNZ` pair is a delivery that says
    /// nothing, and because `UNZ+0` makes the control count agree with the
    /// (absent) content, no count check can catch it.
    #[error("interchange {control_ref} contains no message or group", control_ref = escape_control(.control_ref))]
    EmptyInterchange {
        /// Interchange control reference (`UNB` DE 0020) of the empty interchange.
        control_ref: String,
    },

    /// A message carries no segment between its header and trailer.
    ///
    /// ISO 9735-1 §7.3: a message "shall be started and identified by a message
    /// header, shall be terminated by a message trailer, and shall contain at
    /// least one additional segment".
    #[error("message {message_ref} has no segments between UNH and UNT", message_ref = escape_control(.message_ref))]
    EmptyMessage {
        /// Message reference number (`UNH` DE 0062) of the empty message.
        message_ref: String,
        /// Byte range of the offending `UNH`.
        span: Span,
    },

    /// The interchange carries a package (`UNO`…`UNP`), which this crate does not
    /// tokenize.
    ///
    /// A package wraps an arbitrary object — ISO 9735-1 §7.9, elaborated by
    /// ISO 9735-8 — whose bytes are not EDIFACT-encoded and whose length is
    /// declared in `UNO` S022 DE 0810.  Feeding those bytes to a tokenizer that
    /// scans for delimiters produces nonsense, so the package is reported instead.
    /// Split the object out of the byte stream using the declared length before
    /// parsing the remainder.
    #[error("segment {tag} opens or closes a package, which this crate does not parse", tag = escape_control(.tag))]
    PackageNotSupported {
        /// The package segment encountered: `UNO` or `UNP`.
        tag: String,
        /// Byte range of the offending segment.
        span: Span,
    },

    /// A data element value consists only of spaces.
    ///
    /// ISO 9735-1 §9.3: "A data element value containing only space(s) shall not
    /// be allowed."  Trailing spaces are insignificant and must be suppressed
    /// (§9.1), so a value that is nothing but spaces is an element that should
    /// have been omitted — and a receiver comparing it against a code list, a
    /// reference, or a previous message will not treat it as absent.
    #[error(
        "segment {tag} element {element_index} component {component_index}: value is only spaces",
        tag = escape_control(.tag)
    )]
    BlankDataElementValue {
        /// Segment tag containing the blank value.
        tag: String,
        /// Zero-based element index.
        element_index: usize,
        /// Zero-based component index.
        component_index: usize,
        /// Byte range of the offending value.
        span: Span,
    },

    /// A segment carries nothing but its tag.
    ///
    /// ISO 9735-1 §7.5: "A segment shall contain at least one data element in
    /// addition to the segment tag."  §8.5 adds that a conditional segment whose
    /// only content is the tag "shall be omitted in its entirety" — so `ABC'` is
    /// either a mandatory segment that lost its data or a conditional one that
    /// should not have been sent.
    #[error("segment {tag} contains no data element", tag = escape_control(.tag))]
    SegmentWithoutDataElements {
        /// The offending segment tag.
        tag: String,
        /// Byte range of the offending segment.
        span: Span,
    },

    /// A data element occurred more times than its definition allows.
    ///
    /// ISO 9735-1 §7.5: "Each stand-alone or composite data element's position,
    /// status and maximum number of occurrences within the segment structure
    /// shall be stated in the segment specification." Exceeding the stated
    /// maximum is a structural violation, reported by `CONTRL` as code 35.
    #[error("segment {tag} element {element_index} occurs {actual} times, at most {max} allowed", tag = escape_control(.tag))]
    TooManyRepetitions {
        /// Segment tag carrying the over-repeated element.
        tag: String,
        /// Zero-based element index.
        element_index: usize,
        /// Maximum occurrences the definition allows.
        max: u8,
        /// Occurrences actually present.
        actual: usize,
        /// Byte range of the offending element.
        span: Span,
    },

    /// A value's characters do not match its declared representation class.
    ///
    /// A directory types every data element `a` (alphabetic), `n` (numeric), or
    /// `an` (alphanumeric). ISO 9735-1 §10 fixes what "numeric" admits: digits,
    /// an optional minus, a decimal mark, and an exponent — and explicitly not
    /// the space character or a plus sign. Reported by `CONTRL` as code 37.
    #[error(
        "segment {tag} element {element_index} component {component_index}: {value:?} is not {repr}",
        tag = escape_control(.tag), repr = escape_control(.repr)
    )]
    InvalidCharacterType {
        /// Segment tag containing the value.
        tag: String,
        /// Zero-based element index.
        element_index: usize,
        /// Zero-based component index.
        component_index: usize,
        /// The declared representation, e.g. `n..6`.
        repr: String,
        /// The offending value.
        value: String,
        /// Byte range of the offending value.
        span: Span,
    },

    /// A value is longer than its declared representation allows.
    ///
    /// Length is counted in **characters**, not bytes (ISO 9735-1 §6), and for a
    /// numeric value excludes the sign, the decimal mark, and the exponent
    /// (§10). Reported by `CONTRL` as code 39.
    #[error(
        "segment {tag} element {element_index} component {component_index}: {actual} characters exceeds {repr}",
        tag = escape_control(.tag), repr = escape_control(.repr)
    )]
    DataElementTooLong {
        /// Segment tag containing the value.
        tag: String,
        /// Zero-based element index.
        element_index: usize,
        /// Zero-based component index.
        component_index: usize,
        /// The declared representation, e.g. `an..35`.
        repr: String,
        /// The measured character count.
        actual: usize,
        /// Byte range of the offending value.
        span: Span,
    },

    /// A value is shorter than its declared fixed-length representation.
    ///
    /// Only a fixed-length representation (`n8`, `a1`) has a minimum above one;
    /// a variable one (`an..35`) is satisfied by any non-empty value. Reported
    /// by `CONTRL` as code 40.
    #[error(
        "segment {tag} element {element_index} component {component_index}: {actual} characters is short of {repr}",
        tag = escape_control(.tag), repr = escape_control(.repr)
    )]
    DataElementTooShort {
        /// Segment tag containing the value.
        tag: String,
        /// Zero-based element index.
        element_index: usize,
        /// Zero-based component index.
        component_index: usize,
        /// The declared representation, e.g. `n8`.
        repr: String,
        /// The measured character count.
        actual: usize,
        /// Byte range of the offending value.
        span: Span,
    },

    /// A segment or composite ends in separators that carry no value.
    ///
    /// ISO 9735-1 §8.7.1: "If one or more non-repeating composite data elements
    /// or stand-alone data elements at the end of a segment are omitted, the
    /// data element separators which would normally follow them shall also be
    /// omitted." §8.7.2 says the same for components at the end of a composite.
    ///
    /// `BGM+220+'` and `DTM+137:20260101:'` are therefore both malformed, and a
    /// receiver that trims them is being generous rather than correct. Reported
    /// by `CONTRL` as code 45.
    #[error("segment {tag} ends in a separator that carries no value", tag = escape_control(.tag))]
    TrailingSeparator {
        /// Segment tag carrying the trailing separator.
        tag: String,
        /// `Some(index)` when the trailing separators close that element's
        /// composite; `None` when they close the segment itself.
        element_index: Option<usize>,
        /// Byte range of the offending segment or element.
        span: Span,
    },

    /// An interchange mixes groups with ungrouped messages.
    ///
    /// ISO 9735-1 §7.1 lists what an interchange may contain, and every entry is
    /// exclusive: messages, or packages, or groups containing them — never a
    /// group alongside a bare message. A message outside every group has no
    /// group to be counted in, so `UNZ` DE 0036 cannot describe the interchange
    /// at all. Reported by `CONTRL` as code 30.
    #[error("interchange mixes groups with ungrouped messages")]
    GroupsAndMessagesMixed {
        /// Byte range of the segment that revealed the mix.
        span: Span,
    },

    /// A value carries characters ISO 9735-1 §9.1 requires to be suppressed.
    ///
    /// §9.1: "In variable length numeric data elements, leading zeroes shall be
    /// suppressed... In variable length alphabetic and alphanumeric data
    /// elements, trailing spaces shall be suppressed."
    ///
    /// Both are artefacts of a fixed-width source record copied into a
    /// variable-length field. The value is still readable, so this is a warning
    /// — but a receiver comparing `007` against the code `7`, or `"ACME "`
    /// against `"ACME"`, will not match them.
    #[error("segment {tag} element {element_index} component {component_index}: {kind}", tag = escape_control(.tag))]
    InsignificantCharacters {
        /// Segment tag containing the value.
        tag: String,
        /// Zero-based element index.
        element_index: usize,
        /// Zero-based component index.
        component_index: usize,
        /// Which rule of §9.1 was violated.
        kind: Insignificant,
        /// Byte range of the offending value.
        span: Span,
    },

    /// `UNE` DE 0060 does not match the number of messages in its group.
    ///
    /// ISO 9735-10:2002 §5.3.5, DE 0060: "the number of messages and packages
    /// in the group". Reported by `CONTRL` as code 29 on the group (`UCF`).
    #[error(
        "UNE DE 0060 of group {group_ref} declares {expected} message(s), the group holds {actual}",
        group_ref = escape_control(.group_ref)
    )]
    GroupCountMismatch {
        /// Group reference number (`UNG`/`UNE` DE 0048).
        group_ref: String,
        /// Count declared in `UNE` DE 0060.
        expected: u32,
        /// Messages found in the group.
        actual: u32,
        /// Byte range of the `UNE`.
        span: Span,
    },

    /// An interactive-EDI segment (`UIB`, `UIH`, `UIR`, `UIT`, `UIZ`;
    /// ISO 9735-3), which this batch-EDI crate does not support.
    ///
    /// Reported by `CONTRL` as code 47 ("envelope functionality not supported").
    #[error("segment {tag} belongs to interactive EDI (ISO 9735-3), which is not supported", tag = escape_control(.tag))]
    InteractiveSegmentNotSupported {
        /// The interactive segment tag.
        tag: String,
        /// Byte range of the segment.
        span: Span,
    },

    /// A security segment (`USA`…`USY`; ISO 9735-5, -7, -9), which this crate
    /// does not support.
    ///
    /// The crate neither verifies nor strips security headers, so a secured
    /// message is refused rather than passed as if it were plain. Reported by
    /// `CONTRL` as code 47.
    #[error("segment {tag} is a security segment (ISO 9735-5/-7/-9), which is not supported", tag = escape_control(.tag))]
    SecuritySegmentNotSupported {
        /// The security segment tag.
        tag: String,
        /// Byte range of the segment.
        span: Span,
    },

    /// A segment tag starting with `U` that is no service segment.
    ///
    /// ISO 9735-1:2002 §7.7 reserves tags starting with `U` for service
    /// segments. Reported by `CONTRL` as code 15.
    #[error(
        "segment tag {tag} is reserved for service segments (ISO 9735-1 §7.7) and defines none",
        tag = escape_control(.tag)
    )]
    ReservedServiceSegmentTag {
        /// The reserved tag.
        tag: String,
        /// Byte range of the segment.
        span: Span,
    },

    /// Data follows the interchange trailer.
    ///
    /// One input holds one interchange: from `UNB` (or its `UNA`) to the first
    /// `UNZ` outside a message. A second interchange, a `UNA` after the first
    /// segment, or any stray segment after `UNZ` is reported here, once, at its
    /// first segment. Reported by `CONTRL` as code 33.
    #[error("segment {tag} follows the interchange trailer UNZ; one input holds one interchange", tag = escape_control(.tag))]
    DataAfterInterchangeTrailer {
        /// Tag of the first segment after `UNZ` (`UNA` from the tokenizer).
        tag: String,
        /// Byte range of that segment (of the three bytes `UNA` from the
        /// tokenizer).
        span: Span,
    },

    /// `UNB` S001 DE 0002 names a syntax version this crate does not support.
    ///
    /// Versions `1` to `4` are supported (ISO 9735-1:2002 §2); `X`
    /// (ISO 9735-11) and any other value are refused. Reported by `CONTRL`
    /// as code 2 on the interchange.
    #[error("UNB S001 DE 0002 syntax version {value:?} is not supported; expected 1, 2, 3 or 4")]
    UnsupportedSyntaxVersion {
        /// The declared value.
        value: String,
        /// Byte range of DE 0002.
        span: Span,
    },

    /// `UNB` or `UNG` S004 is not a calendar date or a 24-hour time.
    ///
    /// DE 0017 is `YYMMDD` (syntax versions 1–3) or `CCYYMMDD` (version 4);
    /// DE 0019 is `HHMM` (ISO 9735-10:2002 §5.3.5). A `YYMMDD` year divisible
    /// by 4 admits 29 February, because the century is not transferred.
    /// Reported by `CONTRL` as code 12.
    #[error(
        "segment {tag} element {element_index} component {component_index}: {value:?} is not a valid {}",
        if *component_index == 0 { "date" } else { "time (HHMM)" },
        tag = escape_control(.tag)
    )]
    InvalidDateTime {
        /// `UNB` or `UNG`.
        tag: String,
        /// Zero-based element index of S004.
        element_index: usize,
        /// `0` for DE 0017 (date), `1` for DE 0019 (time).
        component_index: usize,
        /// The offending value.
        value: String,
        /// Byte range of the value.
        span: Span,
    },

    /// A group (`UNG`…`UNE`) holds no message.
    ///
    /// ISO 9735-1:2002 §7.3: a group "shall contain at least one message or
    /// package". Reported by `CONTRL` as code 32 on the group.
    #[error("group {group_ref} contains no message", group_ref = escape_control(.group_ref))]
    EmptyGroup {
        /// Group reference number (`UNG` DE 0048).
        group_ref: String,
        /// Byte range of the `UNG`.
        span: Span,
    },

    /// A [`SegmentLayout`][crate::SegmentLayout] was applied to a segment with a different tag.
    ///
    /// Passing the `NAD` definition to a `DTM` segment would resolve codes
    /// against the wrong table, which is exactly the class of mistake that
    /// code-addressed access exists to prevent — so it is rejected up front.
    #[error("segment layout is for {expected}, but the segment is {actual}", expected = escape_control(.expected), actual = escape_control(.actual))]
    SegmentLayoutMismatch {
        /// Tag the layout describes.
        expected: String,
        /// Tag of the segment the layout was applied to.
        actual: String,
    },

    /// A `CONTRL` would carry an action code its configured policy forbids.
    ///
    /// Raised when a `CONTRL` is constructed with
    /// [`ContrlOptions`][crate::contrl::ContrlOptions] whose
    /// [`ActionPolicy`][crate::contrl::ActionPolicy] does not admit the DE 0083
    /// value a reporting level would need — for example a receipt (`8`) under a
    /// policy that admits only `7` and `4` on `UCI`. The code is neither emitted
    /// nor rewritten: a `CONTRL` the partner's guide does not admit is one it
    /// rejects, and a silently changed verdict says something that is not true.
    #[error("CONTRL {segment_tag} action code {action} is not permitted by the action policy")]
    ActionNotPermitted {
        /// The reporting segment that would carry the code: `UCI`, `UCF` or `UCM`.
        segment_tag: &'static str,
        /// The DE 0083 action code that is not permitted: `4`, `7` or `8`.
        action: &'static str,
    },

    /// A `CONTRL` message identifier component violates its representation.
    ///
    /// [`ContrlIdentifier`][crate::contrl::ContrlIdentifier] validates the
    /// components of its `UNH` S009 against ISO 9735: DE 0052, DE 0054 and
    /// DE 0051 are mandatory `an..3`, DE 0057 is an optional `an..6` that is
    /// non-empty when given.
    #[error("CONTRL message identifier: data element {data_element} value {value:?} is not valid")]
    InvalidContrlIdentifier {
        /// The S009 component at fault: `0052`, `0054`, `0051` or `0057`.
        data_element: &'static str,
        /// The rejected value.
        value: String,
    },

    /// A segment fits no place at its position in an ordered segment group.
    ///
    /// Raised by [`GroupResolution::findings`][crate::group::GroupResolution::findings]
    /// for every segment the resolver skipped. The segment is never attached to
    /// a wrong group; `reason` says whether it fits a place already passed, a
    /// place elsewhere in the message, or none at all.
    #[error("segment {tag} is not placed: {reason}", tag = escape_control(.tag))]
    SegmentNotPlaced {
        /// Segment tag.
        tag: String,
        /// Why it fits nowhere here.
        reason: crate::group::UnplacedReason,
        /// Byte span of the segment.
        span: Span,
    },

    /// A segment place occurs more often in one group instance than its `max`.
    #[error("segment {tag} occurs more than {max} time(s) in its group", tag = escape_control(.tag))]
    TooManySegmentOccurrences {
        /// Segment tag.
        tag: String,
        /// The place's maximum.
        max: u32,
        /// Byte span of the occurrence beyond the maximum.
        span: Span,
    },

    /// A segment group occurs more often in its parent instance than its `max`.
    #[error("segment group {group} occurs more than {max} time(s)", group = escape_control(.group))]
    TooManyGroupOccurrences {
        /// Group name.
        group: String,
        /// The group's maximum.
        max: u32,
        /// Byte span of the trigger segment of the instance beyond the maximum.
        span: Span,
    },

    /// A group schema is malformed: bounds, tag form, empty code list, or
    /// nesting deeper than 64 groups.
    ///
    /// Raised by [`GroupDefBuilder::build`][crate::group::GroupDefBuilder::build]
    /// and [`GroupSchema`][crate::group::GroupSchema] construction — a
    /// configuration fault, never a fault of a message.
    #[error("invalid group schema at {group:?}: {reason}")]
    InvalidGroupSchema {
        /// Name of the group at fault (the root name for a root entry).
        group: String,
        /// What is wrong.
        reason: &'static str,
    },

    /// A [`crate::Writer`] refused to write after an earlier failure.
    ///
    /// The sink may end in a partial segment or an unclosed message, so
    /// nothing more is written to it. Start a new writer; use
    /// [`Writer::into_inner`][crate::Writer::into_inner] to inspect or discard
    /// what was written.
    #[error("writer is poisoned: {reason}; no further output was written")]
    WriterPoisoned {
        /// What poisoned the writer.
        reason: &'static str,
    },
}

impl From<std::io::Error> for EdifactError {
    /// Wraps `e` as [`Io`][Self::Io], unless it carries an `EdifactError` — a
    /// reader adapter of this crate refusing its input — which is returned as is.
    fn from(e: std::io::Error) -> Self {
        if e.get_ref().is_some_and(|inner| inner.is::<Self>()) {
            if let Some(Ok(inner)) = e.into_inner().map(|inner| inner.downcast::<Self>()) {
                return *inner;
            }
            unreachable!("the payload was checked to be an EdifactError");
        }
        Self::Io(IoError(e))
    }
}

impl EdifactError {
    /// This error with every input offset it carries moved by `base` bytes.
    ///
    /// The reader path tokenizes each segment from its own buffer, so the
    /// offsets a per-segment tokenizer reports are relative to the segment's
    /// first byte; rebasing them makes them absolute, as on the slice path.
    /// Only the lexical variants and the tokenizer's `E058` carry such a
    /// position; every other variant is returned unchanged.
    pub(crate) fn rebased(self, base: usize) -> Self {
        match self {
            Self::UnexpectedEof { offset } => Self::UnexpectedEof {
                offset: offset.saturating_add(base),
            },
            Self::InvalidDelimiter { byte, offset } => Self::InvalidDelimiter {
                byte,
                offset: offset.saturating_add(base),
            },
            Self::InvalidText { offset } => Self::InvalidText {
                offset: offset.saturating_add(base),
            },
            Self::InvalidReleaseSequence { offset } => Self::InvalidReleaseSequence {
                offset: offset.saturating_add(base),
            },
            Self::SegmentTooLong { offset, limit } => Self::SegmentTooLong {
                offset: offset.saturating_add(base),
                limit,
            },
            Self::UnexpectedDataToken { offset } => Self::UnexpectedDataToken {
                offset: offset.saturating_add(base),
            },
            Self::DataAfterInterchangeTrailer { tag, span } => Self::DataAfterInterchangeTrailer {
                tag,
                span: span.offset(base),
            },
            other => other,
        }
    }

    /// Stable diagnostic code for this error variant.
    #[must_use]
    pub const fn stable_code(&self) -> &'static str {
        match self {
            Self::UnexpectedEof { .. } => "E001",
            Self::InvalidDelimiter { .. } => "E002",
            Self::InvalidText { .. } => "E003",
            Self::MessageCountMismatch { .. } => "E004",
            Self::SegmentCountMismatch { .. } => "E005",
            Self::InvalidSegmentTag(_) => "E006",
            Self::InvalidUna => "E007",
            Self::MissingRequiredElement { .. } => "E008",
            Self::InvalidUtf8 => "E009",
            Self::Io(_) => "E010",
            Self::InvalidSegmentForMessage { .. } => "E011",
            Self::InvalidElementCount { .. } => "E012",
            Self::InvalidComponentCount { .. } => "E013",
            Self::InvalidCodeValue { .. } => "E014",
            Self::MissingSegment { .. } => "E015",
            Self::QualifierMismatch { .. } => "E016",
            Self::ConditionalRequirementNotMet { .. } => "E017",
            // E018 retired — never reuse (see error-reference).
            Self::InvalidReleaseSequence { .. } => "E019",
            Self::SegmentTooLong { .. } => "E020",
            Self::MissingRequiredComponent { .. } => "E021",
            // E022 retired — never reuse (see error-reference).
            Self::InterchangeTooLarge { .. } => "E023",
            Self::InvalidEventSequence { .. } => "E024",
            Self::InvalidElementPosition => "E025",
            Self::IncompatibleReleaseScopes { .. } => "E026",
            Self::InvalidFieldValue { .. } => "E027",
            Self::UnexpectedDataToken { .. } => "E028",
            // E029 retired — never reuse (see error-reference).
            Self::ValidationErrors { .. } => "E030",
            Self::UnrecognisedSyntaxIdentifier(_) => "E031",
            Self::DuplicateReference { .. } => "E032",
            Self::UnknownDataElement { .. } => "E033",
            Self::AmbiguousDataElement { .. } => "E034",
            Self::SegmentLayoutMismatch { .. } => "E035",
            Self::LimitExceeded { .. } => "E036",
            Self::RepetitionSeparatorNotDeclared => "E037",
            Self::CharacterNotInRepertoire { .. } => "E038",
            Self::UnsupportedCharset { .. } => "E039",
            Self::NonFiniteNumber { .. } => "E040",
            Self::CharacterRepertoireMismatch { .. } => "E041",
            Self::EmptyInterchange { .. } => "E042",
            Self::EmptyMessage { .. } => "E043",
            Self::PackageNotSupported { .. } => "E044",
            Self::BlankDataElementValue { .. } => "E045",
            Self::SegmentWithoutDataElements { .. } => "E046",
            Self::TooManyRepetitions { .. } => "E047",
            Self::InvalidCharacterType { .. } => "E048",
            Self::DataElementTooLong { .. } => "E049",
            Self::DataElementTooShort { .. } => "E050",
            Self::TrailingSeparator { .. } => "E051",
            Self::GroupsAndMessagesMixed { .. } => "E052",
            Self::InsignificantCharacters { .. } => "E053",
            Self::GroupCountMismatch { .. } => "E054",
            Self::InteractiveSegmentNotSupported { .. } => "E055",
            Self::SecuritySegmentNotSupported { .. } => "E056",
            Self::ReservedServiceSegmentTag { .. } => "E057",
            Self::DataAfterInterchangeTrailer { .. } => "E058",
            Self::UnsupportedSyntaxVersion { .. } => "E059",
            Self::InvalidDateTime { .. } => "E060",
            Self::EmptyGroup { .. } => "E061",
            // E062–E065 are reserved; never reuse them for anything else.
            Self::ActionNotPermitted { .. } => "E066",
            Self::InvalidContrlIdentifier { .. } => "E067",
            Self::SegmentNotPlaced { .. } => "E068",
            Self::TooManySegmentOccurrences { .. } => "E069",
            Self::TooManyGroupOccurrences { .. } => "E070",
            Self::InvalidGroupSchema { .. } => "E071",
            Self::WriterPoisoned { .. } => "E072",
        }
    }

    /// Stable recovery hint for common malformed input and validation cases.
    #[must_use]
    pub fn recovery_hint(&self) -> Option<&'static str> {
        match self {
            Self::UnexpectedEof { .. } => {
                Some("Ensure every segment ends with the configured segment terminator")
            }
            Self::InvalidDelimiter { .. } => {
                Some("Check UNA service string advice and delimiter bytes in the payload")
            }
            Self::InvalidText { .. } => {
                Some("Input must be valid UTF-8 text for segment and element values")
            }
            Self::InvalidReleaseSequence { .. } => {
                Some("Release character must escape one following byte; trailing '?' is invalid")
            }
            Self::InvalidSegmentTag(_) => Some("Segment tags must be 3 ASCII uppercase letters"),
            Self::InvalidUna => Some(
                "UNA must be exactly 9 bytes: 'UNA' followed by 6 service characters, of which the active five must be distinct and non-whitespace",
            ),
            Self::MissingRequiredElement { .. } => {
                Some("Provide all mandatory elements for the segment per directory rules")
            }
            Self::MissingRequiredComponent { .. } => Some(
                "Provide all mandatory components for the composite element per directory rules",
            ),
            Self::InvalidSegmentForMessage { .. } => {
                Some("Remove unsupported segment or switch to the correct message type")
            }
            Self::InvalidElementCount { .. } => {
                Some("Adjust the segment element count to the allowed min/max range")
            }
            Self::InvalidComponentCount { .. } => {
                Some("Fix composite element arity to match the expected component count")
            }
            Self::InvalidCodeValue { .. } => {
                Some("Use a value from the referenced code list for this element")
            }
            Self::MissingSegment { .. } => {
                Some("Insert the required segment at the expected position")
            }
            Self::QualifierMismatch { .. } => {
                Some("Set the segment qualifier to the expected value")
            }
            Self::ConditionalRequirementNotMet { .. } => {
                Some("When the condition is met, include the conditionally required element")
            }
            Self::SegmentTooLong { limit, .. } => {
                let _ = limit; // used in the error message; hint is generic
                Some("Increase max_segment_bytes in ReaderConfig or reject the input as malformed")
            }
            Self::InvalidEventSequence { .. } => Some(
                "Emit StartSegment before Element, and Element before ComponentElement; open and close an interchange with begin_interchange and end_interchange only",
            ),
            Self::InvalidElementPosition => Some(
                "Set element position to a value >= 1; positions are one-based (1 = first element slot)",
            ),
            Self::IncompatibleReleaseScopes { .. } => Some(
                "Only compose ProfileRulePack values that share the same release scope, or where at most one has a release scope set",
            ),
            Self::InvalidFieldValue { .. } => Some(
                "Correct the field value to match the expected format or range for this element",
            ),
            Self::UnexpectedDataToken { .. } => Some(
                "A data element appeared before any segment tag; check for partial writes or encoding corruption",
            ),
            Self::DuplicateReference { .. } => Some(
                "Assign a unique control reference to every UNH (DE 0062) and UNG (DE 0048) within an interchange",
            ),
            Self::UnknownDataElement { .. } => Some(
                "Check the data element identifier against the segment definition; the directory is the source of truth",
            ),
            Self::AmbiguousDataElement { .. } => Some(
                "The code appears at more than one position; address the element positionally instead",
            ),
            Self::SegmentLayoutMismatch { .. } => {
                Some("Resolve codes against the segment definition whose tag matches the segment")
            }
            Self::LimitExceeded { .. } => Some(
                "Raise the corresponding ReaderConfig limit (ReaderConfig::unbounded() for trusted input), or reject the input as oversized",
            ),
            Self::RepetitionSeparatorNotDeclared => Some(
                "Build the writer with Writer::with_una and a ServiceStringAdvice whose repetition_sep is set",
            ),
            Self::CharacterNotInRepertoire { .. } => Some(
                "Transliterate the value into the declared repertoire, or declare a wider one in UNB S001 (UNOC for Latin-1, UNOY for UTF-8)",
            ),
            Self::UnsupportedCharset { .. } => Some(
                "UNOX and KECA are not supported; ask the partner for UNOC or UNOY, or transcode the interchange before parsing",
            ),
            Self::NonFiniteNumber { .. } => Some(
                "EDIFACT has no representation for NaN or infinity; check the calculation, or omit the element",
            ),
            Self::CharacterRepertoireMismatch { .. } => Some(
                "Declare the writer's own repertoire in the header, or leave the writer unbound so begin_interchange binds it",
            ),
            Self::EmptyInterchange { .. } => Some(
                "An interchange must carry at least one message or group; send nothing rather than an empty envelope",
            ),
            Self::EmptyMessage { .. } => Some(
                "A message needs at least one segment between UNH and UNT; omit the message entirely if it has no content",
            ),
            Self::PackageNotSupported { .. } => Some(
                "Split the object out of the byte stream using the length in UNO S022 DE 0810, then parse the remaining segments",
            ),
            Self::BlankDataElementValue { .. } => Some(
                "Omit the data element instead of sending spaces; trailing spaces are insignificant and must be suppressed",
            ),
            Self::SegmentWithoutDataElements { .. } => {
                Some("Supply the segment's data, or omit the segment entirely if it is conditional")
            }
            Self::TooManyRepetitions { .. } => Some(
                "Reduce the occurrences to the definition's maximum, or correct the definition if the directory allows more",
            ),
            Self::InvalidCharacterType { .. } => Some(
                "Send a value of the declared class: `n` admits digits, an optional minus, a decimal mark and an exponent — never a space or a plus sign",
            ),
            Self::DataElementTooLong { .. } => Some(
                "Shorten the value to the declared maximum; length is counted in characters, and a numeric value excludes its sign, decimal mark and exponent",
            ),
            Self::DataElementTooShort { .. } => Some(
                "Pad the value to the declared fixed length, or correct the definition if the directory declares it variable",
            ),
            Self::TrailingSeparator { .. } => Some(
                "Stop emitting separators once the last value has been written; ISO 9735-1 §8.7.1 and §8.7.2 require trailing ones to be omitted",
            ),
            Self::GroupsAndMessagesMixed { .. } => Some(
                "Put every message inside a group, or none of them; ISO 9735-1 §7.1 does not allow both in one interchange",
            ),
            Self::InsignificantCharacters { .. } => Some(
                "Suppress the insignificant characters before sending: leading zeroes in a variable-length numeric value, trailing spaces in a variable-length text one",
            ),
            Self::UnrecognisedSyntaxIdentifier(_) => Some(
                "UNB S001 DE 0001 must name a repertoire this crate recognises: UNOA-UNOK or UNOY",
            ),
            Self::ActionNotPermitted { .. } => Some(
                "Permit the action code at that level in the ActionPolicy, or report the outcome the policy admits (for example a rejection instead of a receipt)",
            ),
            Self::InvalidContrlIdentifier { .. } => Some(
                "S009 version, release and agency must be 1-3 characters; the association assigned code must be 1-6 characters",
            ),
            Self::SegmentNotPlaced { .. } => Some(
                "Move the segment to the place the implementation guide gives it, or remove it if the guide has no place for it",
            ),
            Self::TooManySegmentOccurrences { .. } => {
                Some("Reduce the segment's repetitions in this group to the guide's maximum")
            }
            Self::TooManyGroupOccurrences { .. } => {
                Some("Reduce the segment group's repetitions to the guide's maximum")
            }
            Self::InvalidGroupSchema { .. } => Some(
                "Fix the group schema: tags are three upper-case letters, min <= max, max >= 1, one_of lists at least one code, at most 64 nested groups",
            ),
            Self::WriterPoisoned { .. } => Some(
                "Start a new writer; Writer::into_inner returns the sink with the partial output",
            ),
            Self::GroupCountMismatch { .. } => {
                Some("Set UNE DE 0060 to the number of messages between the UNG and the UNE")
            }
            Self::InteractiveSegmentNotSupported { .. } => Some(
                "Interactive EDI (ISO 9735-3) is not supported; exchange the data as a batch interchange (UNB...UNZ)",
            ),
            Self::SecuritySegmentNotSupported { .. } => Some(
                "Secured EDIFACT (ISO 9735-5/-7/-9) is not supported; verify and remove the security segments before parsing, or agree on unsecured exchange",
            ),
            Self::ReservedServiceSegmentTag { .. } => Some(
                "Tags starting with U are reserved for service segments; rename the segment or remove it",
            ),
            Self::DataAfterInterchangeTrailer { .. } => Some(
                "Split the input at the end of each UNZ segment and parse every interchange on its own",
            ),
            Self::UnsupportedSyntaxVersion { .. } => {
                Some("Declare syntax version 1, 2, 3 or 4 in UNB S001 DE 0002")
            }
            Self::InvalidDateTime { .. } => Some(
                "Send a real calendar date (YYMMDD for syntax versions 1-3, CCYYMMDD for 4) and a time HHMM from 0000 to 2359",
            ),
            Self::EmptyGroup { .. } => {
                Some("Put at least one message in the group, or omit the UNG and UNE")
            }
            Self::ValidationErrors { .. }
            | Self::MessageCountMismatch { .. }
            | Self::SegmentCountMismatch { .. }
            | Self::InterchangeTooLarge { .. }
            | Self::InvalidUtf8
            | Self::Io(_) => None,
        }
    }
}

#[cfg(feature = "diagnostics")]
#[cfg_attr(docsrs, doc(cfg(feature = "diagnostics")))]
impl miette::Diagnostic for EdifactError {
    fn code<'a>(&'a self) -> Option<Box<dyn std::fmt::Display + 'a>> {
        Some(Box::new(self.stable_code()))
    }

    /// Mirrors the severity the validation pipeline assigns.
    ///
    /// The two must agree: a `miette` render that calls a control-reference
    /// mismatch a warning while [`ValidationReport`] files it as an error tells
    /// the operator and the program two different things about the same
    /// interchange. [`crate::ValidationReport`] is the source of truth, and this
    /// delegates to it.
    fn severity(&self) -> Option<miette::Severity> {
        Some(match crate::report::severity_for_error(self) {
            crate::ValidationSeverity::Info => miette::Severity::Advice,
            crate::ValidationSeverity::Warning => miette::Severity::Warning,
            crate::ValidationSeverity::Error | crate::ValidationSeverity::Critical => {
                miette::Severity::Error
            }
        })
    }

    /// The help text, with input-derived values escaped like `Display`.
    fn help<'a>(&'a self) -> Option<Box<dyn std::fmt::Display + 'a>> {
        let text = self.help_text()?.to_string();
        Some(Box::new(EscapedText(text)))
    }
}

/// An owned string displayed through [`escape_control`].
#[cfg(feature = "diagnostics")]
struct EscapedText(String);

#[cfg(feature = "diagnostics")]
impl fmt::Display for EscapedText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        escape_control(&self.0).fmt(f)
    }
}

#[cfg(feature = "diagnostics")]
impl EdifactError {
    /// The unescaped help text.
    fn help_text(&self) -> Option<Box<dyn std::fmt::Display + '_>> {
        match self {
            // Static text — no allocation needed.
            Self::InvalidUna => Some(Box::new(
                "UNA segment must be exactly 9 bytes: 'UNA' + 6 service characters. See EDIFACT spec",
            )),
            Self::InvalidUtf8 => Some(Box::new(
                "Internal error: serialized output contains invalid UTF-8. Please report this as a bug",
            )),
            // Dynamic help text.
            Self::UnexpectedEof { offset } => Some(Box::new(format!(
                "Check that all segments are terminated with the segment terminator (usually '). \
                 Reached end at offset {offset}",
            ))),
            Self::InvalidDelimiter { byte, offset } => Some(Box::new(format!(
                "The byte 0x{byte:02X} at offset {offset} is not a valid delimiter. \
                 Check UNA configuration",
            ))),
            Self::InvalidText { offset } => Some(Box::new(format!(
                "The byte sequence at offset {offset} contains invalid UTF-8. \
                 Ensure input is valid UTF-8",
            ))),
            Self::InvalidReleaseSequence { offset } => Some(Box::new(format!(
                "Release character at offset {offset} is dangling. \
                 Ensure '?' is followed by an escaped byte",
            ))),
            Self::MessageCountMismatch {
                expected, actual, ..
            } => Some(Box::new(format!(
                "UNZ DE 0036 declares {expected} but the interchange holds {actual} message(s), \
                 or group(s) when groups are used",
            ))),
            Self::SegmentCountMismatch {
                expected,
                actual,
                message_ref,
                ..
            } => Some(Box::new(format!(
                "UNT for message {message_ref} declares {expected} segment(s) but {actual} were found. \
                 Check the UNT segment count",
            ))),
            Self::InvalidSegmentTag(tag) => Some(Box::new(format!(
                "Segment tag '{tag}' must be exactly 3 ASCII uppercase letters",
            ))),
            Self::MissingRequiredElement { tag, element_index } => Some(Box::new(format!(
                "Segment {tag} requires element at index {element_index}",
            ))),
            Self::MissingRequiredComponent {
                tag,
                element_index,
                component_index,
            } => Some(Box::new(format!(
                "Segment {tag} element {element_index} requires component at index {component_index}",
            ))),
            Self::Io(e) => Some(Box::new(format!("I/O error: {e}"))),
            Self::InvalidSegmentForMessage {
                tag, message_type, ..
            } => Some(Box::new(format!(
                "Segment {tag} should not appear in a {message_type} message. \
                 Check the directory definition",
            ))),
            Self::InvalidElementCount {
                tag,
                min,
                max,
                actual,
                ..
            } => Some(Box::new(format!(
                "Segment {tag} should have between {min} and {max} elements, but has {actual}. \
                 Check segment structure",
            ))),
            Self::InvalidComponentCount {
                tag,
                element_index,
                expected,
                actual,
                ..
            } => Some(Box::new(format!(
                "In segment {tag}, element {element_index} should have {expected} components \
                     but has {actual}. Check element structure",
            ))),
            Self::InvalidCodeValue {
                tag,
                element_index,
                value,
                code_list,
                ..
            } => Some(Box::new(format!(
                "Value '{value}' in segment {tag} element {element_index} is not in the \
                     {code_list} code list. Check the directory for valid codes",
            ))),
            Self::MissingSegment {
                tag,
                expected_position,
            } => Some(Box::new(format!(
                "Segment {tag} is required at position {expected_position} but is missing. \
                 Add this segment to the message",
            ))),
            Self::QualifierMismatch {
                tag,
                actual,
                expected,
                ..
            } => Some(Box::new(format!(
                "Segment {tag} has qualifier '{actual}' but expected '{expected}'. \
                 Check the segment's first component",
            ))),
            Self::ConditionalRequirementNotMet {
                tag,
                element_index,
                condition,
                ..
            } => Some(Box::new(format!(
                "In segment {tag}, element {element_index} is conditionally required when: \
                     {condition}. Check if the condition is met",
            ))),
            Self::SegmentTooLong { offset, limit } => Some(Box::new(format!(
                "Segment starting at byte offset {offset} exceeds the {limit}-byte limit. \
                 Use ReaderConfig::max_segment_bytes to adjust the limit if needed, \
                 or verify the input for a missing segment terminator",
            ))),
            Self::InterchangeTooLarge { count } => Some(Box::new(format!(
                "Interchange contains {count} items which exceeds the u32::MAX limit. \
                 This is an extremely unusual input; verify the message is not corrupted.",
            ))),
            Self::InvalidEventSequence { message } => Some(Box::new(format!(
                "Event sequence violation: {message}. \
                 Check that StartSegment is emitted before Element, and Element before ComponentElement.",
            ))),
            Self::InvalidElementPosition => Some(Box::new(
                "Element positions must be >= 1 (one-based). \
                 Ensure no OwnedElementRef is constructed with position == 0",
            )),
            Self::IncompatibleReleaseScopes { current, incoming } => Some(Box::new(format!(
                "Release scope {current:?} and {incoming:?} are incompatible. \
                 Only compose ProfileRulePack values that share the same release scope, \
                 or where at most one carries a release scope",
            ))),
            Self::InvalidFieldValue {
                tag,
                element_index,
                value,
            } => Some(Box::new(format!(
                "Segment {tag} element {element_index} has invalid value '{value}'. \
                 Check the expected format or range for this field",
            ))),
            Self::UnexpectedDataToken { offset } => Some(Box::new(format!(
                "Data element at offset {offset} appeared before any segment tag. \
                 Check for partial writes or encoding corruption",
            ))),
            Self::ValidationErrors { error_count, .. } => Some(Box::new(format!(
                "Validation found {error_count} error(s). Inspect the ValidationReport for details",
            ))),
            Self::UnrecognisedSyntaxIdentifier(id) => Some(Box::new(format!(
                "Syntax identifier '{id}' is not a repertoire this crate recognises \
                 (UNOA-UNOK, UNOY). Check UNB S001 DE 0001",
            ))),
            Self::DuplicateReference { tag, reference, .. } => Some(Box::new(format!(
                "Reference '{reference}' is used by more than one {tag} in this interchange; \
                 each must be unique so receivers can address messages unambiguously",
            ))),
            Self::UnknownDataElement { tag, data_element } => Some(Box::new(format!(
                "Segment {tag} does not define data element {data_element}. \
                 Check the identifier against the directory definition for {tag}",
            ))),
            Self::AmbiguousDataElement { tag, data_element } => Some(Box::new(format!(
                "Segment {tag} defines data element {data_element} at more than one position, \
                 so code-addressed access cannot pick one; use a positional accessor",
            ))),
            Self::SegmentLayoutMismatch { expected, actual } => Some(Box::new(format!(
                "The supplied layout describes segment {expected} but was applied to {actual}. \
                 Look up the definition by the segment's own tag",
            ))),
            Self::RepetitionSeparatorNotDeclared => Some(Box::new(
                "UNA position 7 holds the space \"not used\" sentinel, so repeating data \
                 elements cannot be expressed. Use Writer::with_una with a repetition_sep",
            )),
            Self::CharacterNotInRepertoire {
                charset,
                character,
                offset,
            } => Some(Box::new(format!(
                "The character {character:?} at offset {offset} has no representation in {charset}. \
                 Transliterate it, or declare a wider repertoire in UNB S001 DE 0001",
            ))),
            Self::UnsupportedCharset { syntax_identifier } => Some(Box::new(format!(
                "'{syntax_identifier}' is stateful or multi-byte, so byte-level delimiter scanning \
                 would be unsound. Transcode the interchange to UNOC or UNOY before parsing",
            ))),
            Self::CharacterRepertoireMismatch { declared, writer } => Some(Box::new(format!(
                "The UNB declares {declared} but the writer encodes {writer}. \
                 The receiver would decode the body with the wrong table",
            ))),
            Self::NonFiniteNumber { value } => Some(Box::new(format!(
                "The value {value} is not finite. EDIFACT numeric data elements have no \
                 representation for NaN or infinity",
            ))),
            Self::LimitExceeded {
                limit: "sniff_window",
                max,
                offset,
            } => Some(Box::new(format!(
                "The UNB syntax identifier lies outside the {max}-byte sniff window at byte \
                 offset {offset}. Strip the leading whitespace or reject the input",
            ))),
            Self::LimitExceeded { limit, max, offset } => Some(Box::new(format!(
                "The input exceeds the configured {limit} limit of {max} at byte offset {offset}. \
                 Raise it via ReaderConfig (ReaderConfig::unbounded() for trusted input) \
                 if the input is legitimate, or reject the input",
            ))),
            Self::EmptyInterchange { control_ref } => Some(Box::new(format!(
                "Interchange {control_ref} carries no message and no group. \
                 ISO 9735-1 §7.1 requires at least one; send nothing rather than an empty envelope",
            ))),
            Self::EmptyMessage { message_ref, .. } => Some(Box::new(format!(
                "Message {message_ref} has nothing between UNH and UNT. \
                 ISO 9735-1 §7.3 requires at least one additional segment",
            ))),
            Self::PackageNotSupported { tag, .. } => Some(Box::new(format!(
                "{tag} opens or closes a package, whose object is arbitrary binary data \
                 rather than EDIFACT. Split it out using the length in UNO S022 DE 0810, \
                 then parse the remaining segments",
            ))),
            Self::BlankDataElementValue {
                tag,
                element_index,
                component_index,
                ..
            } => Some(Box::new(format!(
                "Segment {tag} element {element_index} component {component_index} holds only \
                 spaces. ISO 9735-1 §9.3 forbids that — omit the element instead",
            ))),
            Self::SegmentWithoutDataElements { tag, .. } => Some(Box::new(format!(
                "Segment {tag} carries only its tag. ISO 9735-1 §7.5 requires at least one \
                 data element; §8.5 says a conditional segment with no data is omitted entirely",
            ))),
            Self::TooManyRepetitions {
                tag,
                element_index,
                max,
                actual,
                ..
            } => Some(Box::new(format!(
                "Segment {tag} element {element_index} occurs {actual} times but its definition \
                 allows at most {max}",
            ))),
            Self::InvalidCharacterType {
                repr, value, tag, ..
            } => Some(Box::new(format!(
                "Segment {tag}: {value:?} is not a valid {repr} value. ISO 9735-1 §10 admits \
                 digits, an optional minus sign, a decimal mark and an exponent — the space \
                 character and the plus sign are not allowed",
            ))),
            Self::DataElementTooLong {
                repr, actual, tag, ..
            } => Some(Box::new(format!(
                "Segment {tag}: the value is {actual} characters, but the directory declares \
                 {repr}. Length is counted in characters rather than bytes",
            ))),
            Self::DataElementTooShort {
                repr, actual, tag, ..
            } => Some(Box::new(format!(
                "Segment {tag}: the value is {actual} characters, but the directory declares the \
                 fixed length {repr}",
            ))),
            Self::TrailingSeparator {
                tag, element_index, ..
            } => Some(Box::new(match element_index {
                Some(index) => format!(
                    "Segment {tag} element {index} ends in a component separator with no value \
                     after it. ISO 9735-1 §8.7.2 requires trailing component separators to be \
                     omitted",
                ),
                None => format!(
                    "Segment {tag} ends in a data element separator with no value after it. \
                     ISO 9735-1 §8.7.1 requires trailing data element separators to be omitted",
                ),
            })),
            Self::GroupsAndMessagesMixed { .. } => Some(Box::new(
                "ISO 9735-1 §7.1 lists what an interchange may contain, and the entries are \
                 exclusive: groups containing messages, or bare messages — never both, because a \
                 message outside every group cannot be counted in UNZ DE 0036",
            )),
            Self::InsignificantCharacters { tag, kind, .. } => Some(Box::new(format!(
                "Segment {tag}: {kind}. ISO 9735-1 §9.1 requires insignificant characters to be \
                 suppressed before transfer",
            ))),
            Self::ActionNotPermitted {
                segment_tag,
                action,
            } => Some(Box::new(format!(
                "The CONTRL would carry action code {action} on {segment_tag}, which the configured \
                 ActionPolicy does not admit. Permit it, or change what is being reported",
            ))),
            Self::InvalidContrlIdentifier {
                data_element,
                value,
            } => Some(Box::new(format!(
                "CONTRL S009 data element {data_element} cannot be {value:?}: DE 0052, 0054 and \
                 0051 are mandatory an..3, DE 0057 is an..6 and non-empty when present",
            ))),
            Self::SegmentNotPlaced { tag, reason, .. } => Some(Box::new(format!(
                "Segment {tag} was skipped: {reason}. The implementation guide walks its places \
                 front to back; a segment is never attached to a place it does not fit",
            ))),
            Self::TooManySegmentOccurrences { tag, max, .. } => Some(Box::new(format!(
                "Segment {tag} may occur at most {max} time(s) at this place in one group instance",
            ))),
            Self::TooManyGroupOccurrences { group, max, .. } => Some(Box::new(format!(
                "Segment group {group} may occur at most {max} time(s) in its parent",
            ))),
            Self::InvalidGroupSchema { group, reason } => Some(Box::new(format!(
                "The group schema is malformed at {group:?}: {reason}",
            ))),
            Self::GroupCountMismatch {
                group_ref,
                expected,
                actual,
                ..
            } => Some(Box::new(format!(
                "UNE of group {group_ref} declares {expected} message(s) but the group holds \
                 {actual}",
            ))),
            Self::InteractiveSegmentNotSupported { tag, .. } => Some(Box::new(format!(
                "{tag} is an interactive-EDI segment (ISO 9735-3); only batch interchanges \
                 (UNB...UNZ) are supported",
            ))),
            Self::SecuritySegmentNotSupported { tag, .. } => Some(Box::new(format!(
                "{tag} is a security segment (ISO 9735-5/-7/-9); secured messages are refused, \
                 not passed unverified",
            ))),
            Self::ReservedServiceSegmentTag { tag, .. } => Some(Box::new(format!(
                "{tag} starts with U, which ISO 9735-1 §7.7 reserves for service segments, but \
                 no service segment has this tag",
            ))),
            Self::DataAfterInterchangeTrailer { tag, .. } => Some(Box::new(format!(
                "{tag} follows the UNZ that ends the interchange. Split concatenated \
                 interchanges and parse each on its own",
            ))),
            Self::UnsupportedSyntaxVersion { value, .. } => Some(Box::new(format!(
                "Syntax version {value:?} is not supported: UNB S001 DE 0002 must be 1, 2, 3 or 4",
            ))),
            Self::InvalidDateTime {
                tag,
                component_index,
                value,
                ..
            } => Some(Box::new(if *component_index == 0 {
                format!(
                    "{tag} S004 DE 0017 {value:?} is no calendar date: YYMMDD (syntax 1-3) or \
                     CCYYMMDD (syntax 4), month 01-12, day valid for the month"
                )
            } else {
                format!("{tag} S004 DE 0019 {value:?} is no time: HHMM, 0000 to 2359")
            })),
            Self::EmptyGroup { group_ref, .. } => Some(Box::new(format!(
                "Group {group_ref} has no message; ISO 9735-1 §7.3 requires at least one",
            ))),
            Self::WriterPoisoned { .. } => Some(Box::new(
                "The output is incomplete and must not be sent. Close every message with \
                 MessageWriter::finish(); after a failed write start a new Writer \
                 (Writer::into_inner returns the sink with the partial output)",
            )),
        }
    }
}

// ── validation report ─────────────────────────────────────────────────────────

pub use crate::report::ValidationReport;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebased_moves_every_lexical_offset_and_nothing_else() {
        let cases = [
            (
                EdifactError::UnexpectedEof { offset: 1 },
                EdifactError::UnexpectedEof { offset: 11 },
            ),
            (
                EdifactError::InvalidDelimiter {
                    byte: b'+',
                    offset: 1,
                },
                EdifactError::InvalidDelimiter {
                    byte: b'+',
                    offset: 11,
                },
            ),
            (
                EdifactError::InvalidText { offset: 1 },
                EdifactError::InvalidText { offset: 11 },
            ),
            (
                EdifactError::InvalidReleaseSequence { offset: 1 },
                EdifactError::InvalidReleaseSequence { offset: 11 },
            ),
            (
                EdifactError::SegmentTooLong {
                    offset: 1,
                    limit: 5,
                },
                EdifactError::SegmentTooLong {
                    offset: 11,
                    limit: 5,
                },
            ),
            (
                EdifactError::UnexpectedDataToken { offset: 1 },
                EdifactError::UnexpectedDataToken { offset: 11 },
            ),
            (
                EdifactError::InvalidSegmentTag("B?".to_owned()),
                EdifactError::InvalidSegmentTag("B?".to_owned()),
            ),
            (EdifactError::InvalidUna, EdifactError::InvalidUna),
            (
                EdifactError::LimitExceeded {
                    limit: "max_segments",
                    max: 1,
                    offset: 1,
                },
                EdifactError::LimitExceeded {
                    limit: "max_segments",
                    max: 1,
                    offset: 1,
                },
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(error.rebased(10), expected);
        }
    }

    #[test]
    fn recovery_hint_exists_for_common_malformed_cases() {
        let err = EdifactError::InvalidReleaseSequence { offset: 10 };
        assert!(err.recovery_hint().is_some());

        let err = EdifactError::InvalidCodeValue {
            tag: "BGM".to_owned(),
            element_index: 0,
            value: "X".to_owned(),
            code_list: "1001".to_owned(),
            span: Span::new(0, 9),
            suggestion: None,
        };
        assert!(err.recovery_hint().is_some());
    }

    #[test]
    fn a_limit_error_names_its_budget_and_position() {
        let err = EdifactError::LimitExceeded {
            limit: "max_message_bytes",
            max: 1_000,
            offset: 63_105,
        };
        assert_eq!(err.stable_code(), "E036");
        assert_eq!(
            err.to_string(),
            "input exceeded the max_message_bytes limit of 1000 \
             at byte offset 63105"
        );
        assert!(err.recovery_hint().is_some());
    }

    #[test]
    fn group_resolution_codes_are_distinct_from_every_other_code() {
        let span = Span::new(0, 3);
        let new = [
            EdifactError::SegmentNotPlaced {
                tag: "LOC".into(),
                reason: crate::group::UnplacedReason::OutOfOrder,
                span,
            },
            EdifactError::TooManySegmentOccurrences {
                tag: "DTM".into(),
                max: 1,
                span,
            },
            EdifactError::TooManyGroupOccurrences {
                group: "SG5".into(),
                max: 1,
                span,
            },
            EdifactError::InvalidGroupSchema {
                group: "SG5".into(),
                reason: "max is 0",
            },
        ];
        let codes: Vec<&str> = new.iter().map(EdifactError::stable_code).collect();
        assert_eq!(codes, ["E068", "E069", "E070", "E071"]);
        for err in &new {
            assert!(err.recovery_hint().is_some(), "{err:?}");
        }
    }

    #[test]
    fn writer_poisoned_has_its_own_code_and_hint() {
        let err = EdifactError::WriterPoisoned {
            reason: "a message was dropped without finish()",
        };
        assert_eq!(err.stable_code(), "E072");
        assert!(err.recovery_hint().is_some());
        assert_eq!(
            err.to_string(),
            "writer is poisoned: a message was dropped without finish(); \
             no further output was written"
        );
    }

    #[test]
    fn service_segment_codes_are_errors_with_a_hint_and_a_contrl_code() {
        use crate::contrl::SyntaxError;
        let span = Span::new(0, 3);
        let cases = [
            (
                EdifactError::GroupCountMismatch {
                    group_ref: "G1".into(),
                    expected: 2,
                    actual: 1,
                    span,
                },
                "E054",
                "29",
            ),
            (
                EdifactError::InteractiveSegmentNotSupported {
                    tag: "UIH".into(),
                    span,
                },
                "E055",
                "47",
            ),
            (
                EdifactError::SecuritySegmentNotSupported {
                    tag: "USH".into(),
                    span,
                },
                "E056",
                "47",
            ),
            (
                EdifactError::ReservedServiceSegmentTag {
                    tag: "UXX".into(),
                    span,
                },
                "E057",
                "15",
            ),
            (
                EdifactError::DataAfterInterchangeTrailer {
                    tag: "UNB".into(),
                    span,
                },
                "E058",
                "33",
            ),
            (
                EdifactError::UnsupportedSyntaxVersion {
                    value: "X".into(),
                    span,
                },
                "E059",
                "2",
            ),
            (
                EdifactError::InvalidDateTime {
                    tag: "UNB".into(),
                    element_index: 3,
                    component_index: 0,
                    value: "261399".into(),
                    span,
                },
                "E060",
                "12",
            ),
            (
                EdifactError::EmptyGroup {
                    group_ref: "G1".into(),
                    span,
                },
                "E061",
                "32",
            ),
        ];
        for (err, code, contrl) in cases {
            assert_eq!(err.stable_code(), code);
            assert!(err.recovery_hint().is_some(), "{code}");
            assert_eq!(SyntaxError::for_error(&err).code(), contrl, "{code}");
            let issue =
                crate::ValidationIssue::new(crate::ValidationSeverity::Error, err.to_string())
                    .with_error_code(code);
            assert_eq!(SyntaxError::for_issue(&issue).code(), contrl, "{code}");
            assert_eq!(
                crate::report::severity_for_error(&err),
                crate::ValidationSeverity::Error,
                "{code}"
            );
        }
    }

    // ── escaping input-derived text ──────────────────────────────────────────

    /// A value carrying every kind of character the escape must neutralise.
    const HOSTILE: &str = "A\r\n\x1b[31m\u{85}\u{2028}\u{2029}\u{202e}\0\tZ";

    fn h() -> String {
        HOSTILE.to_owned()
    }

    #[test]
    fn escape_control_spellings() {
        let escaped = |s: &str| escape_control(s).to_string();
        assert_eq!(escaped("BY"), "BY");
        assert_eq!(escaped("A\r\nB"), r"A\r\nB");
        assert_eq!(escaped("\0\t"), r"\0\t");
        assert_eq!(escaped("\x1b[31m"), r"\u{1b}[31m");
        assert_eq!(escaped("\u{85}"), r"\u{85}");
        assert_eq!(escaped("\u{2028}\u{2029}"), r"\u{2028}\u{2029}");
        assert_eq!(escaped("A\u{202e}gnp"), r"A\u{202e}gnp");
        assert_eq!(escaped("\u{61c}\u{2066}"), r"\u{61c}\u{2066}");
        assert_eq!(escaped("Müller 'x' \"y\" \\"), "Müller 'x' \"y\" \\");
        let once = escaped(HOSTILE);
        assert!(!once.chars().any(needs_escape));
        assert_eq!(escaped(&once), once, "idempotent");
    }

    /// Position of `error`'s variant. No wildcard arm: a new variant does not
    /// compile until it is listed here, and the test below fails until it is
    /// built with hostile values.
    fn variant_index(error: &EdifactError) -> usize {
        use EdifactError as E;
        match error {
            E::UnexpectedEof { .. } => 0,
            E::InvalidDelimiter { .. } => 1,
            E::InvalidText { .. } => 2,
            E::InvalidReleaseSequence { .. } => 3,
            E::MessageCountMismatch { .. } => 4,
            E::SegmentCountMismatch { .. } => 5,
            E::InvalidSegmentTag(..) => 6,
            E::InvalidUna => 7,
            E::MissingRequiredElement { .. } => 8,
            E::MissingRequiredComponent { .. } => 9,
            E::InvalidUtf8 => 10,
            E::Io(..) => 11,
            E::InvalidSegmentForMessage { .. } => 12,
            E::InvalidElementCount { .. } => 13,
            E::InvalidComponentCount { .. } => 14,
            E::InvalidCodeValue { .. } => 15,
            E::MissingSegment { .. } => 16,
            E::QualifierMismatch { .. } => 17,
            E::ConditionalRequirementNotMet { .. } => 18,
            E::ValidationErrors { .. } => 19,
            E::SegmentTooLong { .. } => 20,
            E::InterchangeTooLarge { .. } => 21,
            E::InvalidEventSequence { .. } => 22,
            E::InvalidElementPosition => 23,
            E::IncompatibleReleaseScopes { .. } => 24,
            E::InvalidFieldValue { .. } => 25,
            E::UnexpectedDataToken { .. } => 26,
            E::UnrecognisedSyntaxIdentifier(..) => 27,
            E::DuplicateReference { .. } => 28,
            E::UnknownDataElement { .. } => 29,
            E::AmbiguousDataElement { .. } => 30,
            E::LimitExceeded { .. } => 31,
            E::RepetitionSeparatorNotDeclared => 32,
            E::NonFiniteNumber { .. } => 33,
            E::CharacterNotInRepertoire { .. } => 34,
            E::CharacterRepertoireMismatch { .. } => 35,
            E::UnsupportedCharset { .. } => 36,
            E::EmptyInterchange { .. } => 37,
            E::EmptyMessage { .. } => 38,
            E::PackageNotSupported { .. } => 39,
            E::BlankDataElementValue { .. } => 40,
            E::SegmentWithoutDataElements { .. } => 41,
            E::TooManyRepetitions { .. } => 42,
            E::InvalidCharacterType { .. } => 43,
            E::DataElementTooLong { .. } => 44,
            E::DataElementTooShort { .. } => 45,
            E::TrailingSeparator { .. } => 46,
            E::GroupsAndMessagesMixed { .. } => 47,
            E::InsignificantCharacters { .. } => 48,
            E::GroupCountMismatch { .. } => 49,
            E::InteractiveSegmentNotSupported { .. } => 50,
            E::SecuritySegmentNotSupported { .. } => 51,
            E::ReservedServiceSegmentTag { .. } => 52,
            E::DataAfterInterchangeTrailer { .. } => 53,
            E::UnsupportedSyntaxVersion { .. } => 54,
            E::InvalidDateTime { .. } => 55,
            E::EmptyGroup { .. } => 56,
            E::SegmentLayoutMismatch { .. } => 57,
            E::ActionNotPermitted { .. } => 58,
            E::InvalidContrlIdentifier { .. } => 59,
            E::SegmentNotPlaced { .. } => 60,
            E::TooManySegmentOccurrences { .. } => 61,
            E::TooManyGroupOccurrences { .. } => 62,
            E::InvalidGroupSchema { .. } => 63,
            E::WriterPoisoned { .. } => 64,
        }
    }

    /// Every variant, each string field set to [`HOSTILE`].
    fn hostile_variants() -> Vec<EdifactError> {
        use EdifactError as E;
        vec![
            E::UnexpectedEof { offset: 1 },
            E::InvalidDelimiter { byte: 1, offset: 1 },
            E::InvalidText { offset: 1 },
            E::InvalidReleaseSequence { offset: 1 },
            E::MessageCountMismatch {
                expected: 1,
                actual: 1,
                span: Span::new(0, 1),
            },
            E::SegmentCountMismatch {
                expected: 1,
                actual: 1,
                message_ref: h(),
                span: Span::new(0, 1),
            },
            E::InvalidSegmentTag(h()),
            E::InvalidUna,
            E::MissingRequiredElement {
                tag: h(),
                element_index: 1,
            },
            E::MissingRequiredComponent {
                tag: h(),
                element_index: 1,
                component_index: 1,
            },
            E::InvalidUtf8,
            E::Io(IoError(std::io::Error::other("x"))),
            E::InvalidSegmentForMessage {
                tag: h(),
                message_type: h(),
                span: Span::new(0, 1),
            },
            E::InvalidElementCount {
                tag: h(),
                min: 1,
                max: 1,
                actual: 1,
                span: Span::new(0, 1),
            },
            E::InvalidComponentCount {
                tag: h(),
                element_index: 1,
                expected: 1,
                actual: 1,
                span: Span::new(0, 1),
            },
            E::InvalidCodeValue {
                tag: h(),
                element_index: 1,
                value: h(),
                code_list: h(),
                span: Span::new(0, 1),
                suggestion: None,
            },
            E::MissingSegment {
                tag: h(),
                expected_position: h(),
            },
            E::QualifierMismatch {
                tag: h(),
                actual: h(),
                expected: h(),
                span: Span::new(0, 1),
            },
            E::ConditionalRequirementNotMet {
                tag: h(),
                element_index: 1,
                condition: h(),
                span: Span::new(0, 1),
            },
            E::ValidationErrors {
                error_count: 1,
                report: Box::default(),
            },
            E::SegmentTooLong {
                offset: 1,
                limit: 1,
            },
            E::InterchangeTooLarge { count: 1 },
            E::InvalidEventSequence { message: "x" },
            E::InvalidElementPosition,
            E::IncompatibleReleaseScopes {
                current: h(),
                incoming: h(),
            },
            E::InvalidFieldValue {
                tag: h(),
                element_index: 1,
                value: h(),
            },
            E::UnexpectedDataToken { offset: 1 },
            E::UnrecognisedSyntaxIdentifier(h()),
            E::DuplicateReference {
                tag: h(),
                reference: h(),
                span: Span::new(0, 1),
            },
            E::UnknownDataElement {
                tag: h(),
                data_element: h(),
            },
            E::AmbiguousDataElement {
                tag: h(),
                data_element: h(),
            },
            E::LimitExceeded {
                limit: "x",
                max: 1,
                offset: 1,
            },
            E::RepetitionSeparatorNotDeclared,
            E::NonFiniteNumber { value: h() },
            E::CharacterNotInRepertoire {
                charset: "x",
                character: '\u{1b}',
                offset: 1,
            },
            E::CharacterRepertoireMismatch {
                declared: h(),
                writer: "x",
            },
            E::UnsupportedCharset {
                syntax_identifier: h(),
            },
            E::EmptyInterchange { control_ref: h() },
            E::EmptyMessage {
                message_ref: h(),
                span: Span::new(0, 1),
            },
            E::PackageNotSupported {
                tag: h(),
                span: Span::new(0, 1),
            },
            E::BlankDataElementValue {
                tag: h(),
                element_index: 1,
                component_index: 1,
                span: Span::new(0, 1),
            },
            E::SegmentWithoutDataElements {
                tag: h(),
                span: Span::new(0, 1),
            },
            E::TooManyRepetitions {
                tag: h(),
                element_index: 1,
                max: 1,
                actual: 1,
                span: Span::new(0, 1),
            },
            E::InvalidCharacterType {
                tag: h(),
                element_index: 1,
                component_index: 1,
                repr: h(),
                value: h(),
                span: Span::new(0, 1),
            },
            E::DataElementTooLong {
                tag: h(),
                element_index: 1,
                component_index: 1,
                repr: h(),
                actual: 1,
                span: Span::new(0, 1),
            },
            E::DataElementTooShort {
                tag: h(),
                element_index: 1,
                component_index: 1,
                repr: h(),
                actual: 1,
                span: Span::new(0, 1),
            },
            E::TrailingSeparator {
                tag: h(),
                element_index: Some(1),
                span: Span::new(0, 1),
            },
            E::GroupsAndMessagesMixed {
                span: Span::new(0, 1),
            },
            E::InsignificantCharacters {
                tag: h(),
                element_index: 1,
                component_index: 1,
                kind: Insignificant::TrailingSpaces,
                span: Span::new(0, 1),
            },
            E::GroupCountMismatch {
                group_ref: h(),
                expected: 1,
                actual: 1,
                span: Span::new(0, 1),
            },
            E::InteractiveSegmentNotSupported {
                tag: h(),
                span: Span::new(0, 1),
            },
            E::SecuritySegmentNotSupported {
                tag: h(),
                span: Span::new(0, 1),
            },
            E::ReservedServiceSegmentTag {
                tag: h(),
                span: Span::new(0, 1),
            },
            E::DataAfterInterchangeTrailer {
                tag: h(),
                span: Span::new(0, 1),
            },
            E::UnsupportedSyntaxVersion {
                value: h(),
                span: Span::new(0, 1),
            },
            E::InvalidDateTime {
                tag: h(),
                element_index: 1,
                component_index: 1,
                value: h(),
                span: Span::new(0, 1),
            },
            E::EmptyGroup {
                group_ref: h(),
                span: Span::new(0, 1),
            },
            E::SegmentLayoutMismatch {
                expected: h(),
                actual: h(),
            },
            E::ActionNotPermitted {
                segment_tag: "x",
                action: "x",
            },
            E::InvalidContrlIdentifier {
                data_element: "x",
                value: h(),
            },
            E::SegmentNotPlaced {
                tag: h(),
                reason: crate::group::UnplacedReason::Unknown,
                span: Span::new(0, 1),
            },
            E::TooManySegmentOccurrences {
                tag: h(),
                max: 1,
                span: Span::new(0, 1),
            },
            E::TooManyGroupOccurrences {
                group: h(),
                max: 1,
                span: Span::new(0, 1),
            },
            E::InvalidGroupSchema {
                group: h(),
                reason: "x",
            },
            E::WriterPoisoned { reason: "x" },
        ]
    }

    #[test]
    fn display_escapes_every_variant() {
        let errors = hostile_variants();
        let mut seen: Vec<usize> = errors.iter().map(variant_index).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(
            seen,
            (0..errors.len()).collect::<Vec<_>>(),
            "every variant once"
        );
        for error in &errors {
            // `Io` displays the caller's own reader error unchanged
            // (`#[error(transparent)]`); the reader is trusted.
            if matches!(error, EdifactError::Io(_)) {
                continue;
            }
            let text = error.to_string();
            assert!(
                !text.chars().any(needs_escape),
                "{error:?} displays {text:?}"
            );
            #[cfg(feature = "diagnostics")]
            if let Some(help) = miette::Diagnostic::help(error) {
                let help = help.to_string();
                assert!(!help.chars().any(needs_escape), "{error:?} help {help:?}");
            }
        }
    }

    #[test]
    fn clean_values_display_unchanged() {
        let error = EdifactError::QualifierMismatch {
            tag: "NAD".to_owned(),
            actual: "BY".to_owned(),
            expected: "SU".to_owned(),
            span: Span::new(0, 1),
        };
        assert_eq!(
            error.to_string(),
            "segment NAD has qualifier 'BY', expected 'SU'"
        );
    }
}
