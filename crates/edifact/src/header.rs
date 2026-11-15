//! The interchange header (`UNB`), typed: parse it, change it, write it.
//!
//! One [`InterchangeHeader`] serves every direction: [`InterchangeHeader::parse`]
//! reads a `UNB` segment, [`InterchangeHeader::read`] and
//! [`InterchangeHeader::read_from`] read it from raw input before the body is
//! parsed, [`validate_envelope`](crate::validate_envelope) carries it, and
//! [`Writer::begin_interchange`](crate::Writer::begin_interchange) and
//! [`InterchangeBuilder`](crate::InterchangeBuilder) write it.
//!
//! Positions follow ISO 9735-10:2002 §5.1.5 (`UNB`); the interchange structure
//! is ISO 9735-1:2002 §7.2. Nothing is defaulted: a missing or invalid position
//! is an error, never a guessed value.
//!
//! ```
//! use edifact::{Charset, InterchangeHeader, Party, PreparationDateTime, SyntaxVersion, from_bytes};
//!
//! let segments: Vec<_> = from_bytes(b"UNB+UNOC:3+SENDER:14+RECEIVER:14+260802:0915+REF1'")
//!     .collect::<Result<_, _>>()?;
//! let header = InterchangeHeader::parse(&segments[0])?;
//! assert_eq!(header.charset, Charset::UnoC);
//! assert_eq!(header.syntax_version, SyntaxVersion::V3);
//! assert_eq!(header.sender, Party::new("SENDER").with_qualifier("14"));
//! // Version 3 transfers no century: the caller states it.
//! assert_eq!(header.prepared.with_century(20)?, PreparationDateTime::new(2026, 8, 2, 9, 15)?);
//! # Ok::<(), edifact::EdifactError>(())
//! ```

use crate::{
    charset::Charset,
    error::EdifactError,
    model::{Element, OwnedSegment, Segment, Span},
    parser::ReaderConfig,
};
use std::io::{Chain, Cursor, Read};

/// Syntax version number, `UNB` S001 DE 0002.
///
/// Versions 1 and 2 use the version 3 representations. Syntax `X` and any other
/// value are refused with [`EdifactError::UnsupportedSyntaxVersion`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SyntaxVersion {
    /// Version 1.
    V1,
    /// Version 2.
    V2,
    /// Version 3.
    V3,
    /// Version 4 (ISO 9735:1998 and later).
    V4,
}

impl SyntaxVersion {
    /// The DE 0002 value, `1`–`4`.
    #[must_use]
    pub const fn number(self) -> u8 {
        match self {
            Self::V1 => 1,
            Self::V2 => 2,
            Self::V3 => 3,
            Self::V4 => 4,
        }
    }

    /// The version a DE 0002 number names.
    pub(crate) const fn from_number(number: u8) -> Option<Self> {
        match number {
            1 => Some(Self::V1),
            2 => Some(Self::V2),
            3 => Some(Self::V3),
            4 => Some(Self::V4),
            _ => None,
        }
    }
}

/// The year of a [`PreparationDateTime`].
///
/// Syntax versions 1–3 transfer `YYMMDD`: the century is not in the data and
/// stays unknown until [`PreparationDateTime::with_century`] states it.
/// Neither variant orders against the other.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Year {
    /// A two-digit year, `0`–`99`, century unknown.
    TwoDigit(u8),
    /// A full year, `0`–`9999`.
    Full(u16),
}

/// Date and time of preparation, `UNB` S004: a calendar date and an
/// hour and minute.
///
/// Valid by construction. Versions 1–3 write `YYMMDD`, version 4 writes
/// `CCYYMMDD`; a version 4 header refuses a two-digit year
/// ([`EdifactError::InvalidDateTime`]), because its century is unknown.
///
/// ```
/// use edifact::{PreparationDateTime, Year};
///
/// let parsed = PreparationDateTime::from_two_digit_year(99, 1, 1, 0, 0)?;
/// assert_eq!(parsed.year(), Year::TwoDigit(99));
/// assert_eq!(parsed.with_century(19)?.year(), Year::Full(1999));
/// // Not a date: the 13th month.
/// assert!(PreparationDateTime::new(2026, 13, 1, 0, 0).is_err());
/// # Ok::<(), edifact::EdifactError>(())
/// ```
///
/// A two-digit year never compares against a full one:
///
/// ```compile_fail
/// use edifact::PreparationDateTime;
/// let a = PreparationDateTime::from_two_digit_year(99, 1, 1, 0, 0).unwrap();
/// let b = PreparationDateTime::new(2099, 1, 1, 0, 0).unwrap();
/// let _ = a < b;
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(try_from = "RawDateTime", into = "RawDateTime")
)]
pub struct PreparationDateTime {
    year: Year,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
}

