//! EDIFACT writer — serializes [`Segment`]s to wire format.

use crate::{
    charset::Charset,
    envelope::MessageIdentifier,
    error::EdifactError,
    header::{InterchangeHeader, SyntaxVersion},
    model::{Components, Element, OwnedSegment, Segment},
    parser::ReaderConfig,
    tokenizer::ServiceStringAdvice,
};
use smallvec::SmallVec;
use std::borrow::Cow;
use std::collections::HashSet;
use std::io::Write;

/// One data element of a segment being written: simple or composite.
///
/// The everyday EDIFACT segment mixes both shapes — `NAD+MS+id::agency`,
/// `DTM+137:20260101:102` — and this enum lets a single call express that
/// without pre-joining components into a string (which loses the distinction
/// between a separator and a literal `:` in a value).
///
/// `From` impls cover the common literals, so `"MS".into()` and
/// `["a", "", "b"].into()` both work; the [`elements!`][crate::elements] macro
/// wraps that up entirely.
///
/// # Example
///
/// ```rust
/// use edifact::{DataElement, Writer};
///
/// let mut w = Writer::new(Vec::new());
/// w.write_elements(
///     "NAD",
///     &[
///         DataElement::Simple("MS"),
///         DataElement::Composite(&["9900112233445", "", "293"]),
///     ],
/// )?;
/// assert_eq!(w.finish()?, b"NAD+MS+9900112233445::293'".to_vec());
/// # Ok::<(), edifact::EdifactError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataElement<'a> {
    /// A simple data element — one value, no component separators.
    Simple(&'a str),
    /// A composite data element — components written in order, separated by the
    /// active component separator.  A separator byte *inside* a component value
    /// is escaped rather than promoted to a boundary.
    Composite(&'a [&'a str]),
}

impl<'a> DataElement<'a> {
    /// The components of this element, as a slice.
    #[inline]
    #[must_use]
    pub fn components(&self) -> &[&'a str] {
        match self {
            Self::Simple(value) => std::slice::from_ref(value),
            Self::Composite(components) => components,
        }
    }
}

impl<'a> From<&'a str> for DataElement<'a> {
    #[inline]
    fn from(value: &'a str) -> Self {
        Self::Simple(value)
    }
}

impl<'a> From<&'a [&'a str]> for DataElement<'a> {
    #[inline]
    fn from(components: &'a [&'a str]) -> Self {
        Self::Composite(components)
    }
}

impl<'a, const N: usize> From<&'a [&'a str; N]> for DataElement<'a> {
    #[inline]
    fn from(components: &'a [&'a str; N]) -> Self {
        Self::Composite(components)
    }
}

/// Borrow a value as a [`DataElement`], choosing simple or composite by type.
///
/// A single string borrows as [`DataElement::Simple`]; an array, slice, or `Vec`
/// of strings borrows as [`DataElement::Composite`]. This is what lets the
/// [`elements!`][crate::elements] macro accept both shapes from arbitrary
/// expressions rather than only from literals.
///
/// # Example
///
/// ```rust
/// use edifact::{AsDataElement, DataElement};
///
/// let qualifier = String::from("MS");
/// let party = ["9900112233445", "", "293"];
///
/// assert_eq!(qualifier.as_data_element(), DataElement::Simple("MS"));
/// assert_eq!(
///     party.as_data_element(),
///     DataElement::Composite(&["9900112233445", "", "293"]),
/// );
/// ```
pub trait AsDataElement {
    /// Borrow `self` as a [`DataElement`].
    fn as_data_element(&self) -> DataElement<'_>;
}

impl AsDataElement for str {
    #[inline]
    fn as_data_element(&self) -> DataElement<'_> {
        DataElement::Simple(self)
    }
}

impl AsDataElement for &str {
    #[inline]
    fn as_data_element(&self) -> DataElement<'_> {
        DataElement::Simple(self)
    }
}

impl AsDataElement for String {
    #[inline]
    fn as_data_element(&self) -> DataElement<'_> {
        DataElement::Simple(self.as_str())
    }
}

impl AsDataElement for Cow<'_, str> {
    #[inline]
    fn as_data_element(&self) -> DataElement<'_> {
        DataElement::Simple(self.as_ref())
    }
}

impl<const N: usize> AsDataElement for [&str; N] {
    #[inline]
    fn as_data_element(&self) -> DataElement<'_> {
        DataElement::Composite(self)
    }
}

impl AsDataElement for [&str] {
    #[inline]
    fn as_data_element(&self) -> DataElement<'_> {
        DataElement::Composite(self)
    }
}

impl AsDataElement for Vec<&str> {
    #[inline]
    fn as_data_element(&self) -> DataElement<'_> {
        DataElement::Composite(self)
    }
}

impl AsDataElement for DataElement<'_> {
    #[inline]
    fn as_data_element(&self) -> DataElement<'_> {
        *self
    }
}

