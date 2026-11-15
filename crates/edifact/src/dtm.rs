//! `DTM` date/time values: a codec for UN code list 2379.
//!
//! A `DTM` segment carries one composite, C507: a qualifier (DE 2005), a value
//! (DE 2380) and a format code (DE 2379) that says how to read the value. This
//! module decodes the value by its format code, writes it back byte for byte,
//! and compares values across formats.
//!
//! ```
//! use edifact::{Dtm, DtmRelation, SegmentSliceExt};
//!
//! let segments: Vec<_> = edifact::from_bytes(
//!     b"DTM+163:202610010000?+02:303'DTM+164:202609302300?+00:303'",
//! )
//! .collect::<Result<_, _>>()?;
//!
//! let start = Dtm::from_segment(segments.qualified("DTM", "163").unwrap())?;
//! let end = Dtm::from_segment(segments.qualified("DTM", "164").unwrap())?;
//!
//! assert_eq!(start.value.to_string(), "202610010000+02");
//! // 22:00 UTC is before 23:00 UTC, whatever the strings say.
//! assert_eq!(start.value.compare(&end.value), DtmRelation::Before);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Supported codes
//!
//! Layouts are those of UNCL 2379 (UNTDID D.01B; `104` from D.15B). Every other code is
//! [`DtmError::UnsupportedFormat`]; two-digit-year codes are not supported,
//! because the century would be a guess.
//!
//! | Code | Layout | Meaning | [`DtmFormat`] |
//! |---|---|---|---|
//! | `102` | `CCYYMMDD` | calendar date | `Ccyymmdd` |
//! | `104` | `MMWWMMWW` | weeks of months | `MmwwMmww` |
//! | `106` | `MMDD` | day of a month | `Mmdd` |
//! | `203` | `CCYYMMDDHHMM` | date and time, minutes | `Ccyymmddhhmm` |
//! | `204` | `CCYYMMDDHHMMSS` | date and time, seconds | `Ccyymmddhhmmss` |
//! | `303` | `CCYYMMDDHHMMZZZ` | `203` with time zone | `CcyymmddhhmmZzz` |
//! | `304` | `CCYYMMDDHHMMSSZZZ` | `204` with time zone | `CcyymmddhhmmssZzz` |
//! | `401` | `HHMM` | time | `Hhmm` |
//! | `501` | `HHMMHHMM` | time span | `HhmmHhmm` |
//! | `602` | `CCYY` | year | `Ccyy` |
//! | `610` | `CCYYMM` | month | `Ccyymm` |
//! | `718` | `CCYYMMDD-CCYYMMDD` | period of days | `CcyymmddCcyymmdd` |
//! | `719` | `CCYYMMDDHHMM-CCYYMMDDHHMM` | period of minutes | `CcyymmddhhmmCcyymmddhhmm` |
//! | `802` | count | months | `Months` |
//! | `803` | count | weeks | `Weeks` |
//! | `804` | count | days | `Days` |
//! | `805` | count | hours | `Hours` |
//!
//! # Conventions where UNCL 2379 is silent
//!
//! - `ZZZ` is a sign and two digits of whole hours (`+00`, `-05`), at most 14.
//!   `-00` is kept as written. Any other zone form is refused.
//! - Periods are written without the hyphen the layout name shows (`718`, `719`
//!   say so). `718` covers whole days, end day included; `719` and `501` are
//!   half-open, `[start, end)`.
//! - `104` weeks run `01`–`05`, with no year; the end week is included.
//!   A `104` value compares only with another `104` value: which days a week
//!   holds is the implementation guide's convention.
//! - Hour `24` is accepted with zero minutes and seconds, kept as written, and
//!   compares equal to `00` of the next day. Second `60` is refused.
//! - Years run from `0001` to `9999`. Counts are canonical `i32` decimals
//!   (`0`, `-5`; not `+1`, `01` or `-0`).
//!
//! The value is the unescaped one the parser hands out (`+00`, not `?+00`);
//! the writer escapes it again.

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use crate::validator::{MessagePositions, ValidationRuleContext, Validator};
use crate::{
    Element, FindingKind, OwnedSegment, Segment, ValidationIssue, ValidationReport,
    ValidationSeverity,
};

// ── DtmFormat ─────────────────────────────────────────────────────────────────

/// A supported UNCL 2379 format code (DE 2379).
///
/// ```
/// use edifact::DtmFormat;
///
/// let f: DtmFormat = "303".parse()?;
/// assert_eq!(f, DtmFormat::CcyymmddhhmmZzz);
/// assert_eq!(f.layout(), "CCYYMMDDHHMMZZZ");
/// assert!(f.is_zoned());
/// assert_eq!(f.to_string(), "303");
/// assert!(DtmFormat::from_code("101").is_err()); // two-digit year
/// # Ok::<(), edifact::DtmError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DtmFormat {
    /// `102` `CCYYMMDD`: calendar date.
    Ccyymmdd,
    /// `104` `MMWWMMWW`: period from a week of a month to a week of a month,
    /// written without the hyphen.
    MmwwMmww,
    /// `106` `MMDD`: day of a month.
    Mmdd,
    /// `203` `CCYYMMDDHHMM`: calendar date including time with minutes.
    Ccyymmddhhmm,
    /// `204` `CCYYMMDDHHMMSS`: calendar date including time with seconds.
    Ccyymmddhhmmss,
    /// `303` `CCYYMMDDHHMMZZZ`: `203` plus time zone.
    CcyymmddhhmmZzz,
    /// `304` `CCYYMMDDHHMMSSZZZ`: `204` plus time zone.
    CcyymmddhhmmssZzz,
    /// `401` `HHMM`: time without seconds.
    Hhmm,
    /// `501` `HHMMHHMM`: time span without seconds.
    HhmmHhmm,
    /// `602` `CCYY`: calendar year including century.
    Ccyy,
    /// `610` `CCYYMM`: month within a calendar year.
    Ccyymm,
    /// `718` `CCYYMMDD-CCYYMMDD`: period of days, written without the hyphen.
    CcyymmddCcyymmdd,
    /// `719` `CCYYMMDDHHMM-CCYYMMDDHHMM`: period of minutes, written without
    /// the hyphen.
    CcyymmddhhmmCcyymmddhhmm,
    /// `802`: a quantity of months.
    Months,
    /// `803`: a quantity of weeks.
    Weeks,
    /// `804`: a quantity of days.
    Days,
    /// `805`: a quantity of hours.
    Hours,
}

impl DtmFormat {
    const ALL: [Self; 17] = [
        Self::Ccyymmdd,
        Self::MmwwMmww,
        Self::Mmdd,
        Self::Ccyymmddhhmm,
        Self::Ccyymmddhhmmss,
        Self::CcyymmddhhmmZzz,
        Self::CcyymmddhhmmssZzz,
        Self::Hhmm,
        Self::HhmmHhmm,
        Self::Ccyy,
        Self::Ccyymm,
        Self::CcyymmddCcyymmdd,
        Self::CcyymmddhhmmCcyymmddhhmm,
        Self::Months,
        Self::Weeks,
        Self::Days,
        Self::Hours,
    ];