/// The serde form of [`PreparationDateTime`], validated on the way in.
#[cfg(feature = "serde")]
#[derive(serde::Serialize, serde::Deserialize)]
struct RawDateTime {
    year: Year,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
}

#[cfg(feature = "serde")]
impl TryFrom<RawDateTime> for PreparationDateTime {
    type Error = EdifactError;

    fn try_from(raw: RawDateTime) -> Result<Self, EdifactError> {
        Self::checked(raw.year, raw.month, raw.day, raw.hour, raw.minute)
    }
}

#[cfg(feature = "serde")]
impl From<PreparationDateTime> for RawDateTime {
    fn from(value: PreparationDateTime) -> Self {
        Self {
            year: value.year,
            month: value.month,
            day: value.day,
            hour: value.hour,
            minute: value.minute,
        }
    }
}

/// `E060` for S004 component `component` (0 date, 1 time) of a `UNB`.
fn invalid_date_time(component: usize, value: String) -> EdifactError {
    EdifactError::InvalidDateTime {
        tag: "UNB".to_owned(),
        element_index: 3,
        component_index: component,
        value,
        span: Span::default(),
    }
}

impl PreparationDateTime {
    /// A date with a full year (`0`–`9999`) and a time.
    ///
    /// # Errors
    ///
    /// [`EdifactError::InvalidDateTime`] when the date is not in the calendar
    /// or the time is not 00:00–23:59.
    pub fn new(year: u16, month: u8, day: u8, hour: u8, minute: u8) -> Result<Self, EdifactError> {
        Self::checked(Year::Full(year), month, day, hour, minute)
    }

    /// A date with a two-digit year (`0`–`99`), as versions 1–3 transfer it.
    ///
    /// 29 February is valid when `yy` is divisible by 4: the century is unknown.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn from_two_digit_year(
        yy: u8,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
    ) -> Result<Self, EdifactError> {
        Self::checked(Year::TwoDigit(yy), month, day, hour, minute)
    }

    fn checked(year: Year, month: u8, day: u8, hour: u8, minute: u8) -> Result<Self, EdifactError> {
        let (leap, year_ok) = match year {
            Year::TwoDigit(yy) => (yy % 4 == 0, yy <= 99),
            Year::Full(y) => (crate::envelope::is_leap_year(y), y <= 9999),
        };
        let date_ok = year_ok && day >= 1 && day <= crate::envelope::days_in_month(month, leap);
        let value = Self {
            year,
            month,
            day,
            hour,
            minute,
        };
        if !date_ok {
            return Err(invalid_date_time(0, value.date_text()));
        }
        if !crate::envelope::is_valid_time(u16::from(hour), u16::from(minute)) {
            return Err(invalid_date_time(1, value.time_text()));
        }
        Ok(value)
    }

    /// The year.
    #[must_use]
    pub const fn year(self) -> Year {
        self.year
    }

    /// The month, `1`–`12`.
    #[must_use]
    pub const fn month(self) -> u8 {
        self.month
    }

    /// The day of the month.
    #[must_use]
    pub const fn day(self) -> u8 {
        self.day
    }

    /// The hour, `0`–`23`.
    #[must_use]
    pub const fn hour(self) -> u8 {
        self.hour
    }

    /// The minute, `0`–`59`.
    #[must_use]
    pub const fn minute(self) -> u8 {
        self.minute
    }

    /// Resolve a two-digit year to `century * 100 + yy`; a full year is kept.
    ///
    /// The only way a two-digit year gains a century.
    ///
    /// # Errors
    ///
    /// [`EdifactError::InvalidDateTime`] when the resolved date does not exist
    /// (29 February in a century year that is no leap year, such as 1900).
    pub fn with_century(self, century: u16) -> Result<Self, EdifactError> {
        match self.year {
            Year::Full(_) => Ok(self),
            Year::TwoDigit(yy) => Self::checked(
                Year::Full(century.saturating_mul(100).saturating_add(u16::from(yy))),
                self.month,
                self.day,
                self.hour,
                self.minute,
            ),
        }
    }

    fn date_text(self) -> String {
        match self.year {
            Year::TwoDigit(yy) => format!("{yy:02}{:02}{:02}", self.month, self.day),
            Year::Full(y) => format!("{y:04}{:02}{:02}", self.month, self.day),
        }
    }

    fn time_text(self) -> String {
        format!("{:02}{:02}", self.hour, self.minute)
    }

    /// S004 as DE 0017 and DE 0019 for `version`.
    pub(crate) fn format_for(self, version: SyntaxVersion) -> Result<[String; 2], EdifactError> {
        let date = match (version, self.year) {
            (SyntaxVersion::V4, Year::Full(y)) => {
                format!("{y:04}{:02}{:02}", self.month, self.day)
            }
            (SyntaxVersion::V4, Year::TwoDigit(_)) => {
                return Err(invalid_date_time(0, self.date_text()));
            }
            (_, Year::Full(y)) => format!("{:02}{:02}{:02}", y % 100, self.month, self.day),
            (_, Year::TwoDigit(yy)) => format!("{yy:02}{:02}{:02}", self.month, self.day),
        };
        Ok([date, self.time_text()])
    }

    /// S004 from DE 0017 (`n6` two-digit, `n8` full year) and DE 0019 (`n4`).
    fn parse(date: &str, time: &str) -> Option<Self> {
        let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
        if !digits(date) || !digits(time) || time.len() != 4 {
            return None;
        }
        let num = |s: &str| s.parse::<u16>().ok();
        let (year, rest) = match date.len() {
            6 => (Year::TwoDigit(num(&date[..2])? as u8), &date[2..]),
            8 => (Year::Full(num(&date[..4])?), &date[4..]),
            _ => return None,
        };
        let (month, day) = (num(&rest[..2])? as u8, num(&rest[2..])? as u8);
        let (hour, minute) = (num(&time[..2])? as u8, num(&time[2..])? as u8);
        Self::checked(year, month, day, hour, minute).ok()
    }
}