/// Build a `&[`[`DataElement`]`]` from a mix of simple values and component lists.
///
/// Each entry is an arbitrary expression borrowed through
/// [`AsDataElement`]: a string becomes a simple data element, an array or slice
/// of strings becomes a composite. This is the shorthand for the mixed-segment
/// shape that dominates real EDIFACT:
///
/// ```rust
/// use edifact::{Writer, elements};
///
/// // Runtime values, not just literals — the everyday builder shape.
/// let qualifier = String::from("MS");
/// let gln = "9900112233445";
///
/// let mut w = Writer::new(Vec::new());
/// w.write_elements("NAD", elements![qualifier.as_str(), [gln, "", "293"]])?;
/// w.write_elements("DTM", elements![["137", "20260101", "102"]])?;
/// assert_eq!(
///     w.finish()?,
///     b"NAD+MS+9900112233445::293'DTM+137:20260101:102'".to_vec(),
/// );
/// # Ok::<(), edifact::EdifactError>(())
/// ```
///
/// Composite components must be string *slices*: a `[String; N]` cannot borrow
/// as `&[&str]` without allocating, so write `[id.as_str(), "", agency]`.
///
/// The expansion borrows temporaries, so the result must be consumed within the
/// same statement — passing it directly as an argument, as above, always is.
#[macro_export]
macro_rules! elements {
    () => {
        &[] as &[$crate::DataElement<'_>]
    };
    ($($element:expr),+ $(,)?) => {
        &[$($crate::AsDataElement::as_data_element(&$element)),+][..]
    };
}

/// Streaming EDIFACT writer.
///
/// Wraps any [`Write`] implementation and serializes segments one at a time.
/// Call [`Writer::finish`] to flush and get the underlying writer back.
///
/// Trailing empty data elements and trailing empty components are never
/// written (ISO 9735-1:2002 §8.7); an empty value between present ones keeps
/// its separator.
///
/// ```rust
/// use edifact::Writer;
///
/// let mut w = Writer::new(Vec::new());
/// w.write_simple("BGM", &["220", "", "9", ""])?;
/// w.write_composites("NAD", &[&["BY"][..], &["", "", ""][..]])?;
/// assert_eq!(w.finish()?, b"BGM+220++9'NAD+BY'".to_vec());
/// # Ok::<(), edifact::EdifactError>(())
/// ```
///
/// # What it will not write
///
/// Everything the writer emits reparses. These cases are *refused* before a
/// byte reaches the sink, so a refused call leaves nothing half-written and the
/// writer stays usable:
///
/// - A segment tag that is not three ASCII uppercase letters
///   ([`EdifactError::InvalidSegmentTag`]). A tag is written verbatim — EDIFACT
///   has no way to escape one — so `bgm`, `BGMX`, or `B+M` would produce bytes
///   that do not read back as the segment they came from.
/// - A repeating data element when no repetition separator is declared
///   ([`EdifactError::RepetitionSeparatorNotDeclared`]).
/// - A `UNB` or `UNZ` while an interchange opened by
///   [`begin_interchange`](Self::begin_interchange) is open
///   ([`EdifactError::InvalidEventSequence`]): the writer owns that envelope.
///   There, a `UNE` whose DE 0060 is not its group's message count is
///   [`EdifactError::GroupCountMismatch`].
/// - Every refusal of [`begin_interchange`](Self::begin_interchange) and
///   [`end_interchange`](Self::end_interchange).
///
/// Delimiters *inside a value* are not a problem: those are release-escaped.
///
/// # Poisoning
///
/// Any other error — a failed write to the sink, or a value the bound
/// [`Charset`] cannot encode — can leave a partial segment behind. The writer
/// is then *poisoned*: every later write, `begin_*`, `end_interchange`,
/// `finish` and `finish_unt` returns [`EdifactError::WriterPoisoned`] and
/// writes nothing. Dropping a [`MessageWriter`] without
/// [`finish`][MessageWriter::finish] poisons the writer too. The output of a
/// poisoned writer is incomplete by construction; [`is_poisoned`][Self::is_poisoned]
/// reports the state and [`into_inner`][Self::into_inner] returns the sink as
/// it is.
///
/// ```rust
/// use edifact::{EdifactError, MessageIdentifier, Writer};
///
/// let mut w = Writer::new(Vec::new());
/// {
///     let mut msg = w.begin_message(MessageIdentifier::new("1", "ORDERS", "D", "96A", "UN"))?;
///     msg.write_simple("BGM", &["220"])?;
///     // Dropped without `finish()`: no `UNT` is written.
/// }
/// assert!(w.is_poisoned());
/// assert!(matches!(
///     w.write_simple("BGM", &["220"]),
///     Err(EdifactError::WriterPoisoned { .. })
/// ));
/// assert_eq!(w.into_inner(), b"UNH+1+ORDERS:D:96A:UN'BGM+220'".to_vec());
/// # Ok::<(), edifact::EdifactError>(())
/// ```
///
/// # Wrap unbuffered sinks
///
/// `Writer` issues a separate write for each tag, delimiter, and value chunk, so
/// a segment costs roughly one write per component. Against an in-memory
/// `Vec<u8>` that is free, but against a [`File`][std::fs::File] or a socket each
/// one is a syscall.
///
/// The writer deliberately does **not** buffer internally: an internal buffer
/// would silently discard everything not yet flushed if the writer were dropped
/// without [`finish`][Self::finish]. Wrap the sink instead, which makes the
/// buffering visible and keeps the flush contract in one place:
///
/// ```rust
/// use std::io::BufWriter;
/// use edifact::Writer;
///
/// let sink = Vec::new(); // stands in for a File or TcpStream
/// let mut writer = Writer::new(BufWriter::new(sink));
/// writer.write_simple("BGM", &["220"])?;
/// // `finish` flushes the `Writer` and hands the `BufWriter` back.
/// let buffered = writer.finish()?;
/// assert_eq!(buffered.into_inner().unwrap(), b"BGM+220'".to_vec());
/// # Ok::<(), edifact::EdifactError>(())
/// ```
pub struct Writer<W: Write> {
    inner: W,
    ssa: ServiceStringAdvice,
    /// Running count of segments written.  `u64` to prevent silent overflow on
    /// pathological inputs (a `u32` would wrap after ~4 billion segments).
    segment_count: u64,
    /// `segment_count` as of the most recent `UNH`, used by [`Writer::finish_unt`]
    /// to derive a per-message DE 0074 rather than a writer-lifetime total.
    message_start_count: u64,
    /// Tag of the segment the event-emitter path is writing.  The
    /// whole-segment methods pass the tag to `end_segment` directly.
    open_tag: [u8; 3],
    /// Separators held back since the last non-empty value of the open
    /// segment.  Written before the next value; dropped at the segment end,
    /// which is what truncates trailing empty elements and components.
    pending: SmallVec<[u8; 16]>,
    /// Repertoire every value is encoded into, when the writer is bound to one.
    ///
    /// `None` emits UTF-8 unchecked, which is correct for `UNOY` and for any
    /// payload that happens to be ASCII.
    charset: Option<Charset>,
    /// The interchange opened by `begin_interchange`, while it is open.
    interchange: Option<OpenInterchange>,
    /// Why the writer stopped writing, once a failure left the sink in an
    /// unknown state. Never reset.
    poisoned: Option<&'static str>,
}

/// What `end_interchange` needs to close the open interchange.
struct OpenInterchange {
    control_ref: String,
    version: SyntaxVersion,
    messages: u64,
    groups: u64,
    ungrouped_messages: u64,
    in_group: bool,
    /// Messages in the open group, for its `UNE`.
    group_messages: u64,
    in_message: bool,
}

/// Poison reason for a failed write to the sink.
const SINK_FAILED: &str = "an earlier write to the sink failed";
/// Poison reason for an error raised after a call already wrote bytes.
const SEGMENT_INCOMPLETE: &str = "an earlier call left a segment incomplete";
/// Poison reason for a [`MessageWriter`] dropped while open.
const MESSAGE_ABANDONED: &str = "a message was dropped without finish()";

/// Refuse a tag the parser would not read back.
///
/// A tag is emitted verbatim — EDIFACT has no way to escape it — so a
/// lowercase, mis-sized, or delimiter-bearing tag produces bytes that do not
/// reparse as the segment they came from. Applies the parser's predicate, so
/// the two cannot drift.
#[inline]
pub(crate) fn check_tag(tag: &str) -> Result<(), EdifactError> {
    if crate::tokenizer::is_valid_segment_tag(tag) {
        Ok(())
    } else {
        Err(EdifactError::InvalidSegmentTag(tag.to_owned()))
    }
}

/// The refusal for a raw `UNB`/`UNZ` inside an interchange the writer owns.
fn envelope_owned() -> EdifactError {
    EdifactError::InvalidEventSequence {
        message: "UNB and UNZ of an open interchange are written by begin_interchange and end_interchange",
    }
}

/// Return the offset of the first byte in `hay` that must be release-escaped.
///
/// The escape set is the four splitting delimiters plus the repetition separator
/// when the active UNA declares one.  A space at UNA position 7 is the
/// conventional "not used" sentinel and is never escaped.
#[inline]
fn find_escape(ssa: &ServiceStringAdvice, hay: &[u8]) -> Option<usize> {
    let first = memchr::memchr3(ssa.element_sep, ssa.component_sep, ssa.release_char, hay);
    let second = if ssa.repetition_sep == b' ' {
        memchr::memchr(ssa.segment_term, hay)
    } else {
        memchr::memchr2(ssa.segment_term, ssa.repetition_sep, hay)
    };
    match (first, second) {
        (None, None) => None,
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (Some(a), Some(b)) => Some(a.min(b)),
    }
}

impl<W: Write> Writer<W> {
    /// Create a new writer with default EDIFACT delimiters.
    pub fn new(inner: W) -> Self {
        Self::from_parts(inner, ServiceStringAdvice::default())
    }

    fn from_parts(inner: W, ssa: ServiceStringAdvice) -> Self {
        Self {
            inner,
            ssa,
            segment_count: 0,
            message_start_count: 0,
            open_tag: *b"   ",
            pending: SmallVec::new(),
            charset: None,
            interchange: None,
            poisoned: None,
        }
    }

    /// Whether an earlier failure poisoned this writer.
    ///
    /// A poisoned writer returns [`EdifactError::WriterPoisoned`] from every
    /// write and completion method and writes nothing more. See
    /// [Poisoning](Self#poisoning).
    #[must_use]
    pub fn is_poisoned(&self) -> bool {
        self.poisoned.is_some()
    }

    /// Return the sink as it is: no flush, no checks, poisoned or not.
    ///
    /// The way to inspect or discard the partial output of a poisoned writer;
    /// [`finish`][Self::finish] refuses it. Flush a buffered sink yourself.
    #[must_use]
    pub fn into_inner(self) -> W {
        self.inner
    }

    /// Fail with [`EdifactError::WriterPoisoned`] once the writer is poisoned.
    #[inline]
    pub(crate) fn check_healthy(&self) -> Result<(), EdifactError> {
        match self.poisoned {
            None => Ok(()),
            Some(reason) => Err(EdifactError::WriterPoisoned { reason }),
        }
    }

    /// Refuse a tag the parser would not read back, a `UNB`/`UNZ` inside an
    /// interchange this writer opened, or there a `UNE` whose DE 0060 (`first`
    /// value) is not the group's message count.
    ///
    /// `values` are the first components of the first two data elements.
    #[inline]
    fn check_segment_tag(&self, tag: &str, values: [Option<&str>; 2]) -> Result<(), EdifactError> {
        check_tag(tag)?;
        let Some(open) = &self.interchange else {
            return Ok(());
        };
        match tag {
            "UNB" | "UNZ" => Err(envelope_owned()),
            "UNE"
                if open.in_group
                    && values[0].and_then(|v| v.parse::<u64>().ok())
                        != Some(open.group_messages) =>
            {
                Err(EdifactError::GroupCountMismatch {
                    group_ref: values[1].unwrap_or("").to_owned(),
                    expected: values[0].and_then(|v| v.parse().ok()).unwrap_or(u32::MAX),
                    actual: u32::try_from(open.group_messages).unwrap_or(u32::MAX),
                    span: crate::model::Span::default(),
                })
            }
            _ => Ok(()),
        }
    }

    /// Poison the writer, keeping the first reason.
    #[inline]
    fn poison(&mut self, reason: &'static str) {
        self.poisoned.get_or_insert(reason);
    }

    /// Run the writing part of one call, poisoning the writer if it fails.
    ///
    /// Everything that can leave bytes in the sink runs in here; refusals run
    /// before it.
    #[inline]
    fn write_body(
        &mut self,
        body: impl FnOnce(&mut Self) -> Result<(), EdifactError>,
    ) -> Result<(), EdifactError> {
        let result = body(self);
        if let Err(error) = &result {
            self.poison(match error {
                EdifactError::Io(_) => SINK_FAILED,
                _ => SEGMENT_INCOMPLETE,
            });
        }
        result
    }

    /// Bind this writer to a character repertoire.
    ///
    /// Every value is then encoded into `charset` rather than emitted as UTF-8,
    /// and a character the repertoire cannot carry is rejected with
    /// [`EdifactError::CharacterNotInRepertoire`] instead of being written as
    /// bytes the receiver decodes as something else.
    ///
    /// This is the write-side counterpart of
    /// [`decode_interchange`][crate::decode_interchange]: a `UNOC` interchange
    /// must go out as ISO 8859-1, not UTF-8, or `ü` arrives as two mojibake
    /// characters. [`begin_interchange`](Self::begin_interchange) binds an
    /// unbound writer to the header's repertoire.
    ///
    /// # Example
    ///
    /// ```
    /// use edifact::{Charset, Writer};
    ///
    /// let mut writer = Writer::new(Vec::new()).with_charset(Charset::UnoC);
    /// writer.write_composites("NAD", &[&["BY"], &["Müller"]])?;
    /// // `ü` goes out as the single Latin-1 byte 0xFC.
    /// assert_eq!(writer.finish()?, b"NAD+BY+M\xFCller'".to_vec());
    /// # Ok::<(), edifact::EdifactError>(())
    /// ```
    ///
    /// A value outside the repertoire is refused:
    ///
    /// ```
    /// use edifact::{Charset, EdifactError, Writer};
    ///
    /// let mut writer = Writer::new(Vec::new()).with_charset(Charset::UnoA);
    /// // Level A is upper-case only.
    /// let err = writer.write_composites("NAD", &[&["BY"], &["Müller"]]).unwrap_err();
    /// assert!(matches!(err, EdifactError::CharacterNotInRepertoire { .. }));
    /// ```
    #[must_use]
    pub fn with_charset(mut self, charset: Charset) -> Self {
        self.charset = Some(charset);
        self
    }

    /// The repertoire this writer encodes into, if it is bound to one.
    #[must_use]
    pub fn charset(&self) -> Option<Charset> {
        self.charset
    }

    /// Create a writer that uses `ssa`'s delimiters **without** emitting a `UNA`.
    ///
    /// For replying on an inbound interchange's delimiters, or round-tripping a
    /// syntax-version-4 interchange that has repeating elements but no `UNA`:
    /// the writer needs the repetition separator, and
    /// [`with_una`][Self::with_una] would add a header the original lacked.
    ///
    /// # Errors
    ///
    /// [`EdifactError::InvalidUna`] when the service characters are not mutually
    /// distinct printable non-alphanumeric ASCII — see
    /// [`ServiceStringAdvice::is_valid`].
    ///
    /// # Example
    ///
    /// ```
    /// use edifact::{ServiceStringAdvice, Writer, from_bytes};
    ///
    /// // Version 4: `*` separates RFF's two occurrences, with no UNA to say so.
    /// let input = b"UNB+UNOC:4+S+R+260101:0900+I'RFF+ON:1*ON:2'UNZ+0+I'";
    /// let segments: Vec<_> = from_bytes(input).collect::<Result<Vec<_>, _>>()?;
    ///
    /// let ssa = ServiceStringAdvice::for_syntax_version(Some(4));
    /// let mut writer = Writer::with_service_string_advice(Vec::new(), ssa)?;
    /// for segment in &segments {
    ///     writer.write_segment(segment)?;
    /// }
    /// // Byte-for-byte the input, with no UNA invented.
    /// assert_eq!(writer.finish()?, input.to_vec());
    /// # Ok::<(), edifact::EdifactError>(())
    /// ```
    pub fn with_service_string_advice(
        inner: W,
        ssa: ServiceStringAdvice,
    ) -> Result<Self, EdifactError> {
        if !ssa.is_valid() {
            return Err(EdifactError::InvalidUna);
        }
        Ok(Self::from_parts(inner, ssa))
    }

    /// Create a writer with custom delimiters and write a `UNA` segment first.
    ///
    /// [`with_service_string_advice`][Self::with_service_string_advice] is the
    /// same thing without the header.
    ///
    /// # Errors
    ///
    /// As [`with_service_string_advice`][Self::with_service_string_advice], plus
    /// a failed `UNA` write. No `Writer` exists yet then, so the sink is dropped
    /// with the error.
    pub fn with_una(mut inner: W, ssa: ServiceStringAdvice) -> Result<Self, EdifactError> {
        // All five active service characters must be mutually distinct, non-whitespace,
        // and within the ASCII range so they never bisect multi-byte UTF-8 sequences.
        if !ssa.is_valid() {
            return Err(EdifactError::InvalidUna);
        }
        // UNA: component_sep, element_sep, decimal_mark, release_char, repetition_sep, segment_term
        let una = [
            b'U',
            b'N',
            b'A',
            ssa.component_sep,
            ssa.element_sep,
            ssa.decimal_mark,
            ssa.release_char,
            ssa.repetition_sep,
            ssa.segment_term,
        ];
        inner.write_all(&una)?;
        Ok(Self::from_parts(inner, ssa))
    }

    /// Hold back a separator until a non-empty value follows it.
    ///
    /// An element separator drops the held component and repetition
    /// separators before it — they would end the previous element in empty
    /// components — and a repetition separator drops held component ones.
    #[inline]
    fn sep(&mut self, separator: u8) {
        let ssa = self.ssa;
        let droppable = |b: u8| {
            b == ssa.component_sep
                || (separator == ssa.element_sep
                    && ssa.is_repetition_active()
                    && b == ssa.repetition_sep)
        };
        if separator != ssa.component_sep {
            while self.pending.last().is_some_and(|&b| droppable(b)) {
                self.pending.pop();
            }
        }
        self.pending.push(separator);
    }

    /// Write `value` after the separators held back for it; an empty value
    /// writes nothing.
    #[inline]
    fn value(&mut self, value: &str) -> Result<(), EdifactError> {
        if value.is_empty() {
            return Ok(());
        }
        if !self.pending.is_empty() {
            self.inner.write_all(&self.pending)?;
            self.pending.clear();
        }
        self.write_escaped(value)
    }

    /// Record the end of a segment: terminator, count, and envelope
    /// bookkeeping.
    ///
    /// Every emit path funnels through here, so
    /// [`finish_unt`][Self::finish_unt] derives DE 0074 from the current
    /// message and [`end_interchange`][Self::end_interchange] counts every
    /// message and group, however they were written.
    #[inline]
    fn end_segment(&mut self, tag: &str) -> Result<(), EdifactError> {
        self.pending.clear();
        self.inner.write_all(&[self.ssa.segment_term])?;
        self.count(tag);
        Ok(())
    }

    /// Count one segment tagged `tag`.
    #[inline]
    fn count(&mut self, tag: &str) {
        if tag == "UNH" {
            self.message_start_count = self.segment_count;
        }
        if let Some(open) = &mut self.interchange {
            match tag {
                "UNH" => {
                    open.messages += 1;
                    open.ungrouped_messages += u64::from(!open.in_group);
                    open.group_messages += u64::from(open.in_group);
                    open.in_message = true;
                }
                "UNT" => open.in_message = false,
                "UNG" => {
                    open.groups += 1;
                    open.in_group = true;
                    open.group_messages = 0;
                }
                "UNE" => open.in_group = false,
                _ => {}
            }
        }
        self.segment_count += 1;
    }

    /// Write a single segment, including any ISO 9735-4 repetitions.
    ///
    /// # Errors
    ///
    /// Returns [`EdifactError::RepetitionSeparatorNotDeclared`] when the segment
    /// carries a repeating data element but the active service string advice
    /// declares no repetition separator.  The check runs **before** any byte is
    /// written, so a rejected segment leaves nothing behind in the sink — a
    /// half-written `RFF+` would otherwise corrupt the interchange for every
    /// caller that recovers from the error and carries on.
    pub fn write_segment(&mut self, seg: &Segment<'_>) -> Result<(), EdifactError> {
        self.check_healthy()?;
        check_repetition(&self.ssa, seg)?;
        let tag = seg.tag();
        self.check_segment_tag(tag, [seg.component_str(0, 0), seg.component_str(1, 0)])?;

        self.write_body(|w| {
            w.inner.write_all(tag.as_bytes())?;
            for element in &seg.elements {
                w.sep(w.ssa.element_sep);
                for (repetition, components) in element.repetitions().enumerate() {
                    if repetition > 0 {
                        w.sep(w.ssa.repetition_sep);
                    }
                    for (i, (component, _)) in components.iter().enumerate() {
                        if i > 0 {
                            w.sep(w.ssa.component_sep);
                        }
                        w.value(component)?;
                    }
                }
            }
            w.end_segment(tag)
        })
    }

    /// Write a segment whose data elements are all **simple** — one value each.
    ///
    /// The shorthand for the commonest segment shape. Each string is one whole
    /// data element: a component separator inside a value is escaped as data,
    /// not promoted to a component boundary, so the output is correct whatever
    /// delimiters the writer uses.
    ///
    /// Reach for [`write_composites`][Self::write_composites] when the elements
    /// have components, or [`write_elements`][Self::write_elements] when the
    /// segment mixes the two shapes.
    ///
    /// # Example
    ///
    /// ```
    /// use edifact::Writer;
    ///
    /// let mut w = Writer::new(Vec::new());
    /// w.write_simple("BGM", &["220", "PO-4711", "9"])?;
    /// // A literal `:` stays inside the value it belongs to.
    /// w.write_simple("FTX", &["AAA", "ACME:INC"])?;
    /// assert_eq!(w.finish()?, b"BGM+220+PO-4711+9'FTX+AAA+ACME?:INC'".to_vec());
    /// # Ok::<(), edifact::EdifactError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Refuses an invalid tag without writing. A sink failure, or a value this
    /// writer's [`Charset`] cannot encode, poisons the writer.
    pub fn write_simple<S: AsRef<str>>(
        &mut self,
        tag: &str,
        elements: &[S],
    ) -> Result<(), EdifactError> {
        self.check_healthy()?;
        self.check_segment_tag(
            tag,
            [
                elements.first().map(AsRef::as_ref),
                elements.get(1).map(AsRef::as_ref),
            ],
        )?;
        self.write_body(|w| {
            w.inner.write_all(tag.as_bytes())?;
            for element in elements {
                w.sep(w.ssa.element_sep);
                w.value(element.as_ref())?;
            }
            w.end_segment(tag)
        })
    }

    /// Write a segment whose data elements are all **composite** — a list of
    /// components each.
    ///
    /// Component boundaries are given explicitly rather than inferred by
    /// splitting, so a value containing the active component separator is
    /// escaped instead of being silently reinterpreted as a boundary.
    ///
    /// The bounds accept borrowed and owned data alike — `&[&[&str]]`,
    /// `&[Vec<String>]`, `&[[String; 3]]` — so runtime-built segments need no
    /// conversion. A one-component element is a simple data element, which is
    /// what makes this the general all-elements form;
    /// [`write_elements`][Self::write_elements] is the shorthand for the mixed
    /// shape and [`write_simple`][Self::write_simple] for the all-simple one.
    ///
    /// # Example
    ///
    /// ```
    /// use edifact::Writer;
    ///
    /// let mut w = Writer::new(Vec::new());
    /// // Borrowed literals …
    /// w.write_composites("NAD", &[&["MS"][..], &["ACME:INC"][..]])?;
    /// // … and owned, runtime-built data, through the same call.
    /// let dtm = vec![vec!["137".to_string(), "20260101".to_string()]];
    /// w.write_composites("DTM", &dtm)?;
    /// assert_eq!(w.finish()?, b"NAD+MS+ACME?:INC'DTM+137:20260101'".to_vec());
    /// # Ok::<(), edifact::EdifactError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Refuses an invalid tag without writing. A sink failure, or a value this
    /// writer's [`Charset`] cannot encode, poisons the writer.
    pub fn write_composites<E, S>(&mut self, tag: &str, elements: &[E]) -> Result<(), EdifactError>
    where
        E: AsRef<[S]>,
        S: AsRef<str>,
    {
        self.check_healthy()?;
        self.check_segment_tag(
            tag,
            [
                elements
                    .first()
                    .and_then(|e| e.as_ref().first())
                    .map(AsRef::as_ref),
                elements
                    .get(1)
                    .and_then(|e| e.as_ref().first())
                    .map(AsRef::as_ref),
            ],
        )?;
        self.write_body(|w| {
            w.inner.write_all(tag.as_bytes())?;
            for element in elements {
                w.sep(w.ssa.element_sep);
                for (i, comp) in element.as_ref().iter().enumerate() {
                    if i > 0 {
                        w.sep(w.ssa.component_sep);
                    }
                    w.value(comp.as_ref())?;
                }
            }
            w.end_segment(tag)
        })
    }

    /// Write a segment whose data elements mix simple and composite shapes.
    ///
    /// This is the general form of segment emission and the one that matches
    /// how EDIFACT segments are actually specified: `NAD` takes a simple
    /// qualifier followed by a composite party identification, `DTM` takes a
    /// single composite.  [`write_simple`][Self::write_simple] and
    /// [`write_composites`][Self::write_composites] are the uniform special
    /// cases.
    ///
    /// Component boundaries are explicit, so a value containing the active
    /// component separator is escaped rather than silently promoted to a
    /// boundary.  Nothing is allocated.
    ///
    /// # Example
    ///
    /// ```rust
    /// use edifact::{DataElement, Writer, elements};
    ///
    /// let mut w = Writer::new(Vec::new());
    /// // Explicit form …
    /// w.write_elements(
    ///     "NAD",
    ///     &[DataElement::Simple("MS"), DataElement::Composite(&["ACME:INC", "", "9"])],
    /// )?;
    /// // … or the `elements!` shorthand.
    /// w.write_elements("DTM", elements![["137", "20260101", "102"]])?;
    /// assert_eq!(
    ///     w.finish()?,
    ///     b"NAD+MS+ACME?:INC::9'DTM+137:20260101:102'".to_vec(),
    /// );
    /// # Ok::<(), edifact::EdifactError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Refuses an invalid tag without writing. A sink failure, or a value this
    /// writer's [`Charset`] cannot encode, poisons the writer.
    pub fn write_elements(
        &mut self,
        tag: &str,
        elements: &[DataElement<'_>],
    ) -> Result<(), EdifactError> {
        self.check_healthy()?;
        self.check_segment_tag(
            tag,
            [
                elements
                    .first()
                    .and_then(|e| e.components().first().copied()),
                elements
                    .get(1)
                    .and_then(|e| e.components().first().copied()),
            ],
        )?;
        self.write_body(|w| {
            w.inner.write_all(tag.as_bytes())?;
            for element in elements {
                w.sep(w.ssa.element_sep);
                for (i, comp) in element.components().iter().enumerate() {
                    if i > 0 {
                        w.sep(w.ssa.component_sep);
                    }
                    w.value(comp)?;
                }
            }
            w.end_segment(tag)
        })
    }

    /// Flush and return the underlying writer.
    ///
    /// # Errors
    ///
    /// [`EdifactError::WriterPoisoned`] without flushing when the writer is
    /// poisoned — use [`into_inner`][Self::into_inner] to get the partial
    /// output — and [`EdifactError::Io`] when the flush fails.
    pub fn finish(mut self) -> Result<W, EdifactError> {
        self.check_healthy()?;
        self.inner.flush()?;
        Ok(self.inner)
    }

    /// Write the `UNT` segment and return the inner writer.
    ///
    /// The count written into `UNT` DE 0074 covers the current message only:
    /// `UNH`, every segment written since it, and `UNT` itself.  Segments written
    /// before the message's `UNH` — an interchange-level `UNB`, or a preceding
    /// message — are excluded, as EDIFACT requires.
    ///
    /// If no `UNH` has been written, the count falls back to every segment
    /// written so far plus one.
    ///
    /// # Errors
    ///
    /// [`EdifactError::WriterPoisoned`] when the writer is poisoned, otherwise
    /// any write or flush failure.
    pub fn finish_unt(mut self, message_ref: &str) -> Result<W, EdifactError> {
        // DE 0074 counts UNH + content + UNT.  `message_start_count` is the
        // absolute segment count immediately after UNH, so content is
        // `segment_count - message_start_count` and the total adds UNH and UNT.
        let count = self.segment_count - self.message_start_count + 1;
        let count_str = count.to_string();
        self.write_composites("UNT", &[&[count_str.as_str()], &[message_ref]])?;
        self.finish()
    }

    /// Returns the total number of segments written so far.
    pub fn segment_count(&self) -> u64 {
        self.segment_count
    }

    /// Returns the active [`ServiceStringAdvice`] (delimiter configuration).
    pub fn service_string_advice(&self) -> ServiceStringAdvice {
        self.ssa
    }

    /// Escape a value string for inclusion in an EDIFACT segment.
    ///
    /// Any character in `value` that matches the active element separator,
    /// component separator, release character, or segment terminator is escaped
    /// by prefixing it with the release character (default `?`).
    ///
    /// Returns a borrowed `Cow::Borrowed(value)` when no escaping is needed,
    /// avoiding an allocation on the fast path.
    ///
    /// # Example
    ///
    /// ```rust
    /// let writer = edifact::Writer::new(std::io::sink());
    /// // '+' must be escaped since it is the default element separator.
    /// assert_eq!(writer.escape_value("price+tax"), "price?+tax");
    /// ```
    pub fn escape_value<'v>(&self, value: &'v str) -> Cow<'v, str> {
        let bytes = value.as_bytes();
        if find_escape(&self.ssa, bytes).is_none() {
            return Cow::Borrowed(value);
        }
        // Built as a `String` from the start, so no fallible UTF-8 conversion
        // (with an unreachable failure branch) is needed.  Every delimiter is
        // single-byte ASCII (enforced by
        // `ServiceStringAdvice::is_valid`), so each hit lands on a character
        // boundary and both halves of the split are valid `&str`.
        let release = self.ssa.release_char as char;
        let mut out = String::with_capacity(value.len() + 4);
        let mut last = 0;
        while let Some(hit) = find_escape(&self.ssa, &bytes[last..]) {
            let abs = last + hit;
            out.push_str(&value[last..abs]);
            out.push(release);
            out.push(bytes[abs] as char);
            last = abs + 1;
        }
        out.push_str(&value[last..]);
        Cow::Owned(out)
    }
    // ── event-emitter helpers ─────────────────────────────────────────────────
    //
    // One helper per event, so [`crate::WriterEmitter`] writes each event as it
    // arrives. The emitter checks health first; each helper refuses before its
    // first byte and poisons on any failure after it.

    /// Write a segment tag (`StartSegment`).
    #[inline]
    pub(crate) fn emit_tag(&mut self, tag: &str) -> Result<(), EdifactError> {
        self.check_segment_tag(tag, [None, None])?;
        self.write_body(|w| w.inner.write_all(tag.as_bytes()).map_err(Into::into))?;
        // A valid tag is three ASCII bytes.
        self.open_tag.copy_from_slice(tag.as_bytes());
        Ok(())
    }

    /// Write an element separator and the first component (`Element`).
    #[inline]
    pub(crate) fn emit_element(&mut self, value: &str) -> Result<(), EdifactError> {
        self.write_body(|w| {
            w.sep(w.ssa.element_sep);
            w.value(value)
        })
    }

    /// Write a component separator and a component (`ComponentElement`).
    #[inline]
    pub(crate) fn emit_component(&mut self, value: &str) -> Result<(), EdifactError> {
        self.write_body(|w| {
            w.sep(w.ssa.component_sep);
            w.value(value)
        })
    }

    /// Write a repetition separator and a component (`RepeatElement`), or
    /// refuse when no repetition separator is declared.
    ///
    /// Emitting the space sentinel would produce output that reads back as a
    /// single occurrence whose value contains a space — corrupt, and quietly so.
    #[inline]
    pub(crate) fn emit_repetition(&mut self, value: &str) -> Result<(), EdifactError> {
        if !self.ssa.is_repetition_active() {
            return Err(EdifactError::RepetitionSeparatorNotDeclared);
        }
        self.write_body(|w| {
            w.sep(w.ssa.repetition_sep);
            w.value(value)
        })
    }

    /// Write the segment terminator and count the segment (`EndSegment`).
    #[inline]
    pub(crate) fn emit_segment_end(&mut self) -> Result<(), EdifactError> {
        let tag = self.open_tag;
        // `open_tag` holds a tag `emit_tag` validated: ASCII.
        let tag = std::str::from_utf8(&tag).unwrap_or("");
        self.write_body(|w| w.end_segment(tag))?;
        self.open_tag = *b"   ";
        Ok(())
    }

    /// Write text, encoding it into the bound repertoire when there is one.
    ///
    /// Callers must only pass slices that start and end on a character boundary.
    /// Every delimiter is single-byte ASCII (enforced by
    /// [`ServiceStringAdvice::is_valid`]), so splitting a value at a delimiter
    /// always satisfies that.
    #[inline]
    fn write_text(&mut self, text: &str) -> Result<(), EdifactError> {
        match self.charset {
            None => self.inner.write_all(text.as_bytes())?,
            Some(charset) => self.inner.write_all(&charset.encode(text)?)?,
        }
        Ok(())
    }

    /// Write a value, escaping any delimiter characters.
    fn write_escaped(&mut self, value: &str) -> Result<(), EdifactError> {
        let release = self.ssa.release_char;
        let bytes = value.as_bytes();
        let mut last = 0;
        let mut pos = 0;
        while pos < bytes.len() {
            let Some(hit) = find_escape(&self.ssa, &bytes[pos..]) else {
                break;
            };
            let abs = pos + hit;
            if abs > last {
                self.write_text(&value[last..abs])?;
            }
            // The escaped byte is a service character, hence ASCII in every
            // repertoire — it needs no encoding pass.
            self.inner.write_all(&[release, bytes[abs]])?;
            last = abs + 1;
            pos = abs + 1;
        }
        self.write_text(&value[last..])
    }

    // ── Interchange envelope ──────────────────────────────────────────────────

    /// Open an interchange: validate `header` and write its `UNB`.
    ///
    /// The writer then counts the messages (and groups, when any `UNG` is
    /// written) itself; [`end_interchange`](Self::end_interchange) writes the
    /// `UNZ`. An unbound writer is bound to the header's repertoire.
    ///
    /// ```
    /// use edifact::{Charset, InterchangeHeader, MessageIdentifier, Party, PreparationDateTime,
    ///               SyntaxVersion, Writer};
    ///
    /// let header = InterchangeHeader::new(Charset::UnoC, SyntaxVersion::V3,
    ///     Party::new("SENDER"), Party::new("RECEIVER"),
    ///     PreparationDateTime::new(2026, 1, 1, 9, 0)?, "IC1");
    /// let mut w = Writer::new(Vec::new());
    /// w.begin_interchange(&header)?;
    /// let mut msg = w.begin_message(MessageIdentifier::new("1", "ORDERS", "D", "96A", "UN"))?;
    /// msg.write_simple("BGM", &["220"])?;
    /// msg.finish()?;
    /// w.end_interchange()?;
    /// assert_eq!(
    ///     w.finish()?,
    ///     b"UNB+UNOC:3+SENDER+RECEIVER+260101:0900+IC1'UNH+1+ORDERS:D:96A:UN'BGM+220'UNT+3+1'UNZ+1+IC1'".to_vec(),
    /// );
    /// # Ok::<(), edifact::EdifactError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Refused before the first byte, the writer staying usable:
    ///
    /// - [`EdifactError::InvalidEventSequence`] when an interchange is open.
    /// - The header's own findings, as [`InterchangeHeader::to_segment`]
    ///   reports them (`E008`, `E021`, `E048`–`E050`, `E060`).
    /// - [`EdifactError::CharacterRepertoireMismatch`] when the writer is bound
    ///   to another repertoire, [`EdifactError::CharacterNotInRepertoire`] for a
    ///   header value the repertoire cannot carry.
    /// - [`EdifactError::InvalidUna`] for versions 1–3 while a repetition
    ///   separator is active (version 4 introduced it).
    ///
    /// A write failure poisons the writer.
    pub fn begin_interchange(&mut self, header: &InterchangeHeader) -> Result<(), EdifactError> {
        self.check_healthy()?;
        if self.interchange.is_some() {
            return Err(EdifactError::InvalidEventSequence {
                message: "begin_interchange while an interchange is open; call end_interchange first",
            });
        }
        let unb = header.to_segment()?;
        let charset = header.charset;
        if let Some(bound) = self.charset
            && bound != charset
        {
            return Err(EdifactError::CharacterRepertoireMismatch {
                declared: charset.syntax_identifier().to_owned(),
                writer: bound.syntax_identifier(),
            });
        }
        if header.syntax_version != SyntaxVersion::V4 && self.ssa.is_repetition_active() {
            return Err(EdifactError::InvalidUna);
        }
        check_repertoire(Some(charset), &unb)?;
        self.charset = Some(charset);
        self.write_segment(&unb)?;
        self.interchange = Some(OpenInterchange {
            control_ref: header.control_ref.clone(),
            version: header.syntax_version,
            messages: 0,
            groups: 0,
            ungrouped_messages: 0,
            in_group: false,
            group_messages: 0,
            in_message: false,
        });
        Ok(())
    }

    /// Close the open interchange: write `UNZ` with the computed count.
    ///
    /// DE 0036 is the number of groups when any `UNG` was written, else the
    /// number of messages (ISO 9735-10:2002 DE 0036); DE 0020 repeats the
    /// header's.
    ///
    /// # Errors
    ///
    /// Refused before the first byte, the writer staying usable:
    /// [`EdifactError::InvalidEventSequence`] when no interchange is open or a
    /// message is still open, [`EdifactError::EmptyInterchange`] for no message,
    /// [`EdifactError::GroupsAndMessagesMixed`] when grouped and ungrouped
    /// messages were written, [`EdifactError::DataElementTooLong`] when the
    /// count exceeds DE 0036 for the version. A poisoned writer returns
    /// [`EdifactError::WriterPoisoned`]; a write failure poisons it.
    pub fn end_interchange(&mut self) -> Result<(), EdifactError> {
        self.check_healthy()?;
        let Some(open) = &self.interchange else {
            return Err(EdifactError::InvalidEventSequence {
                message: "end_interchange without begin_interchange",
            });
        };
        if open.in_message {
            return Err(EdifactError::InvalidEventSequence {
                message: "end_interchange inside a message; write its UNT first",
            });
        }
        let count = if open.groups > 0 {
            open.groups
        } else {
            open.messages
        };
        if count == 0 {
            return Err(EdifactError::EmptyInterchange {
                control_ref: open.control_ref.clone(),
            });
        }
        if open.groups > 0 && open.ungrouped_messages > 0 {
            return Err(EdifactError::GroupsAndMessagesMixed {
                span: crate::model::Span::default(),
            });
        }
        let unz = Segment::new(
            "UNZ",
            vec![
                Element::of(&[count.to_string()]),
                Element::of(std::slice::from_ref(&open.control_ref)),
            ],
        );
        let mut findings = Vec::new();
        crate::directory_validator::check_service_segment(
            &unz,
            Some(open.version.number()),
            &mut findings,
        );
        if let Some(first) = findings.into_iter().next() {
            return Err(first);
        }
        self.interchange = None;
        self.write_segment(&unz)
    }

    /// Append one checked, encoded message verbatim and count it.
    pub(crate) fn write_verbatim_message(
        &mut self,
        bytes: &[u8],
        segments: u64,
    ) -> Result<(), EdifactError> {
        self.check_healthy()?;
        self.write_body(|w| w.inner.write_all(bytes).map_err(Into::into))?;
        self.count("UNH");
        self.segment_count += segments.saturating_sub(2);
        self.count("UNT");
        Ok(())
    }

    /// Set the open interchange's message count (tests of DE 0036 overflow).
    #[cfg(test)]
    fn set_message_count(&mut self, messages: u64) {
        if let Some(open) = &mut self.interchange {
            open.messages = messages;
            open.ungrouped_messages = messages;
        }
    }

    /// Write a `UNH` message header and return a [`MessageWriter`] guard.
    ///
    /// S009 carries the association assigned code (DE 0057) when `id` has one.
    /// The guard tracks the per-message segment count. [`MessageWriter::finish`]
    /// writes the matching `UNT` and is the only way to close the message:
    /// dropping the guard without it writes nothing and poisons this writer.
    ///
    /// # Errors
    ///
    /// Returns [`EdifactError`] if writing the `UNH` segment fails.
    pub fn begin_message<'w>(
        &'w mut self,
        id: MessageIdentifier<'_>,
    ) -> Result<MessageWriter<'w, W>, EdifactError> {
        self.write_composites("UNH", &unh_elements(&id))?;
        // Capture `segment_count` after writing UNH so `MessageWriter` knows
        // the absolute count that includes UNH.
        let unh_count = self.segment_count;
        Ok(MessageWriter {
            writer: self,
            message_ref: id.message_ref.to_owned(),
            unh_count,
            done: false,
        })
    }
}