    /// The DE 2379 code, e.g. `"303"`.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Ccyymmdd => "102",
            Self::MmwwMmww => "104",
            Self::Mmdd => "106",
            Self::Ccyymmddhhmm => "203",
            Self::Ccyymmddhhmmss => "204",
            Self::CcyymmddhhmmZzz => "303",
            Self::CcyymmddhhmmssZzz => "304",
            Self::Hhmm => "401",
            Self::HhmmHhmm => "501",
            Self::Ccyy => "602",
            Self::Ccyymm => "610",
            Self::CcyymmddCcyymmdd => "718",
            Self::CcyymmddhhmmCcyymmddhhmm => "719",
            Self::Months => "802",
            Self::Weeks => "803",
            Self::Days => "804",
            Self::Hours => "805",
        }
    }

    /// The UNCL 2379 name of the layout, e.g. `"CCYYMMDDHHMMZZZ"`.
    #[must_use]
    pub const fn layout(self) -> &'static str {
        match self {
            Self::Ccyymmdd => "CCYYMMDD",
            Self::MmwwMmww => "MMWWMMWW",
            Self::Mmdd => "MMDD",
            Self::Ccyymmddhhmm => "CCYYMMDDHHMM",
            Self::Ccyymmddhhmmss => "CCYYMMDDHHMMSS",
            Self::CcyymmddhhmmZzz => "CCYYMMDDHHMMZZZ",
            Self::CcyymmddhhmmssZzz => "CCYYMMDDHHMMSSZZZ",
            Self::Hhmm => "HHMM",
            Self::HhmmHhmm => "HHMMHHMM",
            Self::Ccyy => "CCYY",
            Self::Ccyymm => "CCYYMM",
            Self::CcyymmddCcyymmdd => "CCYYMMDD-CCYYMMDD",
            Self::CcyymmddhhmmCcyymmddhhmm => "CCYYMMDDHHMM-CCYYMMDDHHMM",
            Self::Months => "Month",
            Self::Weeks => "Week",
            Self::Days => "Day",
            Self::Hours => "Hour",
        }
    }

    /// The format for DE 2379 `code`.
    ///
    /// # Errors
    ///
    /// [`DtmError::UnsupportedFormat`] for any other code.
    pub fn from_code(code: &str) -> Result<Self, DtmError> {
        Self::ALL
            .into_iter()
            .find(|f| f.code() == code)
            .ok_or_else(|| DtmError::UnsupportedFormat {
                code: code.to_owned(),
            })
    }

    /// Whether the layout carries a time zone (`303`, `304`).
    #[must_use]
    pub const fn is_zoned(self) -> bool {
        matches!(self, Self::CcyymmddhhmmZzz | Self::CcyymmddhhmmssZzz)
    }

    /// Whether the value is a period (`104`, `501`, `718`, `719`).
    #[must_use]
    pub const fn is_period(self) -> bool {
        matches!(
            self,
            Self::MmwwMmww
                | Self::HhmmHhmm
                | Self::CcyymmddCcyymmdd
                | Self::CcyymmddhhmmCcyymmddhhmm
        )
    }

    const fn is_count(self) -> bool {
        matches!(self, Self::Months | Self::Weeks | Self::Days | Self::Hours)
    }

    /// Byte template of a fixed-width layout: `d` digit, `s` zone sign.
    const fn template(self) -> &'static [u8] {
        match self {
            Self::Ccyymmdd | Self::MmwwMmww => b"dddddddd",
            Self::Mmdd | Self::Hhmm | Self::Ccyy => b"dddd",
            Self::Ccyymmddhhmm => b"dddddddddddd",
            Self::Ccyymmddhhmmss => b"dddddddddddddd",
            Self::CcyymmddhhmmZzz => b"ddddddddddddsdd",
            Self::CcyymmddhhmmssZzz => b"ddddddddddddddsdd",
            Self::HhmmHhmm => b"dddddddd",
            Self::Ccyymm => b"dddddd",
            Self::CcyymmddCcyymmdd => b"dddddddddddddddd",
            Self::CcyymmddhhmmCcyymmddhhmm => b"dddddddddddddddddddddddd",
            Self::Months | Self::Weeks | Self::Days | Self::Hours => b"",
        }
    }
}

impl fmt::Display for DtmFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl FromStr for DtmFormat {
    type Err = DtmError;

    fn from_str(code: &str) -> Result<Self, Self::Err> {
        Self::from_code(code)
    }
}

// ── DtmError ──────────────────────────────────────────────────────────────────

/// Why a `DTM` value or conversion was refused.
///
/// ```
/// use edifact::{DtmError, DtmFormat, DtmValue};
///
/// let err = DtmValue::parse("20180132", "102").unwrap_err();
/// assert_eq!(
///     err,
///     DtmError::InvalidValue { format: DtmFormat::Ccyymmdd, position: 6, reason: "day out of range" },
/// );
/// assert_eq!(err.component(), Some(1));
/// assert_eq!(
///     err.to_string(),
///     "value does not match format 102 (CCYYMMDD): day out of range at position 6",
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DtmError {
    /// The segment is not a `DTM`.
    #[error("segment {tag} is not DTM", tag = crate::error::escape_control(.tag))]
    NotDtm {
        /// The segment's tag.
        tag: String,
    },
    /// A C507 component is absent or empty: 0 qualifier, 1 value, 2 format code.
    #[error("DTM C507 component {component} is missing")]
    MissingComponent {
        /// Component index within C507.
        component: usize,
    },
    /// The format code is not one of the supported UNCL 2379 codes.
    #[error("format code {code:?} is not supported")]
    UnsupportedFormat {
        /// The code as written.
        code: String,
    },
    /// The value does not match its format's layout.
    #[error("value does not match format {format} ({layout}): {reason} at position {position}", layout = .format.layout())]
    InvalidValue {
        /// The declared format.
        format: DtmFormat,
        /// Character index of the offending field or character.
        position: usize,
        /// What is wrong, e.g. `"month out of range"`.
        reason: &'static str,
    },
    /// An offset that is not whole hours within ±14 h.
    #[error("offset of {seconds} s is not whole hours within ±14 h")]
    InvalidOffset {
        /// The offset in seconds, as given.
        seconds: i32,
    },
    /// The value cannot be converted to the requested form.
    #[error("format {format} cannot be converted to {target}")]
    NotConvertible {
        /// The value's format.
        format: DtmFormat,
        /// The requested target.
        target: &'static str,
    },
}

impl DtmError {
    /// The C507 component the error points at: 1 value, 2 format code.
    #[must_use]
    pub fn component(&self) -> Option<usize> {
        match self {
            Self::MissingComponent { component } => Some(*component),
            Self::UnsupportedFormat { .. } => Some(2),
            Self::InvalidValue { .. } => Some(1),
            Self::NotDtm { .. } | Self::InvalidOffset { .. } | Self::NotConvertible { .. } => None,
        }
    }
}

// ── building blocks ───────────────────────────────────────────────────────────

/// Date and time fields as written. Fields a layout lacks hold fixed defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
struct Stamp {
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
}

/// `ZZZ` as written: sign and whole hours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Zone {
    negative: bool,
    hours: u8,
}

impl Zone {
    fn offset_hours(self) -> i8 {
        // `hours` is at most 14.
        let h = self.hours as i8;
        if self.negative { -h } else { h }
    }
}

const fn is_leap(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

const fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 in the proleptic Gregorian calendar.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The value's bytes plus its format, for positioned errors.
struct Input<'a> {
    bytes: &'a [u8],
    format: DtmFormat,
}