/// An interchange sender (S002) or recipient (S003).
///
/// ```
/// use edifact::Party;
///
/// let sender = Party::new("SENDER").with_qualifier("14").with_internal_id("DEPT");
/// assert_eq!(sender.qualifier.as_deref(), Some("14"));
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Party {
    /// Identification, DE 0004 (sender) / DE 0010 (recipient).
    pub id: String,
    /// Identification code qualifier, DE 0007.
    pub qualifier: Option<String>,
    /// Internal identification, DE 0008 (sender) / DE 0014 (recipient).
    pub internal_id: Option<String>,
    /// Internal sub-identification, DE 0042 (sender) / DE 0046 (recipient).
    pub internal_sub_id: Option<String>,
}

impl Party {
    /// A party with identification `id` and nothing else.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            qualifier: None,
            internal_id: None,
            internal_sub_id: None,
        }
    }

    /// Set the identification code qualifier (DE 0007).
    #[must_use]
    pub fn with_qualifier(mut self, qualifier: impl Into<String>) -> Self {
        self.qualifier = Some(qualifier.into());
        self
    }

    /// Set the internal identification (DE 0008 / DE 0014).
    #[must_use]
    pub fn with_internal_id(mut self, id: impl Into<String>) -> Self {
        self.internal_id = Some(id.into());
        self
    }

    /// Set the internal sub-identification (DE 0042 / DE 0046).
    #[must_use]
    pub fn with_internal_sub_id(mut self, id: impl Into<String>) -> Self {
        self.internal_sub_id = Some(id.into());
        self
    }

    fn components(&self) -> Vec<String> {
        vec![
            self.id.clone(),
            opt(&self.qualifier),
            opt(&self.internal_id),
            opt(&self.internal_sub_id),
        ]
    }

    fn from_element(seg: &Segment<'_>, element: usize) -> Self {
        Self {
            id: text(seg, element, 0),
            qualifier: optional(seg, element, 1),
            internal_id: optional(seg, element, 2),
            internal_sub_id: optional(seg, element, 3),
        }
    }
}