/// `UNH` DE 0062 and S009 for `id`.
fn unh_elements<'a>(id: &MessageIdentifier<'a>) -> [Vec<&'a str>; 2] {
    [
        vec![id.message_ref],
        vec![
            id.message_type,
            id.version,
            id.release,
            id.controlling_agency,
            id.association_code,
        ],
    ]
}

/// Refuse a repeating element when `ssa` declares no repetition separator.
#[inline]
fn check_repetition(ssa: &ServiceStringAdvice, seg: &Segment<'_>) -> Result<(), EdifactError> {
    // The sentinel at UNA position 7 is a space.  Emitting it as a separator
    // would produce output that reads back as a single occurrence whose value
    // contains a space — corrupt, and quietly so.
    if !ssa.is_repetition_active() && seg.elements.iter().any(|e| !e.repeats.is_empty()) {
        return Err(EdifactError::RepetitionSeparatorNotDeclared);
    }
    Ok(())
}

/// Refuse a value of `seg` that `charset` cannot carry.
fn check_repertoire(charset: Option<Charset>, seg: &Segment<'_>) -> Result<(), EdifactError> {
    let Some(charset) = charset else {
        return Ok(());
    };
    for element in &seg.elements {
        for components in element.repetitions() {
            for (value, _) in components {
                charset.encode(value)?;
            }
        }
    }
    Ok(())
}