impl Input<'_> {
    fn err(&self, position: usize, reason: &'static str) -> DtmError {
        DtmError::InvalidValue {
            format: self.format,
            position,
            reason,
        }
    }

    /// `n` digits at `at`; the template check has run, so they are digits.
    fn num(&self, at: usize, n: usize) -> u32 {
        self.bytes[at..at + n]
            .iter()
            .fold(0, |acc, b| acc * 10 + u32::from(b - b'0'))
    }

    fn year(&self, at: usize) -> Result<u16, DtmError> {
        match self.num(at, 4) {
            0 => Err(self.err(at, "year 0000 out of range")),
            // At most 9999.
            y => Ok(y as u16),
        }
    }

    fn month(&self, at: usize) -> Result<u8, DtmError> {
        match self.num(at, 2) {
            m @ 1..=12 => Ok(m as u8),
            _ => Err(self.err(at, "month out of range")),
        }
    }

    /// Week of a month, `01`–`05`.
    fn week(&self, at: usize) -> Result<u8, DtmError> {
        match self.num(at, 2) {
            w @ 1..=5 => Ok(w as u8),
            _ => Err(self.err(at, "week out of range")),
        }
    }

    /// `MMWW` at `at`, the week kept in the day field.
    fn month_week(&self, at: usize) -> Result<Stamp, DtmError> {
        Ok(Stamp {
            month: self.month(at)?,
            day: self.week(at + 2)?,
            ..Stamp::default()
        })
    }

    fn day(&self, at: usize, year: u32, month: u8) -> Result<u8, DtmError> {
        match self.num(at, 2) {
            d if d >= 1 && d <= days_in_month(year, u32::from(month)) => Ok(d as u8),
            _ => Err(self.err(at, "day out of range")),
        }
    }

    /// `CCYYMMDD` at `at`.
    fn date(&self, at: usize) -> Result<Stamp, DtmError> {
        let year = self.year(at)?;
        let month = self.month(at + 4)?;
        let day = self.day(at + 6, u32::from(year), month)?;
        Ok(Stamp {
            year,
            month,
            day,
            ..Stamp::default()
        })
    }

    /// `HHMM` at `at`, plus `SS` when `seconds`. Hour 24 only as `2400[00]`.
    fn time(&self, at: usize, seconds: bool, mut stamp: Stamp) -> Result<Stamp, DtmError> {
        let hour = self.num(at, 2);
        let minute = self.num(at + 2, 2);
        let second = if seconds { self.num(at + 4, 2) } else { 0 };
        if hour > 24 {
            return Err(self.err(at, "hour out of range"));
        }
        if minute > 59 {
            return Err(self.err(at + 2, "minute out of range"));
        }
        if second > 59 {
            return Err(self.err(at + 4, "second out of range"));
        }
        if hour == 24 && minute != 0 {
            return Err(self.err(at + 2, "hour 24 needs minute 00"));
        }
        if hour == 24 && second != 0 {
            return Err(self.err(at + 4, "hour 24 needs second 00"));
        }
        // Each is below 60, or 24 for the hour.
        stamp.hour = hour as u8;
        stamp.minute = minute as u8;
        stamp.second = second as u8;
        Ok(stamp)
    }

    /// `ZZZ` at `at`: sign and two digits, at most 14.
    fn zone(&self, at: usize) -> Result<Zone, DtmError> {
        let hours = self.num(at + 1, 2);
        if hours > 14 {
            return Err(self.err(at + 1, "zone hours out of range"));
        }
        Ok(Zone {
            negative: self.bytes[at] == b'-',
            hours: hours as u8,
        })
    }

    /// Check width and the digit/sign template before any field is read.
    fn check_template(&self) -> Result<(), DtmError> {
        let template = self.format.template();
        let bytes = self.bytes;
        if self.format.is_period()
            && bytes.len() == template.len() + 1
            && bytes[template.len() / 2] == b'-'
        {
            return Err(self.err(
                template.len() / 2,
                "hyphen: UNCL 2379 writes periods without one",
            ));
        }
        for (i, (&b, &t)) in bytes.iter().zip(template).enumerate() {
            let ok = match t {
                b's' => b == b'+' || b == b'-',
                _ => b.is_ascii_digit(),
            };
            if !ok {
                let reason = if t == b's' {
                    "expected zone sign + or -"
                } else {
                    "expected digit"
                };
                return Err(self.err(i, reason));
            }
        }
        if bytes.len() != template.len() {
            return Err(self.err(
                bytes.len().min(template.len()),
                "wrong length for the layout",
            ));
        }
        Ok(())
    }

    /// `-?(0|[1-9][0-9]*)` within `i32`.
    fn count(&self) -> Result<i32, DtmError> {
        let bytes = self.bytes;
        let (negative, digits) = match bytes.split_first() {
            Some((b'-', rest)) => (true, rest),
            _ => (false, bytes),
        };
        let first = usize::from(negative);
        if digits.is_empty() {
            return Err(self.err(first, "expected digit"));
        }
        if let Some(i) = digits.iter().position(|b| !b.is_ascii_digit()) {
            return Err(self.err(first + i, "expected digit"));
        }
        if digits[0] == b'0' && digits.len() > 1 {
            return Err(self.err(first, "leading zero"));
        }
        if negative && digits == b"0" {
            return Err(self.err(0, "negative zero"));
        }
        let mut n: i64 = 0;
        for &b in digits {
            n = n
                .checked_mul(10)
                .and_then(|n| n.checked_add(i64::from(b - b'0')))
                .filter(|&n| n <= i64::from(i32::MAX) + 1)
                .ok_or_else(|| self.err(0, "count out of i32 range"))?;
        }
        let n = if negative { -n } else { n };
        i32::try_from(n).map_err(|_| self.err(0, "count out of i32 range"))
    }
}

// ── DtmValue ──────────────────────────────────────────────────────────────────

/// A decoded DE 2380 value with its format.
///
/// Fields are kept as written, so [`Display`][fmt::Display] gives back the
/// exact input. `==` is therefore "same as written": `…2400` is not `==` to the
/// next day's `…0000`, and `-00` is not `==` to `+00`. Use
/// [`compare`][Self::compare] for time semantics.
///
/// ```
/// use edifact::DtmValue;
///
/// let v = DtmValue::parse("202610011200+00", "303")?;
/// assert_eq!((v.year(), v.month(), v.day()), (Some(2026), Some(10), Some(1)));
/// assert_eq!((v.hour(), v.minute(), v.second()), (Some(12), Some(0), None));
/// assert_eq!(v.offset_hours(), Some(0));
/// assert_eq!(v.code(), "303");
/// assert_eq!(v.to_string(), "202610011200+00");
/// # Ok::<(), edifact::DtmError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DtmValue {
    format: DtmFormat,
    start: Stamp,
    /// Second half of a period.
    end: Stamp,
    zone: Option<Zone>,
    count: i32,
}

impl DtmValue {
    /// Decode `value` by DE 2379 `code`.
    ///
    /// ```
    /// use edifact::{DtmError, DtmValue};
    ///
    /// assert_eq!(DtmValue::parse("20261001", "102")?.to_string(), "20261001");
    /// assert_eq!(
    ///     DtmValue::parse("202610010000", "999"),
    ///     Err(DtmError::UnsupportedFormat { code: "999".into() }),
    /// );
    /// # Ok::<(), DtmError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`DtmError::UnsupportedFormat`] for an unsupported code, otherwise as
    /// [`parse_as`][Self::parse_as].
    pub fn parse(value: &str, code: &str) -> Result<Self, DtmError> {
        Self::parse_as(value, DtmFormat::from_code(code)?)
    }

    /// Decode `value` in `format`.
    ///
    /// ```
    /// use edifact::{DtmFormat, DtmValue};
    ///
    /// let p = DtmValue::parse_as("201801010500201801020500", DtmFormat::CcyymmddhhmmCcyymmddhhmm)?;
    /// assert_eq!(p.start().map(|s| s.to_string()).as_deref(), Some("201801010500"));
    /// assert!(DtmValue::parse_as("2026022", DtmFormat::Ccyymmdd).is_err());
    /// assert!(DtmValue::parse_as("2018ü101", DtmFormat::Ccyymmdd).is_err());
    /// # Ok::<(), edifact::DtmError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`DtmError::InvalidValue`] with the character position when the value is
    /// empty, not ASCII, of the wrong width, or has a field out of range.
    pub fn parse_as(value: &str, format: DtmFormat) -> Result<Self, DtmError> {
        let input = Input {
            bytes: value.as_bytes(),
            format,
        };
        if value.is_empty() {
            return Err(input.err(0, "empty"));
        }
        if let Some(position) = value.chars().position(|c| !c.is_ascii()) {
            return Err(input.err(position, "not ASCII"));
        }
        let mut out = Self {
            format,
            start: Stamp::default(),
            end: Stamp::default(),
            zone: None,
            count: 0,
        };
        if format.is_count() {
            out.count = input.count()?;
            return Ok(out);
        }
        input.check_template()?;
        let leap_year = 2000;
        match format {
            DtmFormat::Ccyymmdd => out.start = input.date(0)?,
            DtmFormat::MmwwMmww => {
                out.start = input.month_week(0)?;
                out.end = input.month_week(4)?;
            }
            DtmFormat::Mmdd => {
                let month = input.month(0)?;
                let day = input.day(2, leap_year, month)?;
                out.start = Stamp {
                    month,
                    day,
                    ..Stamp::default()
                };
            }
            DtmFormat::Ccyymmddhhmm => out.start = input.time(8, false, input.date(0)?)?,
            DtmFormat::Ccyymmddhhmmss => out.start = input.time(8, true, input.date(0)?)?,
            DtmFormat::CcyymmddhhmmZzz => {
                out.start = input.time(8, false, input.date(0)?)?;
                out.zone = Some(input.zone(12)?);
            }
            DtmFormat::CcyymmddhhmmssZzz => {
                out.start = input.time(8, true, input.date(0)?)?;
                out.zone = Some(input.zone(14)?);
            }
            DtmFormat::Hhmm => out.start = input.time(0, false, Stamp::default())?,
            DtmFormat::HhmmHhmm => {
                out.start = input.time(0, false, Stamp::default())?;
                out.end = input.time(4, false, Stamp::default())?;
            }
            DtmFormat::Ccyy => {
                out.start = Stamp {
                    year: input.year(0)?,
                    ..Stamp::default()
                };
            }
            DtmFormat::Ccyymm => {
                out.start = Stamp {
                    year: input.year(0)?,
                    month: input.month(4)?,
                    ..Stamp::default()
                };
            }
            DtmFormat::CcyymmddCcyymmdd => {
                out.start = input.date(0)?;
                out.end = input.date(8)?;
            }
            DtmFormat::CcyymmddhhmmCcyymmddhhmm => {
                out.start = input.time(8, false, input.date(0)?)?;
                out.end = input.time(20, false, input.date(12)?)?;
            }
            DtmFormat::Months | DtmFormat::Weeks | DtmFormat::Days | DtmFormat::Hours => {}
        }
        Ok(out)
    }