/// The interchange header, `UNB`, with every position the standard defines.
///
/// ```
/// use edifact::{Charset, InterchangeHeader, Party, PreparationDateTime, SyntaxVersion};
///
/// let header = InterchangeHeader::new(
///     Charset::UnoC,
///     SyntaxVersion::V3,
///     Party::new("SENDER").with_qualifier("14"),
///     Party::new("RECEIVER").with_qualifier("14"),
///     PreparationDateTime::new(2026, 8, 2, 9, 15)?,
///     "REF1",
/// )
/// .with_test_indicator(true);
/// let unb = header.to_segment()?;
/// assert_eq!(
///     edifact::segments_to_bytes([&unb])?,
///     b"UNB+UNOC:3+SENDER:14+RECEIVER:14+260802:0915+REF1++++++1'".to_vec(),
/// );
/// # Ok::<(), edifact::EdifactError>(())
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct InterchangeHeader {
    /// Syntax identifier, S001 DE 0001.
    pub charset: Charset,
    /// Syntax version number, S001 DE 0002.
    pub syntax_version: SyntaxVersion,
    /// Service code list directory version number, S001 DE 0080.
    pub service_code_list_version: Option<String>,
    /// Character encoding, S001 DE 0133, as transferred.
    pub character_encoding: Option<String>,
    /// Syntax release number, S001 DE 0076, as transferred.
    pub syntax_release: Option<String>,
    /// Interchange sender, S002.
    pub sender: Party,
    /// Interchange recipient, S003.
    pub recipient: Party,
    /// Date and time of preparation, S004.
    pub prepared: PreparationDateTime,
    /// Interchange control reference, DE 0020.
    pub control_ref: String,
    /// Recipient reference/password, S005 DE 0022.
    pub recipient_password: Option<String>,
    /// Recipient reference/password qualifier, S005 DE 0025.
    pub recipient_password_qualifier: Option<String>,
    /// Application reference, DE 0026.
    pub application_ref: Option<String>,
    /// Processing priority code, DE 0029.
    pub processing_priority: Option<String>,
    /// Acknowledgement request, DE 0031 (`1`).
    pub acknowledgement_requested: bool,
    /// Interchange agreement identifier, DE 0032.
    pub communications_agreement_id: Option<String>,
    /// Test indicator, DE 0035 (`1`).
    pub test_indicator: bool,
}

impl InterchangeHeader {
    /// A header with the mandatory positions; every optional one absent.
    #[must_use]
    pub fn new(
        charset: Charset,
        syntax_version: SyntaxVersion,
        sender: Party,
        recipient: Party,
        prepared: PreparationDateTime,
        control_ref: impl Into<String>,
    ) -> Self {
        Self {
            charset,
            syntax_version,
            service_code_list_version: None,
            character_encoding: None,
            syntax_release: None,
            sender,
            recipient,
            prepared,
            control_ref: control_ref.into(),
            recipient_password: None,
            recipient_password_qualifier: None,
            application_ref: None,
            processing_priority: None,
            acknowledgement_requested: false,
            communications_agreement_id: None,
            test_indicator: false,
        }
    }

    /// Set S005: the recipient reference/password and its qualifier.
    #[must_use]
    pub fn with_recipient_password(
        mut self,
        password: impl Into<String>,
        qualifier: Option<String>,
    ) -> Self {
        self.recipient_password = Some(password.into());
        self.recipient_password_qualifier = qualifier;
        self
    }

    /// Set the application reference (DE 0026).
    #[must_use]
    pub fn with_application_ref(mut self, reference: impl Into<String>) -> Self {
        self.application_ref = Some(reference.into());
        self
    }

    /// Set the processing priority code (DE 0029).
    #[must_use]
    pub fn with_processing_priority(mut self, priority: impl Into<String>) -> Self {
        self.processing_priority = Some(priority.into());
        self
    }

    /// Request an acknowledgement (DE 0031 `1`).
    #[must_use]
    pub fn with_acknowledgement_requested(mut self, requested: bool) -> Self {
        self.acknowledgement_requested = requested;
        self
    }

    /// Set the interchange agreement identifier (DE 0032).
    #[must_use]
    pub fn with_communications_agreement_id(mut self, id: impl Into<String>) -> Self {
        self.communications_agreement_id = Some(id.into());
        self
    }

    /// Mark the interchange as a test (DE 0035 `1`).
    #[must_use]
    pub fn with_test_indicator(mut self, test: bool) -> Self {
        self.test_indicator = test;
        self
    }

    /// Set the service code list directory version number (S001 DE 0080).
    #[must_use]
    pub fn with_service_code_list_version(mut self, version: impl Into<String>) -> Self {
        self.service_code_list_version = Some(version.into());
        self
    }