/// `seg` as it reads back after writing: trailing empty components,
/// occurrences and data elements removed (ISO 9735-1:2002 §8.7).
pub(crate) fn canonical(mut seg: OwnedSegment) -> OwnedSegment {
    fn trim(components: &mut Components<'_>) {
        while components.len() > 1 && components.last().is_some_and(|(v, _)| v.is_empty()) {
            components.pop();
        }
        if components.is_empty() {
            components.push((Cow::Borrowed(""), crate::model::Span::default()));
        }
    }
    let empty = |components: &Components<'_>| components.iter().all(|(v, _)| v.is_empty());
    for element in &mut seg.elements {
        trim(&mut element.components);
        element.repeats.iter_mut().for_each(trim);
        while element.repeats.last().is_some_and(empty) {
            element.repeats.pop();
        }
    }
    while seg
        .elements
        .last()
        .is_some_and(|e| e.repeats.is_empty() && empty(&e.components))
    {
        seg.elements.pop();
    }
    seg
}

/// Guard for one EDIFACT message within an interchange.
///
/// Obtained from [`Writer::begin_message`], which writes `UNH`.
/// [`finish`](Self::finish) writes `UNT` with the per-message segment count and
/// is the only way to close the message.
///
/// Dropping the guard without a successful `finish` — an early `?`, a panic, or
/// a forgotten call — writes nothing and poisons the [`Writer`]: the output
/// keeps a `UNH` without `UNT`, which no envelope check accepts, and the next
/// writer call returns [`EdifactError::WriterPoisoned`].
///
/// # Example
///
/// ```rust
/// use edifact::{MessageIdentifier, Writer};
///
/// let mut writer = Writer::new(Vec::new());
/// let mut msg = writer.begin_message(MessageIdentifier::new("1", "ORDERS", "D", "96A", "UN"))?;
/// msg.write_simple("BGM", &["220", "PO001", "9"])?;
/// msg.finish()?;
/// assert_eq!(writer.finish()?, b"UNH+1+ORDERS:D:96A:UN'BGM+220+PO001+9'UNT+3+1'".to_vec());
/// # Ok::<(), edifact::EdifactError>(())
/// ```
///
/// Discarding the guard is a compile-time warning:
///
/// ```rust,compile_fail
/// #![deny(unused_must_use)]
/// # fn main() -> Result<(), edifact::EdifactError> {
/// let mut writer = edifact::Writer::new(Vec::new());
/// writer.begin_message(edifact::MessageIdentifier::new("1", "ORDERS", "D", "96A", "UN"))?;
/// # Ok(())
/// # }
/// ```
#[must_use = "a message is closed only by finish(); dropping it abandons the message and poisons the writer"]
pub struct MessageWriter<'w, W: Write> {
    writer: &'w mut Writer<W>,
    message_ref: String,
    /// Absolute segment count immediately after `UNH` was written.
    unh_count: u64,
    /// Set by `finish()` before it writes `UNT`, so no path writes it twice
    /// and `Drop` leaves a finished or failed message alone.
    done: bool,
}