    /// The value's format.
    #[must_use]
    pub fn format(&self) -> DtmFormat {
        self.format
    }

    /// The value's DE 2379 code, e.g. `"303"`.
    #[must_use]
    pub fn code(&self) -> &'static str {
        self.format.code()
    }

    fn has_year(&self) -> bool {
        use DtmFormat as F;
        matches!(
            self.format,
            F::Ccyymmdd
                | F::Ccyymmddhhmm
                | F::Ccyymmddhhmmss
                | F::CcyymmddhhmmZzz
                | F::CcyymmddhhmmssZzz
                | F::Ccyy
                | F::Ccyymm
                | F::CcyymmddCcyymmdd
                | F::CcyymmddhhmmCcyymmddhhmm
        )
    }

    fn has_day(&self) -> bool {
        self.has_year() && !matches!(self.format, DtmFormat::Ccyy | DtmFormat::Ccyymm)
            || self.format == DtmFormat::Mmdd
    }

    fn has_time(&self) -> bool {
        use DtmFormat as F;
        matches!(
            self.format,
            F::Ccyymmddhhmm
                | F::Ccyymmddhhmmss
                | F::CcyymmddhhmmZzz
                | F::CcyymmddhhmmssZzz
                | F::Hhmm
                | F::HhmmHhmm
                | F::CcyymmddhhmmCcyymmddhhmm
        )
    }

    /// The year; a period's start year. `None` if the layout has none.
    #[must_use]
    pub fn year(&self) -> Option<u16> {
        self.has_year().then_some(self.start.year)
    }

    /// The month (1–12). `None` if the layout has none.
    #[must_use]
    pub fn month(&self) -> Option<u8> {
        (self.has_day() || self.format == DtmFormat::Ccyymm).then_some(self.start.month)
    }

    /// The day of the month. `None` if the layout has none.
    #[must_use]
    pub fn day(&self) -> Option<u8> {
        self.has_day().then_some(self.start.day)
    }

    /// The hour as written (0–24). `None` if the layout has none.
    #[must_use]
    pub fn hour(&self) -> Option<u8> {
        self.has_time().then_some(self.start.hour)
    }

    /// The minute. `None` if the layout has none.
    #[must_use]
    pub fn minute(&self) -> Option<u8> {
        self.has_time().then_some(self.start.minute)
    }

    /// The second (`204`, `304`). `None` otherwise.
    #[must_use]
    pub fn second(&self) -> Option<u8> {
        matches!(
            self.format,
            DtmFormat::Ccyymmddhhmmss | DtmFormat::CcyymmddhhmmssZzz
        )
        .then_some(self.start.second)
    }

    /// The zone offset in hours (`303`, `304`); `-00` reads as `0`. `None` for
    /// floating values.
    #[must_use]
    pub fn offset_hours(&self) -> Option<i8> {
        self.zone.map(Zone::offset_hours)
    }

    /// The count (`802`–`805`). `None` otherwise.
    ///
    /// ```
    /// use edifact::DtmValue;
    ///
    /// assert_eq!(DtmValue::parse("-3", "804")?.count(), Some(-3));
    /// assert!(DtmValue::parse("+1", "804").is_err());
    /// assert!(DtmValue::parse("01", "804").is_err());
    /// # Ok::<(), edifact::DtmError>(())
    /// ```
    #[must_use]
    pub fn count(&self) -> Option<i32> {
        self.format.is_count().then_some(self.count)
    }

    fn half(&self, stamp: Stamp) -> Option<Self> {
        let format = match self.format {
            DtmFormat::CcyymmddCcyymmdd => DtmFormat::Ccyymmdd,
            DtmFormat::CcyymmddhhmmCcyymmddhhmm => DtmFormat::Ccyymmddhhmm,
            DtmFormat::HhmmHhmm => DtmFormat::Hhmm,
            _ => return None,
        };
        Some(Self {
            format,
            start: stamp,
            end: Stamp::default(),
            zone: None,
            count: 0,
        })
    }

    /// A period's first half (`718` → `102`, `719` → `203`, `501` → `401`).
    ///
    /// ```
    /// use edifact::DtmValue;
    ///
    /// let p = DtmValue::parse("2026100120261031", "718")?;
    /// assert_eq!(p.start().unwrap().to_string(), "20261001");
    /// assert_eq!(p.end().unwrap().to_string(), "20261031");
    /// assert!(p.is_forward());
    /// assert!(!DtmValue::parse("2026103120261001", "718")?.is_forward());
    /// # Ok::<(), edifact::DtmError>(())
    /// ```
    #[must_use]
    pub fn start(&self) -> Option<Self> {
        self.half(self.start)
    }

    /// A period's second half. See [`start`][Self::start].
    #[must_use]
    pub fn end(&self) -> Option<Self> {
        self.half(self.end)
    }

    /// Whether a period is non-empty and not inverted: `719`/`501` need
    /// start < end, `718` start ≤ end (the end day is included). `true` for
    /// every value that is not a period.
    #[must_use]
    pub fn is_forward(&self) -> bool {
        if !self.format.is_period() {
            return true;
        }
        self.extent().is_some_and(|(_, iv)| iv.lo < iv.hi)
    }

    /// The same wall clock with zone `hours`: `203` → `303`, `204` → `304`.
    ///
    /// ```
    /// use edifact::DtmValue;
    ///
    /// let v = DtmValue::parse("202610011200", "203")?.with_offset(0)?;
    /// assert_eq!((v.code(), v.to_string().as_str()), ("303", "202610011200+00"));
    /// # Ok::<(), edifact::DtmError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`DtmError::NotConvertible`] for other formats,
    /// [`DtmError::InvalidOffset`] when `hours` is beyond ±14.
    pub fn with_offset(self, hours: i8) -> Result<Self, DtmError> {
        let format = match self.format {
            DtmFormat::Ccyymmddhhmm => DtmFormat::CcyymmddhhmmZzz,
            DtmFormat::Ccyymmddhhmmss => DtmFormat::CcyymmddhhmmssZzz,
            format => {
                return Err(DtmError::NotConvertible {
                    format,
                    target: "303/304",
                });
            }
        };
        let zone = zone_for(hours).ok_or(DtmError::InvalidOffset {
            seconds: i32::from(hours) * 3600,
        })?;
        Ok(Self {
            format,
            zone: Some(zone),
            ..self
        })
    }

    // ── comparison ────────────────────────────────────────────────────────────

    /// The interval this value denotes, in seconds on its timeline. `None` for
    /// counts.
    fn extent(&self) -> Option<(Timeline, Interval)> {
        use DtmFormat as F;
        let s = self.start;
        let iv = match self.format {
            F::Ccyymmdd => Interval::span(seconds(s)?, DAY)?,
            F::Ccyymmddhhmm | F::CcyymmddhhmmZzz => Interval::span(seconds(s)?, 60)?,
            F::Ccyymmddhhmmss | F::CcyymmddhhmmssZzz => Interval::span(seconds(s)?, 1)?,
            F::Ccyy => Interval {
                lo: day_start(s.year, 1, 1)?,
                hi: day_start(s.year.checked_add(1)?, 1, 1)?,
            },
            F::Ccyymm => {
                let (ny, nm) = if s.month == 12 {
                    (s.year.checked_add(1)?, 1)
                } else {
                    (s.year, s.month + 1)
                };
                Interval {
                    lo: day_start(s.year, s.month, 1)?,
                    hi: day_start(ny, nm, 1)?,
                }
            }
            F::CcyymmddCcyymmdd => Interval {
                lo: seconds(s)?,
                hi: seconds(self.end)?.checked_add(DAY)?,
            },
            F::CcyymmddhhmmCcyymmddhhmm => Interval {
                lo: seconds(s)?,
                hi: seconds(self.end)?,
            },
            F::MmwwMmww => {
                let week = |s: Stamp| (i64::from(s.month) - 1) * 5 + i64::from(s.day) - 1;
                let iv = Interval {
                    lo: week(s),
                    hi: week(self.end) + 1,
                };
                return Some((Timeline::MonthWeek, iv));
            }
            F::Mmdd => {
                let lo = day_start(2000, s.month, s.day)?.checked_sub(day_start(2000, 1, 1)?)?;
                return Some((Timeline::DayOfYear, Interval::span(lo, DAY)?));
            }
            F::Hhmm => return Some((Timeline::TimeOfDay, Interval::span(time_of_day(s), 60)?)),
            F::HhmmHhmm => {
                let iv = Interval {
                    lo: time_of_day(s),
                    hi: time_of_day(self.end),
                };
                return Some((Timeline::TimeOfDay, iv));
            }
            F::Months | F::Weeks | F::Days | F::Hours => return None,
        };
        let zone = self.zone.map(|z| i64::from(z.offset_hours()) * 3600);
        match zone {
            // Local to UTC: 12:00+02 is 10:00 UTC.
            Some(offset) => Some((Timeline::Utc, iv.shift(-offset)?)),
            None => Some((Timeline::Floating, iv)),
        }
    }

    /// How this value lies in time relative to `other`.
    ///
    /// Each value denotes an interval at its precision (`102` a day, `203` a
    /// minute, `610` a month, a period its extent):
    ///
    /// | Result | When |
    /// |---|---|
    /// | `Before` | this interval ends at or before `other` starts |
    /// | `After` | the mirror |
    /// | `Equal` | identical intervals |
    /// | `Overlapping` | anything else |
    /// | `Indeterminate` | a count, an inverted or empty period, different timelines (`104`, `106`, `401`/`501`, absolute), or floating vs zoned where the answer depends on the offset |
    ///
    /// A floating value (no zone) against a zoned one is tried at −14 h and
    /// +14 h; only if both give `Before`, or both `After`, is that the answer.
    /// [`compare_assuming`][Self::compare_assuming] supplies the offset.
    ///
    /// Accept a period only on `Before`; every other relation is a rejection:
    ///
    /// ```
    /// use edifact::{DtmRelation, DtmValue};
    ///
    /// fn period_is_ordered(start: &DtmValue, end: &DtmValue) -> bool {
    ///     match start.compare(end) {
    ///         DtmRelation::Before => true,
    ///         // After, Equal, Overlapping, Indeterminate
    ///         _ => false,
    ///     }
    /// }
    ///
    /// let p = |v, c| DtmValue::parse(v, c).unwrap();
    /// // 22:00 UTC before 23:00 UTC; a string compare says otherwise.
    /// assert!(period_is_ordered(&p("202610010000+02", "303"), &p("202609302300+00", "303")));
    /// // A minute inside a day overlaps it.
    /// assert!(!period_is_ordered(&p("202610010000", "203"), &p("20261001", "102")));
    /// // Floating vs zoned an hour apart: depends on the zone, so refused.
    /// let (start, end) = (p("202610011200", "203"), p("202610011100+00", "303"));
    /// assert_eq!(start.compare(&end), DtmRelation::Indeterminate);
    /// assert!(!period_is_ordered(&start, &end));
    /// // Months apart: every offset agrees.
    /// assert_eq!(p("20260115", "102").compare(&p("202610011200+00", "303")), DtmRelation::Before);
    /// ```
    #[must_use = "a relation only protects when it is checked"]
    pub fn compare(&self, other: &Self) -> DtmRelation {
        self.relate(other, None)
    }

    /// [`compare`][Self::compare], reading floating values at UTC offset
    /// `offset_hours`. Zoned values keep their own zone. An offset beyond ±14
    /// gives `Indeterminate`.
    ///
    /// ```
    /// use edifact::{DtmRelation, DtmValue};
    ///
    /// let floating = DtmValue::parse("202610011200", "203")?;
    /// let zoned = DtmValue::parse("202610011200+00", "303")?;
    /// assert_eq!(floating.compare(&zoned), DtmRelation::Indeterminate);
    /// assert_eq!(floating.compare_assuming(&zoned, 0), DtmRelation::Equal);
    /// assert_eq!(floating.compare_assuming(&zoned, 2), DtmRelation::Before);
    /// # Ok::<(), edifact::DtmError>(())
    /// ```
    #[must_use = "a relation only protects when it is checked"]
    pub fn compare_assuming(&self, other: &Self, offset_hours: i8) -> DtmRelation {
        if offset_hours.unsigned_abs() > MAX_ZONE_HOURS {
            return DtmRelation::Indeterminate;
        }
        self.relate(other, Some(offset_hours))
    }

    fn relate(&self, other: &Self, offset: Option<i8>) -> DtmRelation {
        let (Some((ta, a)), Some((tb, b))) = (self.extent(), other.extent()) else {
            return DtmRelation::Indeterminate;
        };
        if a.lo >= a.hi || b.lo >= b.hi {
            // Inverted or empty period.
            return DtmRelation::Indeterminate;
        }
        // Floating local time at `o` hours east is `o` hours ahead of UTC.
        let at = |iv: Interval, o: i8| iv.shift(-i64::from(o) * 3600);
        let pair = |o: i8| -> Option<DtmRelation> {
            match (ta, tb) {
                (Timeline::Floating, Timeline::Utc) => Some(relation(at(a, o)?, b)),
                (Timeline::Utc, Timeline::Floating) => Some(relation(a, at(b, o)?)),
                _ => None,
            }
        };
        match (ta, tb) {
            (x, y) if x == y => relation(a, b),
            (Timeline::Floating, Timeline::Utc) | (Timeline::Utc, Timeline::Floating) => {
                let decided = match offset {
                    Some(o) => pair(o),
                    None => match (pair(-MAX_ZONE), pair(MAX_ZONE)) {
                        (Some(r @ (DtmRelation::Before | DtmRelation::After)), Some(s))
                            if r == s =>
                        {
                            Some(r)
                        }
                        _ => None,
                    },
                };
                decided.unwrap_or(DtmRelation::Indeterminate)
            }
            _ => DtmRelation::Indeterminate,
        }
    }
}