    /// Set the character encoding (S001 DE 0133).
    #[must_use]
    pub fn with_character_encoding(mut self, encoding: impl Into<String>) -> Self {
        self.character_encoding = Some(encoding.into());
        self
    }

    /// Set the syntax release number (S001 DE 0076).
    #[must_use]
    pub fn with_syntax_release(mut self, release: impl Into<String>) -> Self {
        self.syntax_release = Some(release.into());
        self
    }

    /// Whether the test indicator (DE 0035) is set.
    #[must_use]
    pub fn is_test(&self) -> bool {
        self.test_indicator
    }

    /// Whether an acknowledgement is requested (DE 0031).
    #[must_use]
    pub fn ack_requested(&self) -> bool {
        self.acknowledgement_requested
    }

    /// Parse a `UNB` segment.
    ///
    /// Every position is checked for presence, class and length under the
    /// declared syntax version, S004 must be a calendar date and time, and
    /// DE 0031/DE 0035 must be `1` when present. The first finding is returned;
    /// nothing is defaulted.
    ///
    /// # Errors
    ///
    /// - [`EdifactError::MissingSegment`] when `unb` is not a `UNB` (or the
    ///   refusal for an interactive or other `U` segment, e.g. `E055`).
    /// - [`EdifactError::UnsupportedSyntaxVersion`] for DE 0002 other than `1`–`4`.
    /// - `E008`/`E021` for a missing position, `E048`–`E050` for a value of the
    ///   wrong class or length, [`EdifactError::InvalidDateTime`] for S004.
    /// - [`EdifactError::UnrecognisedSyntaxIdentifier`] /
    ///   [`EdifactError::UnsupportedCharset`] for DE 0001.
    /// - [`EdifactError::InvalidCodeValue`] for DE 0031/DE 0035 other than `1`.
    pub fn parse(unb: &Segment<'_>) -> Result<Self, EdifactError> {
        if unb.tag() != "UNB" {
            return Err(not_unb(unb));
        }
        let mut findings = Vec::new();
        let version = crate::envelope::syntax_version(unb)?;
        crate::directory_validator::check_service_segment(unb, version, &mut findings);
        crate::envelope::check_s004(unb, version, &mut findings);
        check_flags(unb, &mut findings);
        if let Some(first) = findings.into_iter().next() {
            return Err(first);
        }
        Self::build(unb)
    }

    /// The typed header when the types can be built: a known repertoire and
    /// version, both party ids, a calendar S004 and a control reference.
    /// Representation findings are the envelope's to report.
    pub(crate) fn from_unb_lenient(unb: &Segment<'_>) -> Option<Self> {
        Self::build(unb).ok()
    }

    fn build(unb: &Segment<'_>) -> Result<Self, EdifactError> {
        let missing = |element_index: usize| EdifactError::MissingRequiredElement {
            tag: "UNB".to_owned(),
            element_index,
        };
        let charset = Charset::from_syntax_identifier(&text(unb, 0, 0))?;
        let syntax_version = crate::envelope::syntax_version(unb)?
            .and_then(SyntaxVersion::from_number)
            .ok_or_else(|| missing(0))?;
        let sender = Party::from_element(unb, 1);
        let recipient = Party::from_element(unb, 2);
        if sender.id.is_empty() {
            return Err(missing(1));
        }
        if recipient.id.is_empty() {
            return Err(missing(2));
        }
        let (date, time) = (text(unb, 3, 0), text(unb, 3, 1));
        let prepared =
            PreparationDateTime::parse(&date, &time).ok_or_else(|| invalid_date_time(0, date))?;
        let control_ref = text(unb, 4, 0);
        if control_ref.is_empty() {
            return Err(missing(4));
        }
        Ok(Self {
            charset,
            syntax_version,
            service_code_list_version: optional(unb, 0, 2),
            character_encoding: optional(unb, 0, 3),
            syntax_release: optional(unb, 0, 4),
            sender,
            recipient,
            prepared,
            control_ref,
            recipient_password: optional(unb, 5, 0),
            recipient_password_qualifier: optional(unb, 5, 1),
            application_ref: optional(unb, 6, 0),
            processing_priority: optional(unb, 7, 0),
            acknowledgement_requested: unb.component_str(8, 0) == Some("1"),
            communications_agreement_id: optional(unb, 9, 0),
            test_indicator: unb.component_str(10, 0) == Some("1"),
        })
    }