impl<W: Write> MessageWriter<'_, W> {
    /// Write an all-simple segment within this message.
    ///
    /// Delegates to [`Writer::write_simple`].
    ///
    /// # Errors
    ///
    /// Returns [`EdifactError`] if the underlying writer fails.
    pub fn write_simple<S: AsRef<str>>(
        &mut self,
        tag: &str,
        elements: &[S],
    ) -> Result<(), EdifactError> {
        self.writer.write_simple(tag, elements)
    }

    /// Write an all-composite segment within this message.
    ///
    /// Delegates to [`Writer::write_composites`].
    ///
    /// # Errors
    ///
    /// Returns [`EdifactError`] if the underlying writer fails.
    pub fn write_composites<E, S>(&mut self, tag: &str, elements: &[E]) -> Result<(), EdifactError>
    where
        E: AsRef<[S]>,
        S: AsRef<str>,
    {
        self.writer.write_composites(tag, elements)
    }

    /// Write a segment mixing simple and composite data elements within this message.
    ///
    /// Delegates to [`Writer::write_elements`].
    ///
    /// # Errors
    ///
    /// Returns [`EdifactError`] if the underlying writer fails.
    pub fn write_elements(
        &mut self,
        tag: &str,
        elements: &[DataElement<'_>],
    ) -> Result<(), EdifactError> {
        self.writer.write_elements(tag, elements)
    }

    /// Write a fully-typed segment within this message.
    ///
    /// Delegates to [`Writer::write_segment`].
    ///
    /// # Errors
    ///
    /// Returns [`EdifactError`] if the underlying writer fails.
    pub fn write_segment(&mut self, seg: &Segment<'_>) -> Result<(), EdifactError> {
        self.writer.write_segment(seg)
    }

    /// Write `UNT` and close the message, consuming the guard.
    ///
    /// The count written into `UNT` DE 0074 includes `UNH`, all content segments,
    /// and `UNT` itself — matching the EDIFACT standard.
    ///
    /// # Errors
    ///
    /// [`EdifactError::WriterPoisoned`] when the writer is poisoned. A failed
    /// `UNT` write poisons the writer; it is never retried.
    pub fn finish(mut self) -> Result<(), EdifactError> {
        self.done = true;
        // Segments since UNH: writer.segment_count - unh_count (content only).
        // Total = 1 (UNH) + content + 1 (UNT) = content + 2.
        let count = self.writer.segment_count - self.unh_count + 2;
        let count_str = count.to_string();
        self.writer.write_composites(
            "UNT",
            &[&[count_str.as_str()], &[self.message_ref.as_str()]],
        )
    }
}

impl<W: Write> Drop for MessageWriter<'_, W> {
    /// Abandon an unfinished message: write nothing, poison the writer.
    fn drop(&mut self) {
        if !self.done {
            self.writer.poison(MESSAGE_ABANDONED);
        }
    }
}

// ── MessageBuilder ────────────────────────────────────────────────────────────

/// Build one message as owned segments, with `UNT` computed.
///
/// The segment methods are [`MessageWriter`]'s and refuse what [`Writer`]
/// refuses ([`EdifactError::InvalidSegmentTag`],
/// [`EdifactError::RepetitionSeparatorNotDeclared`],
/// [`EdifactError::CharacterNotInRepertoire`]) before appending, so a refused
/// call changes nothing. [`finish`](Self::finish) returns `UNH`, the body and
/// `UNT`; a builder dropped without it produces nothing. Rendering the
/// segments with [`Writer::write_segment`] gives the bytes [`MessageWriter`]
/// writes for the same calls.
///
/// ```
/// use edifact::{MessageBuilder, MessageIdentifier};
///
/// let id = MessageIdentifier::new("1", "UTILMD", "D", "11A", "UN").with_association_code("S2.1");
/// let mut msg = MessageBuilder::new(id)?;
/// msg.write_simple("BGM", &["E01", "DOC-1"])?;
/// let segments = msg.finish()?;
/// assert_eq!(
///     edifact::segments_to_bytes(&segments)?,
///     b"UNH+1+UTILMD:D:11A:UN:S2.1'BGM+E01+DOC-1'UNT+3+1'".to_vec(),
/// );
/// # Ok::<(), edifact::EdifactError>(())
/// ```
#[must_use = "a message exists only after finish()"]
#[derive(Debug, Clone)]
pub struct MessageBuilder {
    ssa: ServiceStringAdvice,
    charset: Option<Charset>,
    message_ref: String,
    segments: Vec<OwnedSegment>,
}

impl MessageBuilder {
    /// Start a message: `UNH` from `id`.
    ///
    /// # Errors
    ///
    /// None today for the default configuration; the signature leaves room for
    /// the checks every segment gets.
    pub fn new(id: MessageIdentifier<'_>) -> Result<Self, EdifactError> {
        let [reference, s009] = unh_elements(&id);
        let unh = canonical(Segment::new(
            "UNH",
            vec![
                Element::of(&reference).into_owned(),
                Element::of(&s009).into_owned(),
            ],
        ));
        Ok(Self {
            ssa: ServiceStringAdvice::default(),
            charset: None,
            message_ref: id.message_ref.to_owned(),
            segments: vec![unh],
        })
    }

    /// Refuse values `charset` cannot carry, as a bound [`Writer`] does.
    pub fn with_charset(mut self, charset: Charset) -> Self {
        self.charset = Some(charset);
        self
    }

    /// Use `ssa`'s repetition rule ([`Writer::with_service_string_advice`]).
    ///
    /// # Errors
    ///
    /// [`EdifactError::InvalidUna`] when `ssa` is not valid.
    pub fn with_service_string_advice(
        mut self,
        ssa: ServiceStringAdvice,
    ) -> Result<Self, EdifactError> {
        if !ssa.is_valid() {
            return Err(EdifactError::InvalidUna);
        }
        self.ssa = ssa;
        Ok(self)
    }

    /// Append `segment` after the writer's refusals.
    fn push(&mut self, segment: OwnedSegment) -> Result<(), EdifactError> {
        check_repetition(&self.ssa, &segment)?;
        check_tag(segment.tag())?;
        check_repertoire(self.charset, &segment)?;
        self.segments.push(canonical(segment));
        Ok(())
    }

    /// Append an all-simple segment ([`Writer::write_simple`]).
    ///
    /// # Errors
    ///
    /// The refusals of [`MessageBuilder`]; nothing is appended.
    pub fn write_simple<S: AsRef<str>>(
        &mut self,
        tag: &str,
        elements: &[S],
    ) -> Result<(), EdifactError> {
        let elements = elements
            .iter()
            .map(|e| Element::of(&[e.as_ref().to_owned()]))
            .collect();
        self.push(Segment::new(tag.to_owned(), elements))
    }

    /// Append an all-composite segment ([`Writer::write_composites`]).
    ///
    /// # Errors
    ///
    /// The refusals of [`MessageBuilder`]; nothing is appended.
    pub fn write_composites<E, S>(&mut self, tag: &str, elements: &[E]) -> Result<(), EdifactError>
    where
        E: AsRef<[S]>,
        S: AsRef<str>,
    {
        let elements = elements
            .iter()
            .map(|e| {
                let components: Vec<String> =
                    e.as_ref().iter().map(|c| c.as_ref().to_owned()).collect();
                Element::of(&components)
            })
            .collect();
        self.push(Segment::new(tag.to_owned(), elements))
    }

    /// Append a mixed simple/composite segment ([`Writer::write_elements`]).
    ///
    /// # Errors
    ///
    /// The refusals of [`MessageBuilder`]; nothing is appended.
    pub fn write_elements(
        &mut self,
        tag: &str,
        elements: &[DataElement<'_>],
    ) -> Result<(), EdifactError> {
        let elements = elements
            .iter()
            .map(|e| Element::of(e.components()).into_owned())
            .collect();
        self.push(Segment::new(tag.to_owned(), elements))
    }

    /// Append a copy of `seg`, repetitions included ([`Writer::write_segment`]).
    ///
    /// # Errors
    ///
    /// The refusals of [`MessageBuilder`]; nothing is appended.
    pub fn write_segment(&mut self, seg: &Segment<'_>) -> Result<(), EdifactError> {
        let mut owned = seg.clone().into_owned();
        owned.span = crate::model::Span::default();
        owned.tag_span = crate::model::Span::default();
        for element in &mut owned.elements {
            element.span = crate::model::Span::default();
            for (_, span) in element
                .components
                .iter_mut()
                .chain(element.repeats.iter_mut().flatten())
            {
                *span = crate::model::Span::default();
            }
        }
        self.push(owned)
    }

    /// Segments so far, `UNH` included.
    #[must_use]
    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }

    /// Close the message: append `UNT` with the segment count and reference.
    ///
    /// # Errors
    ///
    /// [`EdifactError::EmptyMessage`] when no segment follows `UNH`.
    pub fn finish(mut self) -> Result<Vec<OwnedSegment>, EdifactError> {
        if self.segments.len() < 2 {
            return Err(EdifactError::EmptyMessage {
                message_ref: self.message_ref,
                span: crate::model::Span::default(),
            });
        }
        let count = (self.segments.len() + 1).to_string();
        self.segments.push(Segment::new(
            "UNT",
            vec![Element::of(&[count]), Element::of(&[self.message_ref])],
        ));
        Ok(self.segments)
    }
}

// ── InterchangeBuilder ────────────────────────────────────────────────────────

/// One message handed to [`InterchangeBuilder`].
#[derive(Debug, Clone)]
enum Message {
    Text(String),
    Segments(Vec<OwnedSegment>),
}

/// Build a complete interchange: `UNB`, the messages, and a computed `UNZ`.
///
/// Returns bytes that pass [`validate_envelope`](crate::validate_envelope), or
/// an error and no bytes. A `UNA` is written exactly when the service
/// characters differ from the version's defaults (ISO 9735-1:2002 §5.3).
///
/// ```
/// use edifact::{Charset, InterchangeBuilder, InterchangeHeader, MessageBuilder, MessageIdentifier,
///               Party, PreparationDateTime, SyntaxVersion};
///
/// let header = InterchangeHeader::new(Charset::UnoC, SyntaxVersion::V3,
///     Party::new("SENDER").with_qualifier("14"), Party::new("RECEIVER").with_qualifier("14"),
///     PreparationDateTime::new(2026, 8, 2, 9, 15)?, "REF001");
/// let mut msg = MessageBuilder::new(MessageIdentifier::new("1", "ORDERS", "D", "96A", "UN"))?;
/// msg.write_simple("BGM", &["220", "PO-1", "9"])?;
/// let bytes = InterchangeBuilder::new(header)
///     .message_segments(msg.finish()?)
///     .message("UNH+2+ORDERS:D:96A:UN'BGM+220+PO-2+9'UNT+3+2'")
///     .build()?;
/// assert!(bytes.starts_with(b"UNB+UNOC:3+SENDER:14+RECEIVER:14+260802:0915+REF001'"));
/// assert!(bytes.ends_with(b"UNZ+2+REF001'"));
/// # Ok::<(), edifact::EdifactError>(())
/// ```
#[must_use = "nothing is produced until build()"]
#[derive(Debug, Clone)]
pub struct InterchangeBuilder {
    header: InterchangeHeader,
    ssa: Option<ServiceStringAdvice>,
    messages: Vec<Message>,
}

impl InterchangeBuilder {
    /// A builder for an interchange under `header`.
    pub fn new(header: InterchangeHeader) -> Self {
        Self {
            header,
            ssa: None,
            messages: Vec::new(),
        }
    }

    /// Use `ssa` as the service characters (default: the version's).
    pub fn with_service_string_advice(mut self, ssa: ServiceStringAdvice) -> Self {
        self.ssa = Some(ssa);
        self
    }

    /// Add a message as UTF-8 text: exactly one `UNH`…`UNT`, written with the
    /// interchange's service characters. ASCII whitespace around it is
    /// ignored and not written.
    ///
    /// The text is checked like the envelope checks a message, encoded strictly
    /// into the header's repertoire, and copied verbatim.
    pub fn message(mut self, text: impl Into<String>) -> Self {
        self.messages.push(Message::Text(text.into()));
        self
    }

    /// Add a message as owned segments, `UNH` to `UNT`, such as
    /// [`MessageBuilder::finish`] returns.
    pub fn message_segments(mut self, segments: Vec<OwnedSegment>) -> Self {
        self.messages.push(Message::Segments(segments));
        self
    }

    /// Render the interchange.
    ///
    /// # Errors
    ///
    /// Every refusal of [`Writer::begin_interchange`] and
    /// [`Writer::end_interchange`] (no message: [`EdifactError::EmptyInterchange`]),
    /// and for each message: [`EdifactError::MissingSegment`] without `UNH`
    /// first or `UNT` last, [`EdifactError::InvalidSegmentForMessage`] for
    /// another service segment inside or bytes other than ASCII whitespace
    /// outside it,
    /// [`EdifactError::EmptyMessage`], [`EdifactError::SegmentCountMismatch`],
    /// [`EdifactError::QualifierMismatch`] for the `UNT` reference,
    /// [`EdifactError::DuplicateReference`], the `UNH`/`UNT` representation
    /// findings for the version, any parse error of a text, and
    /// [`EdifactError::CharacterNotInRepertoire`] naming the character and its
    /// byte offset in the text.
    pub fn build(self) -> Result<Vec<u8>, EdifactError> {
        let version = self.header.syntax_version;
        let default_ssa = ServiceStringAdvice::for_syntax_version(Some(version.number()));
        let ssa = self.ssa.unwrap_or(default_ssa);
        let mut writer = if ssa == default_ssa {
            Writer::with_service_string_advice(Vec::new(), ssa)?
        } else {
            Writer::with_una(Vec::new(), ssa)?
        };
        writer.begin_interchange(&self.header)?;
        let charset = self.header.charset;
        let mut references = HashSet::new();
        for message in &self.messages {
            match message {
                Message::Segments(segments) => {
                    check_one_message(segments, version, &mut references)?;
                    for segment in segments {
                        writer.write_segment(segment)?;
                    }
                }
                Message::Text(text) => {
                    let config = ReaderConfig::default().with_service_string_advice(ssa);
                    let segments = crate::from_bytes_with_config(text.as_bytes(), config)
                        .collect::<Result<Vec<_>, _>>()?;
                    let range = check_text_bounds(text.as_bytes(), &segments)?;
                    check_one_message(&segments, version, &mut references)?;
                    let encoded = charset.encode(&text[range.clone()]).map_err(|e| match e {
                        EdifactError::CharacterNotInRepertoire {
                            charset,
                            character,
                            offset,
                        } => EdifactError::CharacterNotInRepertoire {
                            charset,
                            character,
                            offset: offset + range.start,
                        },
                        other => other,
                    })?;
                    writer.write_verbatim_message(&encoded, segments.len() as u64)?;
                }
            }
        }
        writer.end_interchange()?;
        writer.finish()
    }
}