const DAY: i64 = 86_400;
const MAX_ZONE_HOURS: u8 = 14;
const MAX_ZONE: i8 = 14;

fn zone_for(hours: i8) -> Option<Zone> {
    (hours.unsigned_abs() <= MAX_ZONE_HOURS).then_some(Zone {
        negative: hours < 0,
        hours: hours.unsigned_abs(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Timeline {
    /// Local time without a zone.
    Floating,
    /// Zoned values, moved to UTC.
    Utc,
    /// `106`: day of a (leap) year.
    DayOfYear,
    /// `104`: week of a month, five weeks to a month.
    MonthWeek,
    /// `401`, `501`: seconds since midnight.
    TimeOfDay,
}

/// Half-open `[lo, hi)` in seconds. Years 1–10000 keep every bound near
/// ±3.2e11, far inside `i64`; the arithmetic is checked all the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Interval {
    lo: i64,
    hi: i64,
}

impl Interval {
    fn span(lo: i64, len: i64) -> Option<Self> {
        Some(Self {
            lo,
            hi: lo.checked_add(len)?,
        })
    }

    fn shift(self, by: i64) -> Option<Self> {
        Some(Self {
            lo: self.lo.checked_add(by)?,
            hi: self.hi.checked_add(by)?,
        })
    }
}

fn relation(a: Interval, b: Interval) -> DtmRelation {
    if a == b {
        DtmRelation::Equal
    } else if a.hi <= b.lo {
        DtmRelation::Before
    } else if b.hi <= a.lo {
        DtmRelation::After
    } else {
        DtmRelation::Overlapping
    }
}

fn day_start(year: u16, month: u8, day: u8) -> Option<i64> {
    days_from_civil(i64::from(year), i64::from(month), i64::from(day)).checked_mul(DAY)
}

/// Seconds of a stamp; hour 24 lands on the next day's midnight.
fn seconds(s: Stamp) -> Option<i64> {
    day_start(s.year, s.month, s.day)?.checked_add(time_of_day(s))
}

fn time_of_day(s: Stamp) -> i64 {
    i64::from(s.hour) * 3600 + i64::from(s.minute) * 60 + i64::from(s.second)
}

impl fmt::Display for DtmValue {
    /// The DE 2380 value exactly as written, unescaped.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use DtmFormat as F;
        let date = |f: &mut fmt::Formatter<'_>, s: Stamp| {
            write!(f, "{:04}{:02}{:02}", s.year, s.month, s.day)
        };
        let hm = |f: &mut fmt::Formatter<'_>, s: Stamp| write!(f, "{:02}{:02}", s.hour, s.minute);
        let s = self.start;
        match self.format {
            F::Ccyymmdd => date(f, s)?,
            F::MmwwMmww => write!(
                f,
                "{:02}{:02}{:02}{:02}",
                s.month, s.day, self.end.month, self.end.day
            )?,
            F::Mmdd => write!(f, "{:02}{:02}", s.month, s.day)?,
            F::Ccyymmddhhmm | F::CcyymmddhhmmZzz => {
                date(f, s)?;
                hm(f, s)?;
            }
            F::Ccyymmddhhmmss | F::CcyymmddhhmmssZzz => {
                date(f, s)?;
                hm(f, s)?;
                write!(f, "{:02}", s.second)?;
            }
            F::Hhmm => hm(f, s)?,
            F::HhmmHhmm => {
                hm(f, s)?;
                hm(f, self.end)?;
            }
            F::Ccyy => write!(f, "{:04}", s.year)?,
            F::Ccyymm => write!(f, "{:04}{:02}", s.year, s.month)?,
            F::CcyymmddCcyymmdd => {
                date(f, s)?;
                date(f, self.end)?;
            }
            F::CcyymmddhhmmCcyymmddhhmm => {
                date(f, s)?;
                hm(f, s)?;
                date(f, self.end)?;
                hm(f, self.end)?;
            }
            F::Months | F::Weeks | F::Days | F::Hours => write!(f, "{}", self.count)?,
        }
        if let Some(z) = self.zone {
            let sign = if z.negative { '-' } else { '+' };
            write!(f, "{sign}{:02}", z.hours)?;
        }
        Ok(())
    }
}

// ── DtmRelation ───────────────────────────────────────────────────────────────

/// How two [`DtmValue`]s lie in time. See [`DtmValue::compare`].
///
/// Only `Before` means "ordered". `Indeterminate` means the codec cannot
/// tell; treat it as a refusal, never as a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[must_use]
#[non_exhaustive]
pub enum DtmRelation {
    /// Ends at or before the other starts.
    Before,
    /// Starts at or after the other ends.
    After,
    /// The same interval.
    Equal,
    /// The intervals share time but differ.
    Overlapping,
    /// Not decidable: see [`DtmValue::compare`].
    Indeterminate,
}

impl DtmRelation {
    /// The relation seen from the other side: `Before` ↔ `After`.
    ///
    /// ```
    /// use edifact::DtmRelation;
    ///
    /// assert_eq!(DtmRelation::Before.reverse(), DtmRelation::After);
    /// assert_eq!(DtmRelation::Overlapping.reverse(), DtmRelation::Overlapping);
    /// ```
    pub const fn reverse(self) -> Self {
        match self {
            Self::Before => Self::After,
            Self::After => Self::Before,
            other => other,
        }
    }
}

// ── Dtm (C507) ────────────────────────────────────────────────────────────────

/// One `DTM` C507: qualifier (DE 2005) and decoded value.
///
/// Read:
///
/// ```
/// use edifact::{Dtm, SegmentSliceExt};
///
/// let segments: Vec<_> = edifact::from_bytes(b"DTM+137:202610011200?+00:303'")
///     .collect::<Result<_, _>>()?;
/// let dtm = Dtm::from_segment(segments.qualified("DTM", "137").unwrap())?;
/// assert_eq!(dtm.qualifier, "137");
/// assert_eq!(dtm.value.offset_hours(), Some(0));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// Write: the writer escapes `+` as `?+`.
///
/// ```
/// use edifact::{Dtm, DtmValue, MessageBuilder, MessageIdentifier, Writer};
///
/// let stamp = DtmValue::parse("202610011200", "203")?.with_offset(0)?;
/// let dtm = Dtm::new("137", stamp);
///
/// let mut w = Writer::new(Vec::new());
/// w.write_composites("DTM", &[dtm.components()])?;
/// assert_eq!(w.finish()?, b"DTM+137:202610011200?+00:303'");
///
/// let mut msg = MessageBuilder::new(MessageIdentifier::new("1", "UTILMD", "D", "11A", "UN"))?;
/// msg.write_segment(&dtm.to_segment())?;
/// let segments = msg.finish()?;
/// assert_eq!(segments[1].component_str(0, 1), Some("202610011200+00"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Dtm<'a> {
    /// DE 2005, e.g. `"137"`.
    pub qualifier: Cow<'a, str>,
    /// DE 2380 decoded by DE 2379.
    pub value: DtmValue,
}