    /// Read the header from the head of an interchange.
    ///
    /// Applies the prologue rule (byte order mark and whitespace), an optional
    /// `UNA`, and decodes the values with the repertoire S001 names. Reads
    /// nothing after the `UNB` terminator.
    ///
    /// ```
    /// use edifact::InterchangeHeader;
    ///
    /// let mut raw = b"UNB+UNOC:3+M".to_vec();
    /// raw.push(0xFC); // `ü` in ISO 8859-1
    /// raw.extend_from_slice(b"LLER+R+260101:0900+1'UNH+1+ORDERS:D:96A:UN'");
    /// let header = InterchangeHeader::read(&raw)?;
    /// assert_eq!(header.sender.id, "MüLLER");
    /// # Ok::<(), edifact::EdifactError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// As [`parse`](Self::parse), any parse error of the head, and
    /// [`EdifactError::LimitExceeded`] when no `UNB` terminator lies within
    /// the bound of [`read_from`](Self::read_from) under the default
    /// [`ReaderConfig`].
    pub fn read(input: &[u8]) -> Result<Self, EdifactError> {
        let config = ReaderConfig::default();
        let bound = read_bound(&config);
        let view = &input[..input.len().min(bound)];
        let end = locate(view, bound, view.len() == input.len())?
            .expect("a complete input is always located");
        parse_head(&input[..end], config)
    }

    /// Read the header from a reader, and hand the stream back for parsing.
    ///
    /// The returned reader yields every byte consumed, then the rest of the
    /// stream.
    ///
    /// # Resource bounds
    ///
    /// Consumes at most 4 096 (prologue) + 9 (`UNA`) +
    /// [`max_segment_bytes`](ReaderConfig::max_segment_bytes) bytes, in
    /// reads of at most 4 KiB, and stops reading at the `UNB` terminator.
    /// Holds what it consumed; no other limit of `config` applies here.
    ///
    /// ```
    /// use edifact::{InterchangeHeader, ReaderConfig, from_reader};
    ///
    /// let raw: &[u8] = b"UNB+UNOC:3+S+R+260101:0900+1'UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+3+1'UNZ+1+1'";
    /// let (header, rest) = InterchangeHeader::read_from(raw, &ReaderConfig::default())?;
    /// assert_eq!(header.sender.id, "S");
    /// // The returned reader replays the header: parse the whole interchange.
    /// let segments: Vec<_> = from_reader(rest).collect::<Result<_, _>>()?;
    /// assert_eq!(segments.len(), 5);
    /// # Ok::<(), edifact::EdifactError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// As [`read`](Self::read) under `config`, [`EdifactError::LimitExceeded`]
    /// when no `UNB` terminator lies within the bound, and any I/O error.
    #[allow(clippy::type_complexity)]
    pub fn read_from<R: Read>(
        mut reader: R,
        config: &ReaderConfig,
    ) -> Result<(Self, Chain<Cursor<Vec<u8>>, R>), EdifactError> {
        let bound = read_bound(config);
        let mut buf = Vec::with_capacity(bound.min(CHUNK));
        let end = loop {
            let start = buf.len();
            let want = (bound - start).min(CHUNK);
            buf.resize(start + want, 0);
            let read = loop {
                match reader.read(&mut buf[start..]) {
                    Ok(n) => break n,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(e.into()),
                }
            };
            buf.truncate(start + read);
            if let Some(end) = locate(&buf, bound, read == 0)? {
                break end;
            }
        };
        let header = parse_head(&buf[..end], *config)?;
        Ok((header, Cursor::new(buf).chain(reader)))
    }