/// The range of the message in `text`: ASCII whitespace before its `UNH` and
/// after its `UNT` is left out; any other byte there is refused.
fn check_text_bounds(
    text: &[u8],
    segments: &[Segment<'_>],
) -> Result<std::ops::Range<usize>, EdifactError> {
    let outside = |tag: &str, span| EdifactError::InvalidSegmentForMessage {
        tag: tag.to_owned(),
        message_type: "message text (UNH first, UNT last, nothing around them)".to_owned(),
        span,
    };
    let blank = |bytes: &[u8]| {
        bytes
            .iter()
            .all(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
    };
    let (Some(first), Some(last)) = (segments.first(), segments.last()) else {
        // Nothing but whitespace: the message check reports the missing `UNH`.
        return Ok(0..0);
    };
    if first.tag() != "UNH" || !blank(&text[..first.span.start]) {
        let lead = crate::tokenizer::prologue_len(text);
        let tag = if text[lead..].starts_with(b"UNA") {
            "UNA"
        } else {
            "UNH"
        };
        return Err(outside(
            tag,
            crate::model::Span::new(0, lead.max(1).min(text.len())),
        ));
    }
    if !blank(&text[last.span.end..]) {
        return Err(outside(
            last.tag(),
            crate::model::Span::new(last.span.end, text.len()),
        ));
    }
    Ok(first.span.start..last.span.end)
}

/// Refuse `segments` unless they are exactly one message with a body,
/// consistent `UNT`, valid `UNH`/`UNT` for `version` and a reference not in
/// `references`, which then gains it.
pub(crate) fn check_one_message(
    segments: &[Segment<'_>],
    version: SyntaxVersion,
    references: &mut HashSet<String>,
) -> Result<(), EdifactError> {
    let missing = |tag: &str, position: &str| EdifactError::MissingSegment {
        tag: tag.to_owned(),
        expected_position: position.to_owned(),
    };
    let (Some(unh), Some(unt)) = (segments.first(), segments.last()) else {
        return Err(missing("UNH", "first segment of message"));
    };
    if unh.tag() != "UNH" {
        return Err(missing("UNH", "first segment of message"));
    }
    let message_type = unh.component_str(1, 0).unwrap_or("").to_owned();
    let last = segments.len() - 1;
    for (index, seg) in segments.iter().enumerate().skip(1) {
        let tag = seg.tag();
        let trailer = index == last && tag == "UNT";
        if !trailer && matches!(tag, "UNA" | "UNB" | "UNZ" | "UNG" | "UNE" | "UNH" | "UNT") {
            return Err(EdifactError::InvalidSegmentForMessage {
                tag: tag.to_owned(),
                message_type,
                span: seg.span,
            });
        }
        if let Some(refused) = crate::envelope::classify_u_tag(tag, seg.span) {
            return Err(refused);
        }
    }
    if segments.len() < 2 || unt.tag() != "UNT" {
        return Err(missing("UNT", "last segment of message"));
    }
    let message_ref = unh.component_str(0, 0).unwrap_or("").to_owned();
    if segments.len() < 3 {
        return Err(EdifactError::EmptyMessage {
            message_ref,
            span: unh.span,
        });
    }
    let mut findings = Vec::new();
    for seg in [unh, unt] {
        crate::directory_validator::check_service_segment(
            seg,
            Some(version.number()),
            &mut findings,
        );
    }
    if let Some(first) = findings.into_iter().next() {
        return Err(first);
    }
    let declared = unt.component_str(0, 0).unwrap_or("");
    let actual = segments.len();
    if declared.parse::<usize>().ok() != Some(actual) {
        return Err(EdifactError::SegmentCountMismatch {
            expected: declared.parse().unwrap_or(u64::MAX),
            actual: actual as u64,
            message_ref,
            span: unt.span,
        });
    }
    let unt_ref = unt.component_str(1, 0).unwrap_or("");
    if unt_ref != message_ref {
        return Err(EdifactError::QualifierMismatch {
            tag: "UNT".to_owned(),
            actual: unt_ref.to_owned(),
            expected: message_ref,
            span: unt.span,
        });
    }
    if !references.insert(message_ref.clone()) {
        return Err(EdifactError::DuplicateReference {
            tag: "UNH".to_owned(),
            reference: message_ref,
            span: unh.span,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `UNB` header from its strings, as `InterchangeHeader::parse` reads them.
    fn header(
        syntax: &str,
        version: &str,
        sender: &str,
        recipient: &str,
        date: &str,
        time: &str,
        reference: &str,
    ) -> crate::InterchangeHeader {
        let raw = format!("UNB+{syntax}:{version}+{sender}+{recipient}+{date}:{time}+{reference}'");
        let segments: Vec<_> = crate::from_bytes(raw.as_bytes())
            .collect::<Result<_, _>>()
            .expect("header parses");
        crate::InterchangeHeader::parse(&segments[0]).expect("header is valid")
    }
    use crate::model::Element;

    /// A non-default UNA whose delimiters share no byte with the defaults.
    fn exotic_ssa() -> ServiceStringAdvice {
        ServiceStringAdvice {
            component_sep: b'|',
            element_sep: b'!',
            decimal_mark: b',',
            release_char: b'#',
            repetition_sep: b'*',
            segment_term: b'~',
        }
    }

    #[test]
    fn unh_composite_uses_the_active_component_separator() {
        // S009 must be built as a real composite: a literal `:` in a
        // `format!` collapses into one component under a custom UNA.
        let mut buf = Vec::new();
        {
            let mut w = Writer::with_una(&mut buf, exotic_ssa()).unwrap();
            let msg = w
                .begin_message(MessageIdentifier::new("1", "ORDERS", "D", "96A", "UN"))
                .expect("UNH");
            msg.finish().expect("UNT");
        }
        let out = String::from_utf8(buf).unwrap();
        assert!(
            out.contains("UNH!1!ORDERS|D|96A|UN~"),
            "S009 must use `|`, got {out}"
        );
    }

    #[test]
    fn round_trips_through_a_custom_una() {
        // The library must be able to re-read its own output verbatim.
        let mut buf = Vec::new();
        {
            let mut w = Writer::with_una(&mut buf, exotic_ssa()).unwrap();
            w.begin_interchange(&header(
                "UNOA", "4", "SENDER", "RECEIVER", "20200101", "0900", "IC1",
            ))
            .unwrap();
            let mut msg = w
                .begin_message(MessageIdentifier::new("1", "ORDERS", "D", "96A", "UN"))
                .unwrap();
            msg.write_simple("BGM", &["220"]).unwrap();
            msg.finish().unwrap();
            w.end_interchange().unwrap();
        }
        let segs: Vec<_> = crate::from_bytes(&buf)
            .collect::<Result<Vec<_>, _>>()
            .expect("own output must reparse");
        let unh = segs.iter().find(|s| s.tag == "UNH").unwrap();
        assert_eq!(unh.get_element(1).unwrap().get_component(0), Some("ORDERS"));
        assert_eq!(unh.get_element(1).unwrap().get_component(2), Some("96A"));
        crate::validate_envelope(&segs).expect("own output must pass envelope validation");
    }

    #[test]
    fn finish_unt_counts_only_the_current_message() {
        // `finish_unt` used the writer-lifetime segment total, so a preceding
        // UNB inflated DE 0074 and the interchange failed its own validation.
        let mut buf = Vec::new();
        {
            let mut w = Writer::new(&mut buf);
            w.begin_interchange(&header("UNOA", "1", "S", "R", "200101", "0900", "IC1"))
                .unwrap();
            w.write_composites("UNH", &[&["1"][..], &["ORDERS", "D", "96A", "UN"][..]])
                .unwrap();
            w.write_simple("BGM", &["220"]).unwrap();
            w.finish_unt("1").unwrap();
        }
        let out = String::from_utf8(buf).unwrap();
        // UNH + BGM + UNT == 3
        assert!(out.contains("UNT+3+1'"), "expected UNT+3, got {out}");
    }

    #[test]
    fn repetition_separator_is_escaped_when_declared() {
        let mut buf = Vec::new();
        {
            let mut w = Writer::with_una(
                &mut buf,
                ServiceStringAdvice {
                    repetition_sep: b'*',
                    ..ServiceStringAdvice::default()
                },
            )
            .unwrap();
            w.write_composites("FTX", &[&["a*b"]]).unwrap();
        }
        let out = String::from_utf8(buf).unwrap();
        assert!(out.ends_with("FTX+a?*b'"), "rep-sep unescaped in {out}");
    }

    #[test]
    fn repetition_separator_sentinel_is_not_escaped() {
        // Space at UNA position 7 means "not used" and must never be escaped.
        let w = Writer::new(std::io::sink());
        assert_eq!(w.escape_value("a b"), "a b");
    }

    #[test]
    fn write_composites_escapes_a_literal_component_separator() {
        let mut buf = Vec::new();
        {
            let mut w = Writer::new(&mut buf);
            w.write_composites("NAD", &[&["MS"], &["ACME:INC"]])
                .unwrap();
        }
        let segs: Vec<_> = crate::from_bytes(&buf)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        // The `:` stays inside the value instead of splitting the element.
        assert_eq!(
            segs[0].get_element(1).unwrap().get_component(0),
            Some("ACME:INC")
        );
    }

    #[test]
    fn write_elements_mixes_simple_and_composite() {
        let mut buf = Vec::new();
        {
            let mut w = Writer::new(&mut buf);
            w.write_elements(
                "NAD",
                &[
                    DataElement::Simple("MS"),
                    DataElement::Composite(&["9900112233445", "", "293"]),
                ],
            )
            .unwrap();
        }
        assert_eq!(buf, b"NAD+MS+9900112233445::293'");
    }

    #[test]
    fn elements_macro_matches_the_explicit_form() {
        let mut macro_buf = Vec::new();
        {
            let mut w = Writer::new(&mut macro_buf);
            w.write_elements("NAD", elements!["MS", ["ACME", "", "9"]])
                .unwrap();
            w.write_elements("DTM", elements![["137", "20260101", "102"]])
                .unwrap();
        }
        let mut explicit_buf = Vec::new();
        {
            let mut w = Writer::new(&mut explicit_buf);
            w.write_elements(
                "NAD",
                &[
                    DataElement::Simple("MS"),
                    DataElement::Composite(&["ACME", "", "9"]),
                ],
            )
            .unwrap();
            w.write_elements(
                "DTM",
                &[DataElement::Composite(&["137", "20260101", "102"])],
            )
            .unwrap();
        }
        assert_eq!(macro_buf, explicit_buf);
        assert_eq!(macro_buf, b"NAD+MS+ACME::9'DTM+137:20260101:102'");
    }

    #[test]
    fn elements_macro_accepts_arbitrary_expressions() {
        // Builders emit runtime values, not literals.  A `tt`-based macro only
        // matched single-token entries, so `qualifier.as_str()` failed to parse
        // — which is precisely the shape this macro exists for.
        let qualifier = String::from("MS");
        let gln = "9900112233445";
        let dtm: Vec<&str> = vec!["137", "20260101", "102"];

        let mut buf = Vec::new();
        {
            let mut w = Writer::new(&mut buf);
            w.write_elements("NAD", elements![qualifier.as_str(), [gln, "", "293"]])
                .unwrap();
            w.write_elements("DTM", elements![dtm]).unwrap();
            w.write_elements("FTX", elements![qualifier]).unwrap();
            w.write_elements("UNS", elements![]).unwrap();
        }
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "NAD+MS+9900112233445::293'DTM+137:20260101:102'FTX+MS'UNS'"
        );
    }

    #[test]
    fn write_elements_escapes_a_literal_component_separator() {
        // The `:` stays inside the value instead of splitting the element —
        // the failure mode of pre-joining components into one string.
        let mut buf = Vec::new();
        {
            let mut w = Writer::new(&mut buf);
            w.write_elements(
                "NAD",
                &[DataElement::Simple("MS"), DataElement::Simple("ACME:INC")],
            )
            .unwrap();
        }
        let segs: Vec<_> = crate::from_bytes(&buf)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            segs[0].get_element(1).unwrap().get_component(0),
            Some("ACME:INC")
        );
    }

    #[test]
    fn write_elements_uses_the_active_component_separator() {
        let mut buf = Vec::new();
        {
            let mut w = Writer::with_una(&mut buf, exotic_ssa()).unwrap();
            w.write_elements("DTM", elements![["137", "20260101", "102"]])
                .unwrap();
        }
        let out = String::from_utf8(buf).unwrap();
        assert!(
            out.ends_with("DTM!137|20260101|102~"),
            "expected custom delimiters, got {out}"
        );
    }

    #[test]
    fn message_writer_counts_write_elements_segments() {
        // `MessageWriter` had no mixed-emit delegate, so callers dropped to the
        // raw writer and their segments escaped the UNT DE 0074 count.
        let mut buf = Vec::new();
        {
            let mut w = Writer::new(&mut buf);
            let mut msg = w
                .begin_message(MessageIdentifier::new("1", "ORDERS", "D", "96A", "UN"))
                .unwrap();
            msg.write_elements("NAD", elements!["MS", ["ACME", "", "9"]])
                .unwrap();
            msg.write_composites("DTM", &[&["137", "20260101", "102"]])
                .unwrap();
            msg.finish().unwrap();
        }
        let out = String::from_utf8(buf).unwrap();
        // UNH + NAD + DTM + UNT == 4
        assert!(out.contains("UNT+4+1'"), "expected UNT+4, got {out}");
    }

    #[test]
    fn write_segment_records_unh_for_the_unt_count() {
        // `write_segment` and `write_composites` did not record the UNH
        // marker, so `finish_unt` fell back to the writer-lifetime total and
        // DE 0074 came out inflated by every preceding interchange segment.
        let mut buf = Vec::new();
        {
            let mut w = Writer::new(&mut buf);
            w.begin_interchange(&header("UNOA", "1", "S", "R", "200101", "0900", "IC1"))
                .unwrap();
            w.write_segment(&Segment::new(
                "UNH",
                vec![
                    Element::of(&["1"]),
                    Element::of(&["ORDERS", "D", "96A", "UN"]),
                ],
            ))
            .unwrap();
            w.write_segment(&Segment::new("BGM", vec![Element::of(&["220"])]))
                .unwrap();
            w.finish_unt("1").unwrap();
        }
        let out = String::from_utf8(buf).unwrap();
        // UNH + BGM + UNT == 3, not 4 (which would count the UNB).
        assert!(out.contains("UNT+3+1'"), "expected UNT+3, got {out}");
    }

    #[test]
    fn write_composites_records_unh_for_the_unt_count() {
        let mut buf = Vec::new();
        {
            let mut w = Writer::new(&mut buf);
            w.begin_interchange(&header("UNOA", "1", "S", "R", "200101", "0900", "IC1"))
                .unwrap();
            w.write_composites(
                "UNH",
                &[
                    vec!["1".to_owned()],
                    vec![
                        "ORDERS".to_owned(),
                        "D".to_owned(),
                        "96A".to_owned(),
                        "UN".to_owned(),
                    ],
                ],
            )
            .unwrap();
            w.write_simple("BGM", &["220"]).unwrap();
            w.finish_unt("1").unwrap();
        }
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("UNT+3+1'"), "expected UNT+3, got {out}");
    }

    #[test]
    fn escape_value_handles_multi_byte_text() {
        // Escaping splits around delimiters; non-ASCII text on either side must
        // stay intact.
        let w = Writer::new(std::io::sink());
        assert_eq!(w.escape_value("Grüße+Köln"), "Grüße?+Köln");
        assert_eq!(w.escape_value("Grüße"), "Grüße");
        assert!(matches!(w.escape_value("plain"), Cow::Borrowed("plain")));
    }

    #[test]
    fn write_and_parse_simple_segment() {
        let segs: Vec<Segment<'static>> = vec![Segment::new(
            "BGM",
            vec![Element::of(&["220"]), Element::of(&["ORDER123"])],
        )];
        let bytes = crate::segments_to_bytes(&segs).unwrap();
        let s = std::str::from_utf8(&bytes).unwrap();
        assert!(s.starts_with("BGM+220+ORDER123'"));
    }

    #[test]
    fn a_tag_the_parser_would_reject_is_never_written() {
        // A tag is emitted verbatim — EDIFACT cannot escape one — so writing an
        // invalid tag produces bytes that do not reparse as the segment they
        // came from.  Every write path must refuse before the first byte.
        for tag in ["bgm", "BGMX", "BG", "", "B+M", "B'M", "BG1", "BGÜ"] {
            let mut writer = Writer::new(Vec::new());
            let err = writer
                .write_simple(tag, &["220"])
                .expect_err("an invalid tag must be refused");
            assert!(
                matches!(err, EdifactError::InvalidSegmentTag(ref t) if t == tag),
                "expected InvalidSegmentTag({tag:?}), got {err:?}",
            );
            // Nothing reached the sink.
            assert!(
                writer.finish().expect("finish").is_empty(),
                "a refused segment must leave the sink untouched",
            );
        }
    }

    #[test]
    fn every_write_path_validates_the_tag() {
        let bad = "bgm";
        macro_rules! refused {
            ($call:expr) => {
                assert!(
                    matches!($call, Err(EdifactError::InvalidSegmentTag(_))),
                    "a write path accepted an invalid tag",
                );
            };
        }

        let mut w = Writer::new(Vec::new());
        refused!(w.write_simple(bad, &["1"]));
        refused!(w.write_composites(bad, &[&["1"][..]]));
        refused!(w.write_elements(bad, elements!["1"]));
        refused!(w.write_segment(&Segment::new(bad, vec![Element::of(&["1"])])));
    }

    #[test]
    fn whatever_the_writer_emits_the_parser_reads_back() {
        // The round-trip property the tag check exists to preserve.
        let segments = vec![
            Segment::new(
                "UNB",
                vec![Element::of(&["UNOA", "1"]), Element::of(&["S"])],
            ),
            Segment::new(
                "BGM",
                vec![Element::of(&["220"]), Element::of(&["PO?+1'X"])],
            ),
            Segment::new("UNZ", vec![Element::of(&["0"]), Element::of(&["1"])]),
        ];
        let bytes = crate::segments_to_bytes(&segments).expect("write");
        let reparsed: Vec<_> = crate::from_bytes(&bytes)
            .collect::<Result<Vec<_>, _>>()
            .expect("everything the writer emits must reparse");

        assert_eq!(
            reparsed.iter().map(Segment::tag).collect::<Vec<_>>(),
            ["UNB", "BGM", "UNZ"],
        );
        // Delimiters inside a *value* survive, because a value can be escaped.
        assert_eq!(reparsed[1].element_str(1), Some("PO?+1'X"));
    }

    #[test]
    fn release_char_escaped() {
        let segs: Vec<Segment<'static>> = vec![Segment::new(
            "FTX",
            vec![Element::of(&["value+with+delimiters"])],
        )];
        let bytes = crate::segments_to_bytes(&segs).unwrap();
        let s = std::str::from_utf8(&bytes).unwrap();
        // The `+` in the value must be escaped as `?+`
        assert!(s.contains("?+"), "escape missing: {s}");
    }

    #[test]
    fn round_trip_preserves_values() {
        let segs: Vec<Segment<'static>> = vec![
            Segment::new(
                "UNB",
                vec![
                    Element::of(&["UNOA", "1"]),
                    Element::of(&["SENDER"]),
                    Element::of(&["RECEIVER"]),
                ],
            ),
            Segment::new("UNZ", vec![Element::of(&["0"]), Element::of(&["1"])]),
        ];
        let bytes = crate::segments_to_bytes(&segs).unwrap();
        let rt: Vec<crate::OwnedSegment> = crate::from_reader(std::io::Cursor::new(&bytes))
            .collect::<Result<Vec<_>, _>>()
            .expect("round-trip parse failed");
        assert_eq!(rt[0].tag, "UNB");
        assert_eq!(rt[0].element_str(0), Some("UNOA"));
        assert_eq!(rt[1].tag, "UNZ");
    }

    /// Verify that `Writer::with_una` uses the configured delimiters throughout,
    /// and that `write_composites` (the delimiter-agnostic API) produces correct
    /// component separators even with a non-default UNA.
    #[test]
    fn with_una_non_default_delimiters() {
        use crate::tokenizer::ServiceStringAdvice;

        // Custom UNA: comp_sep=|  elem_sep=!  esc=?  dec_mark=,  rep_sep=*  seg_term=~
        let ssa = ServiceStringAdvice {
            component_sep: b'|',
            element_sep: b'!',
            release_char: b'?',
            decimal_mark: b',',
            repetition_sep: b'*',
            segment_term: b'~',
        };

        let buf = Vec::new();
        let mut writer = Writer::with_una(buf, ssa).expect("writer creation failed");

        // write_composites: pre-split; no hard-coded `:` in element strings
        writer
            .write_composites(
                "BGM",
                &[
                    vec!["220".to_owned(), "SUB1".to_owned()],
                    vec!["PO1".to_owned()],
                ],
            )
            .expect("write failed");

        let out = writer.finish().expect("finish failed");
        let s = std::str::from_utf8(&out).unwrap();

        // Output must use `!` as element separator, `|` as component separator, `~` as terminator.
        // The writer also emits a UNA header when with_una is used.
        assert!(s.contains("BGM"), "BGM segment missing: {s}");
        // Slice after UNA so assertions target segment output, not UNA header bytes.
        let after_una = s.find("BGM").map(|i| &s[i..]).unwrap_or(s);
        assert!(
            after_una.contains('!'),
            "missing element sep in segment: {after_una}"
        );
        assert!(
            after_una.contains('|'),
            "missing component sep in segment: {after_una}"
        );
        assert!(
            after_una.ends_with('~'),
            "missing segment term in segment: {after_una}"
        );
        // Decimal mark appears in the UNA header (no decimal-bearing values in this segment).
        assert!(s.contains(','), "missing decimal mark in UNA: {s}");
        assert!(!s.contains('+'), "default element sep leaked: {s}");
        assert!(!s.contains(':'), "default component sep leaked: {s}");
        // segment_term '~' is not the default; ensure no default ' leaks (UNA itself aside)
        assert!(
            !after_una.contains('\''),
            "default segment term leaked after UNA: {after_una}"
        );
    }

    // ── interchange envelope ────────────────────────────────────────────────

    fn v3() -> InterchangeHeader {
        header("UNOC", "3", "S", "R", "260101", "0900", "IC1")
    }

    /// `call` is refused with `code`; nothing reached the sink and the writer
    /// stays usable.
    fn refused(
        w: &mut Writer<Vec<u8>>,
        code: &str,
        call: impl FnOnce(&mut Writer<Vec<u8>>) -> Result<(), EdifactError>,
    ) {
        let before = w.inner.clone();
        let count = w.segment_count();
        assert_eq!(call(w).unwrap_err().stable_code(), code);
        assert_eq!(w.inner, before);
        assert_eq!(w.segment_count(), count);
        assert!(!w.is_poisoned());
    }

    #[test]
    fn begin_interchange_writes_v3_and_v4_headers() {
        let mut w = Writer::new(Vec::new());
        w.begin_interchange(&v3()).unwrap();
        assert_eq!(w.inner, b"UNB+UNOC:3+S+R+260101:0900+IC1'");
        assert_eq!(w.charset(), Some(Charset::UnoC));
        let v4 = header("UNOC", "4", "S", "R", "20260101", "0900", "IC1");
        let mut w = Writer::new(Vec::new());
        w.begin_interchange(&v4).unwrap();
        assert_eq!(w.inner, b"UNB+UNOC:4+S+R+20260101:0900+IC1'");
    }

    #[test]
    fn envelope_refusals_leave_the_writer_healthy() {
        let mut w = Writer::new(Vec::new());
        refused(&mut w, "E024", |w| w.end_interchange());
        w.begin_interchange(&v3()).unwrap();
        refused(&mut w, "E024", |w| w.begin_interchange(&v3()));
        refused(&mut w, "E042", |w| w.end_interchange());

        let mut bound = Writer::new(Vec::new()).with_charset(Charset::UnoA);
        refused(&mut bound, "E041", |w| w.begin_interchange(&v3()));

        let ssa = ServiceStringAdvice::for_syntax_version(Some(4));
        let mut repeating = Writer::with_service_string_advice(Vec::new(), ssa).unwrap();
        refused(&mut repeating, "E007", |w| w.begin_interchange(&v3()));

        let mut long = v3();
        long.control_ref = "123456789012345".into();
        let mut w = Writer::new(Vec::new());
        refused(&mut w, "E049", |w| w.begin_interchange(&long));

        let mut euro = v3();
        euro.sender.id = "S€".into();
        let mut w = Writer::new(Vec::new());
        refused(&mut w, "E038", |w| w.begin_interchange(&euro));
    }

    #[test]
    fn grouped_and_ungrouped_messages_are_e052() {
        let mut w = Writer::new(Vec::new());
        w.begin_interchange(&v3()).unwrap();
        w.write_simple("UNG", &["ORDERS", "S", "R", "260101:0900", "G1"])
            .unwrap();
        let msg = w
            .begin_message(MessageIdentifier::new("1", "ORDERS", "D", "96A", "UN"))
            .unwrap();
        msg.finish().unwrap();
        w.write_simple("UNE", &["1", "G1"]).unwrap();
        let msg = w
            .begin_message(MessageIdentifier::new("2", "ORDERS", "D", "96A", "UN"))
            .unwrap();
        msg.finish().unwrap();
        refused(&mut w, "E052", |w| w.end_interchange());
    }

    #[test]
    fn a_raw_une_must_count_its_group() {
        let mut w = Writer::new(Vec::new());
        w.begin_interchange(&v3()).unwrap();
        w.write_simple("UNG", &["ORDERS", "S", "R", "260101:0900", "G1"])
            .unwrap();
        let msg = w
            .begin_message(MessageIdentifier::new("1", "ORDERS", "D", "96A", "UN"))
            .unwrap();
        msg.finish().unwrap();
        refused(&mut w, "E054", |w| w.write_simple("UNE", &["2", "G1"]));
        refused(&mut w, "E054", |w| {
            w.write_composites("UNE", &[&["0"], &["G1"]])
        });
        w.write_simple("UNE", &["1", "G1"]).unwrap();
        w.end_interchange().unwrap();
        assert!(w.inner.ends_with(b"UNE+1+G1'UNZ+1+IC1'"));
    }

    #[test]
    fn a_count_beyond_de_0036_is_e049() {
        let mut w = Writer::new(Vec::new());
        w.begin_interchange(&v3()).unwrap();
        w.set_message_count(1_000_000);
        refused(&mut w, "E049", |w| w.end_interchange());
        w.set_message_count(999_999);
        w.end_interchange().unwrap();
        assert!(w.inner.ends_with(b"UNZ+999999+IC1'"));
    }

    #[test]
    fn end_interchange_inside_a_message_is_refused() {
        let mut w = Writer::new(Vec::new());
        w.begin_interchange(&v3()).unwrap();
        w.write_composites("UNH", &[&["1"][..], &["ORDERS", "D", "96A", "UN"][..]])
            .unwrap();
        refused(&mut w, "E024", |w| w.end_interchange());
    }

    #[test]
    fn every_generic_path_refuses_raw_unb_and_unz_in_an_open_interchange() {
        for tag in ["UNB", "UNZ"] {
            let mut w = Writer::new(Vec::new());
            w.begin_interchange(&v3()).unwrap();
            refused(&mut w, "E024", |w| w.write_simple(tag, &["1"]));
            refused(&mut w, "E024", |w| w.write_composites(tag, &[&["1"]]));
            refused(&mut w, "E024", |w| w.write_elements(tag, elements!["1"]));
            refused(&mut w, "E024", |w| {
                w.write_segment(&Segment::new(tag, vec![Element::of(&["1"])]))
            });
            // The event-emitter path.
            refused(&mut w, "E024", |w| w.emit_tag(tag));
        }
        // Outside an interchange the writer is a plain segment writer.
        let mut w = Writer::new(Vec::new());
        w.write_simple("UNZ", &["1", "IC1"]).unwrap();
    }

    #[test]
    fn begin_message_writes_the_association_code() {
        let mut w = Writer::new(Vec::new());
        let id =
            MessageIdentifier::new("1", "UTILMD", "D", "11A", "UN").with_association_code("S2.1");
        let mut msg = w.begin_message(id).unwrap();
        msg.write_simple("BGM", &["E01"]).unwrap();
        msg.finish().unwrap();
        let bytes = w.finish().unwrap();
        assert_eq!(bytes, b"UNH+1+UTILMD:D:11A:UN:S2.1'BGM+E01'UNT+3+1'");
        // parse_unh → begin_message round-trips.
        let segments: Vec<_> = crate::from_bytes(&bytes).collect::<Result<_, _>>().unwrap();
        let parsed = crate::parse_unh(&segments[0]).unwrap();
        assert_eq!(parsed, id);
        let mut again = Writer::new(Vec::new());
        again.begin_message(parsed).unwrap().finish().unwrap();
        assert!(
            again
                .finish()
                .unwrap()
                .starts_with(b"UNH+1+UTILMD:D:11A:UN:S2.1'")
        );
    }
}