impl<'a> Dtm<'a> {
    /// A C507 from a qualifier and a value.
    pub fn new(qualifier: impl Into<Cow<'a, str>>, value: DtmValue) -> Self {
        Self {
            qualifier: qualifier.into(),
            value,
        }
    }

    /// Decode element 0 of a `DTM` segment.
    ///
    /// # Errors
    ///
    /// [`DtmError::NotDtm`] for another tag, [`DtmError::MissingComponent`]
    /// for an absent or empty qualifier, value or code, otherwise as
    /// [`DtmValue::parse`]. [`DtmError::component`] names the C507 component.
    pub fn from_segment(segment: &'a Segment<'_>) -> Result<Self, DtmError> {
        if segment.tag != "DTM" {
            return Err(DtmError::NotDtm {
                tag: segment.tag().to_owned(),
            });
        }
        let component = |n| {
            segment
                .component_str(0, n)
                .filter(|v| !v.is_empty())
                .ok_or(DtmError::MissingComponent { component: n })
        };
        let qualifier = component(0)?;
        let value = component(1)?;
        let code = component(2)?;
        Ok(Self::new(qualifier, DtmValue::parse(value, code)?))
    }

    /// `[qualifier, value, code]`, for
    /// [`Writer::write_composites`][crate::Writer::write_composites] or
    /// [`MessageBuilder::write_composites`][crate::MessageBuilder::write_composites].
    #[must_use]
    pub fn components(&self) -> [Cow<'_, str>; 3] {
        [
            Cow::Borrowed(&self.qualifier),
            Cow::Owned(self.value.to_string()),
            Cow::Borrowed(self.value.code()),
        ]
    }

    /// An owned `DTM` segment with this C507.
    #[must_use]
    pub fn to_segment(&self) -> OwnedSegment {
        let [q, v, c] = self.components();
        let element = Element::of(&[q.into_owned(), v.into_owned(), c.into_owned()]);
        Segment::new("DTM", vec![element])
    }
}

// ── DtmValidator ──────────────────────────────────────────────────────────────

/// Reports `DTM` values that contradict their format code.
///
/// | Finding | Severity | Code | Component |
/// |---|---|---|---|
/// | value does not match a supported code | Error | `E027` (CONTRL `12`) | 1 |
/// | unsupported code: the value was not checked | Warning | — | 2 |
///
/// Kind [`FindingKind::Representation`], rule id `DTM-2379`. A missing value
/// or code gives no finding. Which qualifier needs which format is the
/// guide's rule, not this validator's. A gate that admits only checked values
/// must also refuse the unsupported-code warning.
///
/// ```
/// use edifact::{DtmValidator, FindingKind, ValidationContext, ValidationLayer};
///
/// let segments: Vec<_> = edifact::from_bytes(b"DTM+137:20260701:303'DTM+2:x:999'")
///     .collect::<Result<_, _>>()?;
/// let report = ValidationContext::builder()
///     .with_validator(ValidationLayer::CodeList, DtmValidator)
///     .build()
///     .validate(&segments);
///
/// let error = &report.errors()[0];
/// assert_eq!(error.error_code(), Some("E027"));
/// assert_eq!(error.kind(), Some(FindingKind::Representation));
/// assert_eq!(error.component_index, Some(1));
/// assert_eq!(report.warnings()[0].component_index, Some(2));
/// # Ok::<(), edifact::EdifactError>(())
/// ```
#[derive(Debug, Default, Clone, Copy)]
pub struct DtmValidator;