    /// The `UNB` segment, validated for the header's syntax version.
    ///
    /// Positions are written in order; an empty position is kept only before a
    /// present one.
    ///
    /// # Errors
    ///
    /// The first finding of the checks [`parse`](Self::parse) applies, and
    /// [`EdifactError::InvalidDateTime`] for a two-digit year under version 4.
    pub fn to_segment(&self) -> Result<OwnedSegment, EdifactError> {
        let [date, time] = self.prepared.format_for(self.syntax_version)?;
        let flag = |set: bool| if set { "1".to_owned() } else { String::new() };
        let elements: Vec<Vec<String>> = vec![
            vec![
                self.charset.syntax_identifier().to_owned(),
                self.syntax_version.number().to_string(),
                opt(&self.service_code_list_version),
                opt(&self.character_encoding),
                opt(&self.syntax_release),
            ],
            self.sender.components(),
            self.recipient.components(),
            vec![date, time],
            vec![self.control_ref.clone()],
            vec![
                opt(&self.recipient_password),
                opt(&self.recipient_password_qualifier),
            ],
            vec![opt(&self.application_ref)],
            vec![opt(&self.processing_priority)],
            vec![flag(self.acknowledgement_requested)],
            vec![opt(&self.communications_agreement_id)],
            vec![flag(self.test_indicator)],
        ];
        let segment = crate::writer::canonical(OwnedSegment::new(
            "UNB",
            elements.iter().map(|c| Element::of(c)).collect(),
        ));
        let mut findings = Vec::new();
        crate::directory_validator::check_service_segment(
            &segment,
            Some(self.syntax_version.number()),
            &mut findings,
        );
        match findings.into_iter().next() {
            Some(first) => Err(first),
            None => Ok(segment),
        }
    }
}

/// `E014` for a DE 0031 or DE 0035 value other than `1`.
pub(crate) fn check_flags(unb: &Segment<'_>, out: &mut Vec<EdifactError>) {
    for (element, code) in [(8, "0031"), (10, "0035")] {
        match unb.component_str(element, 0) {
            None | Some("" | "1") => {}
            Some(value) => out.push(EdifactError::InvalidCodeValue {
                tag: "UNB".to_owned(),
                element_index: element,
                value: value.to_owned(),
                code_list: code.to_owned(),
                span: unb
                    .get_element(element)
                    .and_then(|e| e.component_span(0))
                    .unwrap_or(unb.span),
                suggestion: Some("1"),
            }),
        }
    }
}

/// The refusal for a head segment that is not `UNB`.
fn not_unb(seg: &Segment<'_>) -> EdifactError {
    crate::envelope::classify_u_tag(seg.tag(), seg.span).unwrap_or_else(|| {
        EdifactError::MissingSegment {
            tag: "UNB".to_owned(),
            expected_position: "first segment of interchange".to_owned(),
        }
    })
}

/// Bytes read per `read` call while looking for the `UNB` terminator.
const CHUNK: usize = 4096;

/// The prologue a header read accepts before its `UNA`/`UNB`.
const PROLOGUE_CAP: usize = 4096;

/// Most bytes a header read consumes: prologue, `UNA` and one segment.
fn read_bound(config: &ReaderConfig) -> usize {
    PROLOGUE_CAP
        .saturating_add(9)
        .saturating_add(config.max_segment_bytes)
}

/// The end of the head segment in `buf` (just past its terminator).
///
/// `Ok(None)`: more input is needed. `complete`: `buf` is all the input there
/// is; its end is then the head's end, and the parse reports what is wrong.
fn locate(buf: &[u8], bound: usize, complete: bool) -> Result<Option<usize>, EdifactError> {
    let exceeded = || EdifactError::LimitExceeded {
        limit: "header_read",
        max: bound as u64,
        offset: buf.len(),
    };
    let (prologue, head) = crate::tokenizer::classify_head(buf);
    if prologue > PROLOGUE_CAP {
        return Err(exceeded());
    }
    let (start, ssa) = match head {
        crate::tokenizer::Head::Una(una) => (
            prologue + 9,
            crate::tokenizer::ServiceStringAdvice::from_una(una),
        ),
        crate::tokenizer::Head::TruncatedUna if !complete => return Ok(None),
        _ => (prologue, crate::tokenizer::ServiceStringAdvice::default()),
    };
    let found = buf.get(start..).and_then(|rest| {
        crate::parser::find_unescaped_term(rest, ssa.segment_term, ssa.release_char)
    });
    match found {
        Some(at) => Ok(Some(start + at + 1)),
        None if buf.len() >= bound => Err(exceeded()),
        None if complete => Ok(Some(buf.len())),
        None => Ok(None),
    }
}

/// Parse the head segment of `head` (prologue, optional `UNA`, one segment)
/// as the parse entry points do, and type it.
fn parse_head(head: &[u8], config: ReaderConfig) -> Result<InterchangeHeader, EdifactError> {
    let decoded = crate::charset::decode_for_parse(head)?;
    match crate::from_bytes_with_config(&decoded, config).next() {
        Some(Ok(segment)) => InterchangeHeader::parse(&segment),
        Some(Err(error)) => Err(error),
        None => Err(EdifactError::MissingSegment {
            tag: "UNB".to_owned(),
            expected_position: "first segment of interchange".to_owned(),
        }),
    }
}