impl Validator for DtmValidator {
    fn validate_batch(
        &self,
        segments: &[Segment<'_>],
        report: &mut ValidationReport,
        _context: &ValidationRuleContext<'_>,
    ) {
        let positions = MessagePositions::new(segments);
        for (index, segment) in segments.iter().enumerate() {
            if segment.tag != "DTM" {
                continue;
            }
            let err = match Dtm::from_segment(segment) {
                Err(err) => err,
                Ok(_) => continue,
            };
            let qualifier = segment.component_str(0, 0).unwrap_or("");
            let value = segment.component_str(0, 1).unwrap_or("");
            let (severity, component, code, suggestion) = match &err {
                DtmError::InvalidValue { format, .. } => (
                    ValidationSeverity::Error,
                    1,
                    Some("E027"),
                    format!("write the value as {}", format.layout()),
                ),
                DtmError::UnsupportedFormat { code } => (
                    ValidationSeverity::Warning,
                    2,
                    None,
                    format!("value not checked: format code {code:?} is not decoded"),
                ),
                _ => continue,
            };
            let mut issue =
                ValidationIssue::new(severity, format!("DTM+{qualifier} value {value:?}: {err}"))
                    .with_kind(FindingKind::Representation)
                    .with_rule_id("DTM-2379")
                    .with_segment("DTM")
                    .with_element_index(0)
                    .with_component_index(component)
                    .with_suggestion(suggestion);
            if let Some(code) = code {
                issue = issue.with_error_code(code);
            }
            if let Some(span) = segment
                .get_element(0)
                .and_then(|e| e.component_span(component))
            {
                issue = issue.with_span(span);
            }
            if let Some(position) = positions.position(index) {
                issue = issue.with_segment_position(position);
            }
            match severity {
                ValidationSeverity::Warning => report.add_warning(issue),
                _ => report.add_error(issue),
            }
        }
    }

    fn fork(&self) -> Option<Box<dyn Validator + Send + Sync>> {
        Some(Box::new(Self))
    }
}

// ── time conversions ──────────────────────────────────────────────────────────

#[cfg(feature = "dtm-time")]
#[cfg_attr(docsrs, doc(cfg(feature = "dtm-time")))]
impl DtmValue {
    fn not_convertible(&self, target: &'static str) -> DtmError {
        DtmError::NotConvertible {
            format: self.format,
            target,
        }
    }

    fn time_date(&self, target: &'static str) -> Result<time::Date, DtmError> {
        let month =
            time::Month::try_from(self.start.month).map_err(|_| self.not_convertible(target))?;
        time::Date::from_calendar_date(i32::from(self.start.year), month, self.start.day)
            .map_err(|_| self.not_convertible(target))
    }

    /// A `102` value as [`time::Date`].
    ///
    /// ```
    /// use edifact::DtmValue;
    /// use time::{Date, Month};
    ///
    /// let d = DtmValue::parse("20261001", "102")?.to_date()?;
    /// assert_eq!(d, Date::from_calendar_date(2026, Month::October, 1).unwrap());
    /// # Ok::<(), edifact::DtmError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`DtmError::NotConvertible`] for any other format.
    pub fn to_date(&self) -> Result<time::Date, DtmError> {
        if self.format != DtmFormat::Ccyymmdd {
            return Err(self.not_convertible("Date"));
        }
        self.time_date("Date")
    }

    fn wall_clock(&self, target: &'static str) -> Result<time::PrimitiveDateTime, DtmError> {
        let date = self.time_date(target)?;
        let s = self.start;
        if s.hour == 24 {
            let next = date
                .next_day()
                .ok_or_else(|| self.not_convertible(target))?;
            return Ok(time::PrimitiveDateTime::new(next, time::Time::MIDNIGHT));
        }
        let t = time::Time::from_hms(s.hour, s.minute, s.second)
            .map_err(|_| self.not_convertible(target))?;
        Ok(time::PrimitiveDateTime::new(date, t))
    }

    /// A floating `203`/`204` value as [`time::PrimitiveDateTime`]; hour 24
    /// becomes the next day's midnight. Attach an offset with
    /// `assume_offset`: the codec never assumes one.
    ///
    /// ```
    /// use edifact::DtmValue;
    /// use time::UtcOffset;
    ///
    /// let v = DtmValue::parse("201801012400", "203")?;
    /// let at = v.to_primitive_date_time()?.assume_offset(UtcOffset::UTC);
    /// assert_eq!(at.unix_timestamp(), 1_514_851_200); // 2018-01-02 00:00 UTC
    /// assert!(v.to_offset_date_time().is_err());
    /// # Ok::<(), edifact::DtmError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`DtmError::NotConvertible`] for any other format, or past year 9999.
    pub fn to_primitive_date_time(&self) -> Result<time::PrimitiveDateTime, DtmError> {
        const TARGET: &str = "PrimitiveDateTime";
        if !matches!(
            self.format,
            DtmFormat::Ccyymmddhhmm | DtmFormat::Ccyymmddhhmmss
        ) {
            return Err(self.not_convertible(TARGET));
        }
        self.wall_clock(TARGET)
    }

    /// A zoned `303`/`304` value as [`time::OffsetDateTime`] with its offset.
    ///
    /// ```
    /// use edifact::DtmValue;
    ///
    /// let at = DtmValue::parse("202610011200+02", "303")?.to_offset_date_time()?;
    /// assert_eq!(at.offset().whole_hours(), 2);
    /// assert_eq!(at.unix_timestamp(), 1_790_848_800); // 10:00 UTC
    /// # Ok::<(), edifact::DtmError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`DtmError::NotConvertible`] for floating and other formats, or past
    /// year 9999.
    pub fn to_offset_date_time(&self) -> Result<time::OffsetDateTime, DtmError> {
        const TARGET: &str = "OffsetDateTime";
        let Some(zone) = self.zone else {
            return Err(self.not_convertible(TARGET));
        };
        let offset = time::UtcOffset::from_hms(zone.offset_hours(), 0, 0)
            .map_err(|_| self.not_convertible(TARGET))?;
        Ok(self.wall_clock(TARGET)?.assume_offset(offset))
    }

    /// `dt` in `format` (`102`, `203`, `204`), truncated to its precision.
    ///
    /// ```
    /// use edifact::{DtmFormat, DtmValue};
    /// use time::{Date, Month, PrimitiveDateTime, Time};
    ///
    /// let dt = PrimitiveDateTime::new(
    ///     Date::from_calendar_date(2026, Month::October, 1).unwrap(),
    ///     Time::from_hms(12, 30, 45).unwrap(),
    /// );
    /// let v = DtmValue::from_primitive(dt, DtmFormat::Ccyymmddhhmm)?;
    /// assert_eq!(v.to_string(), "202610011230");
    /// # Ok::<(), edifact::DtmError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`DtmError::NotConvertible`] for another format or a year outside
    /// 1–9999.
    pub fn from_primitive(
        dt: time::PrimitiveDateTime,
        format: DtmFormat,
    ) -> Result<Self, DtmError> {
        let seconds = match format {
            DtmFormat::Ccyymmdd => None,
            DtmFormat::Ccyymmddhhmm => Some(false),
            DtmFormat::Ccyymmddhhmmss => Some(true),
            _ => {
                return Err(DtmError::NotConvertible {
                    format,
                    target: "102/203/204",
                });
            }
        };
        Self::from_wall_clock(dt, format, seconds, None)
    }

    fn from_wall_clock(
        dt: time::PrimitiveDateTime,
        format: DtmFormat,
        seconds: Option<bool>,
        zone: Option<Zone>,
    ) -> Result<Self, DtmError> {
        let year = u16::try_from(dt.year())
            .ok()
            .filter(|y| (1..=9999).contains(y))
            .ok_or(DtmError::NotConvertible {
                format,
                target: "a year in 0001-9999",
            })?;
        let mut start = Stamp {
            year,
            month: u8::from(dt.month()),
            day: dt.day(),
            ..Stamp::default()
        };
        if let Some(with_seconds) = seconds {
            start.hour = dt.hour();
            start.minute = dt.minute();
            if with_seconds {
                start.second = dt.second();
            }
        }
        Ok(Self {
            format,
            start,
            end: Stamp::default(),
            zone,
            count: 0,
        })
    }

    /// `dt` in its own offset as `303`/`304`, truncated to its precision.
    ///
    /// For a send timestamp, pass `OffsetDateTime::now_utc()`.
    ///
    /// ```
    /// use edifact::{DtmFormat, DtmValue};
    /// use time::OffsetDateTime;
    ///
    /// let t = OffsetDateTime::from_unix_timestamp(1_790_856_045).unwrap(); // 12:00:45 UTC
    /// let v = DtmValue::from_offset_date_time(t, DtmFormat::CcyymmddhhmmZzz)?;
    /// assert_eq!(v.to_string(), "202610011200+00");
    /// # Ok::<(), edifact::DtmError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`DtmError::InvalidOffset`] for an offset that is not whole hours
    /// within ±14, [`DtmError::NotConvertible`] for another format or a year
    /// outside 1–9999.
    pub fn from_offset_date_time(
        dt: time::OffsetDateTime,
        format: DtmFormat,
    ) -> Result<Self, DtmError> {
        let seconds = match format {
            DtmFormat::CcyymmddhhmmZzz => false,
            DtmFormat::CcyymmddhhmmssZzz => true,
            _ => {
                return Err(DtmError::NotConvertible {
                    format,
                    target: "303/304",
                });
            }
        };
        let offset = dt.offset().whole_seconds();
        let zone = (offset % 3600 == 0)
            .then(|| i8::try_from(offset / 3600).ok())
            .flatten()
            .and_then(zone_for)
            .ok_or(DtmError::InvalidOffset { seconds: offset })?;
        let local = time::PrimitiveDateTime::new(dt.date(), dt.time());
        Self::from_wall_clock(local, format, Some(seconds), Some(zone))
    }
}

#[cfg(feature = "dtm-time")]
#[cfg_attr(docsrs, doc(cfg(feature = "dtm-time")))]
impl TryFrom<time::Date> for DtmValue {
    type Error = DtmError;

    /// `date` as `102`.
    ///
    /// ```
    /// use edifact::DtmValue;
    /// use time::{Date, Month};
    ///
    /// let d = Date::from_calendar_date(2024, Month::February, 29).unwrap();
    /// assert_eq!(DtmValue::try_from(d)?.to_string(), "20240229");
    /// # Ok::<(), edifact::DtmError>(())
    /// ```
    fn try_from(date: time::Date) -> Result<Self, Self::Error> {
        Self::from_primitive(date.midnight(), DtmFormat::Ccyymmdd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leap_years() {
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert!(DtmValue::parse("20240229", "102").is_ok());
        assert!(DtmValue::parse("20000229", "102").is_ok());
        assert!(DtmValue::parse("20260229", "102").is_err());
        assert!(DtmValue::parse("19000229", "102").is_err());
        assert!(DtmValue::parse("0229", "106").is_ok());
    }

    #[test]
    fn days_from_civil_known_points() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(days_from_civil(1, 1, 1), -719_162);
        assert_eq!(days_from_civil(9999, 12, 31), 2_932_896);
        assert_eq!(days_from_civil(10000, 1, 1), 2_932_897);
        assert_eq!(days_from_civil(2026, 10, 1), 20_727);
    }

    #[cfg(feature = "dtm-time")]
    mod time_conversions {
        use super::super::*;
        use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset};

        fn date(y: i32, m: Month, d: u8) -> Date {
            Date::from_calendar_date(y, m, d).unwrap()
        }

        #[test]
        fn zoned_303_keeps_instant_and_offset() {
            let at = DtmValue::parse("202610011200+02", "303")
                .unwrap()
                .to_offset_date_time()
                .unwrap();
            assert_eq!(at.offset(), UtcOffset::from_hms(2, 0, 0).unwrap());
            assert_eq!(at.date(), date(2026, Month::October, 1));
            assert_eq!(at.time(), Time::from_hms(12, 0, 0).unwrap());
            let utc = at.to_offset(UtcOffset::UTC);
            assert_eq!(utc.hour(), 10);
        }

        #[test]
        fn floating_needs_an_explicit_offset() {
            let v = DtmValue::parse("202610011200", "203").unwrap();
            assert_eq!(
                v.to_offset_date_time(),
                Err(DtmError::NotConvertible {
                    format: DtmFormat::Ccyymmddhhmm,
                    target: "OffsetDateTime"
                })
            );
            let at = v
                .to_primitive_date_time()
                .unwrap()
                .assume_offset(UtcOffset::from_hms(1, 0, 0).unwrap());
            assert_eq!(at.to_offset(UtcOffset::UTC).hour(), 11);
        }

        #[test]
        fn hour_24_is_next_midnight() {
            let dt = DtmValue::parse("201801012400", "203")
                .unwrap()
                .to_primitive_date_time()
                .unwrap();
            assert_eq!(
                dt,
                PrimitiveDateTime::new(date(2018, Month::January, 2), Time::MIDNIGHT)
            );
        }

        #[test]
        fn date_round_trip() {
            let d = date(2026, Month::October, 1);
            let v = DtmValue::try_from(d).unwrap();
            assert_eq!(v.to_string(), "20261001");
            assert_eq!(v.to_date().unwrap(), d);
            assert!(
                DtmValue::parse("202610011200", "203")
                    .unwrap()
                    .to_date()
                    .is_err()
            );
            assert!(DtmValue::try_from(date(0, Month::January, 1)).is_err());
        }

        #[test]
        fn from_primitive_truncates() {
            let dt = PrimitiveDateTime::new(
                date(2026, Month::October, 1),
                Time::from_hms(12, 30, 45).unwrap(),
            );
            let s = |f| DtmValue::from_primitive(dt, f).unwrap().to_string();
            assert_eq!(s(DtmFormat::Ccyymmdd), "20261001");
            assert_eq!(s(DtmFormat::Ccyymmddhhmm), "202610011230");
            assert_eq!(s(DtmFormat::Ccyymmddhhmmss), "20261001123045");
            assert!(DtmValue::from_primitive(dt, DtmFormat::CcyymmddhhmmZzz).is_err());
        }

        #[test]
        fn from_offset_date_time_checks_the_offset() {
            let t = OffsetDateTime::from_unix_timestamp(1_790_856_045).unwrap();
            let v = DtmValue::from_offset_date_time(t, DtmFormat::CcyymmddhhmmZzz).unwrap();
            assert_eq!(v.to_string(), "202610011200+00");
            assert_eq!(v.to_string().len(), 15);
            let s = DtmValue::from_offset_date_time(t, DtmFormat::CcyymmddhhmmssZzz).unwrap();
            assert_eq!(s.to_string(), "20261001120045+00");

            let ist = t.to_offset(UtcOffset::from_hms(5, 30, 0).unwrap());
            assert_eq!(
                DtmValue::from_offset_date_time(ist, DtmFormat::CcyymmddhhmmZzz),
                Err(DtmError::InvalidOffset { seconds: 19_800 })
            );
            let far = t.to_offset(UtcOffset::from_hms(-15, 0, 0).unwrap());
            assert_eq!(
                DtmValue::from_offset_date_time(far, DtmFormat::CcyymmddhhmmZzz),
                Err(DtmError::InvalidOffset { seconds: -54_000 })
            );
            let east = t.to_offset(UtcOffset::from_hms(-5, 0, 0).unwrap());
            assert_eq!(
                DtmValue::from_offset_date_time(east, DtmFormat::CcyymmddhhmmZzz)
                    .unwrap()
                    .to_string(),
                "202610010700-05"
            );
        }

        #[test]
        fn range_edges_never_panic() {
            let v = DtmValue::parse("999912312400", "203").unwrap();
            assert!(matches!(
                v.to_primitive_date_time(),
                Err(DtmError::NotConvertible { .. })
            ));
            for s in [
                "999912312359-14",
                "999912312359+14",
                "000101010000+14",
                "000101010000-14",
            ] {
                let at = DtmValue::parse(s, "303").unwrap().to_offset_date_time();
                assert!(at.is_ok(), "{s}: {at:?}");
            }
            assert!(
                DtmValue::parse("999912312400+00", "303")
                    .unwrap()
                    .to_offset_date_time()
                    .is_err()
            );
        }
    }
}