/// The value at `(element, component)`, or `""`.
fn text(seg: &Segment<'_>, element: usize, component: usize) -> String {
    seg.component_str(element, component)
        .unwrap_or("")
        .to_owned()
}

/// The value at `(element, component)` when present and not empty.
fn optional(seg: &Segment<'_>, element: usize, component: usize) -> Option<String> {
    seg.component_str(element, component)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn opt(value: &Option<String>) -> String {
    value.clone().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(result: Result<impl std::fmt::Debug, EdifactError>) -> &'static str {
        result.expect_err("must fail").stable_code()
    }

    #[test]
    fn two_digit_29_february_needs_no_century() {
        assert!(PreparationDateTime::from_two_digit_year(0, 2, 29, 0, 0).is_ok());
        assert_eq!(
            code(PreparationDateTime::from_two_digit_year(1, 2, 29, 0, 0)),
            "E060"
        );
        // 1900 is no leap year: the resolved date does not exist.
        let d = PreparationDateTime::from_two_digit_year(0, 2, 29, 0, 0).unwrap();
        assert_eq!(code(d.with_century(19)), "E060");
        assert_eq!(d.with_century(20).unwrap().year(), Year::Full(2000));
    }

    #[test]
    fn hour_24_is_not_a_time() {
        assert_eq!(code(PreparationDateTime::new(2026, 1, 1, 24, 0)), "E060");
        assert_eq!(code(PreparationDateTime::new(2026, 1, 1, 23, 60)), "E060");
        assert!(PreparationDateTime::new(2026, 1, 1, 23, 59).is_ok());
    }

    #[test]
    fn with_century_resolves_only_two_digit_years() {
        let d = PreparationDateTime::from_two_digit_year(99, 1, 1, 0, 0).unwrap();
        assert_eq!(d.with_century(19).unwrap().year(), Year::Full(1999));
        assert_eq!(d.with_century(20).unwrap().year(), Year::Full(2099));
        let full = PreparationDateTime::new(1999, 1, 1, 0, 0).unwrap();
        assert_eq!(full.with_century(20).unwrap(), full);
    }

    #[test]
    fn version_4_refuses_a_two_digit_year() {
        let d = PreparationDateTime::from_two_digit_year(99, 1, 1, 0, 0).unwrap();
        assert_eq!(code(d.format_for(SyntaxVersion::V4)), "E060");
        assert_eq!(
            d.format_for(SyntaxVersion::V3).unwrap(),
            ["990101".to_owned(), "0000".to_owned()]
        );
        let full = PreparationDateTime::new(2099, 1, 1, 7, 5).unwrap();
        assert_eq!(
            full.format_for(SyntaxVersion::V4).unwrap(),
            ["20990101".to_owned(), "0705".to_owned()]
        );
        assert_eq!(full.format_for(SyntaxVersion::V1).unwrap()[0], "990101");
    }

    #[test]
    fn two_digit_and_full_years_are_never_equal() {
        let two = PreparationDateTime::from_two_digit_year(99, 1, 1, 0, 0).unwrap();
        let full = PreparationDateTime::new(2099, 1, 1, 0, 0).unwrap();
        assert_ne!(two, full);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_keeps_a_two_digit_year() {
        let two = PreparationDateTime::from_two_digit_year(99, 1, 1, 0, 0).unwrap();
        let json = serde_json::to_string(&two).unwrap();
        assert!(json.contains("TwoDigit"), "{json}");
        let back: PreparationDateTime = serde_json::from_str(&json).unwrap();
        assert_eq!(back, two);
        assert_eq!(back.year(), Year::TwoDigit(99));
        // Deserialization validates.
        let bad = json.replace("\"day\":1", "\"day\":32");
        assert!(serde_json::from_str::<PreparationDateTime>(&bad).is_err());
    }

    #[test]
    fn writing_a_v4_header_from_a_two_digit_year_is_refused() {
        let header = InterchangeHeader::new(
            Charset::UnoC,
            SyntaxVersion::V4,
            Party::new("S"),
            Party::new("R"),
            PreparationDateTime::from_two_digit_year(99, 1, 1, 0, 0).unwrap(),
            "1",
        );
        assert_eq!(code(header.to_segment()), "E060");
    }
}
