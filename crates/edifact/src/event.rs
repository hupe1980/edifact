//! Event model for EDIFACT serialization.
//!
//! [`EdifactEvent`] carries its text as a [`Cow`], so one type serves both
//! streaming emission (every value borrows from the value being serialized) and
//! [`VecEmitter`], which has to keep events past the borrow that produced them.

use crate::EdifactError;
use std::borrow::Cow;
use std::io::Write;

// ── event types ───────────────────────────────────────────────────────────────

/// A borrowed EDIFACT event emitted during serialization.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EdifactEvent<'a> {
    /// Beginning of a new segment (e.g. `"BGM"`, `"NAD"`).
    StartSegment {
        /// Segment tag.
        tag: Cow<'a, str>,
    },
    /// A data element value — first (or only) component of a new element.
    Element {
        /// Element text value.
        value: Cow<'a, str>,
    },
    /// An additional component within the current element.
    ComponentElement {
        /// Component text value.
        value: Cow<'a, str>,
    },
    /// The first component of a further occurrence of the current data element.
    ///
    /// The write-side mirror of [`Token::RepeatElement`][crate::Token::RepeatElement]:
    /// the parser splits repeating data elements, so the serializer has to be
    /// able to produce them, or a value that round-trips through a typed struct
    /// comes back collapsed into one occurrence.
    ///
    /// Requires an active repetition separator — see
    /// [`ServiceStringAdvice::is_repetition_active`][crate::ServiceStringAdvice::is_repetition_active].
    /// Without one there is no byte to separate occurrences with, so
    /// [`WriterEmitter`] returns
    /// [`EdifactError::RepetitionSeparatorNotDeclared`] rather than emitting
    /// output that reads back as a single occurrence.
    RepeatElement {
        /// First component of the new occurrence.
        value: Cow<'a, str>,
    },
    /// End of the current segment.
    EndSegment,
}

impl<'a> EdifactEvent<'a> {
    /// A [`StartSegment`][Self::StartSegment] event for `tag`.
    #[inline]
    #[must_use]
    pub fn start(tag: impl Into<Cow<'a, str>>) -> Self {
        Self::StartSegment { tag: tag.into() }
    }

    /// An [`Element`][Self::Element] event carrying `value`.
    #[inline]
    #[must_use]
    pub fn element(value: impl Into<Cow<'a, str>>) -> Self {
        Self::Element {
            value: value.into(),
        }
    }

    /// A [`ComponentElement`][Self::ComponentElement] event carrying `value`.
    #[inline]
    #[must_use]
    pub fn component(value: impl Into<Cow<'a, str>>) -> Self {
        Self::ComponentElement {
            value: value.into(),
        }
    }

    /// A [`RepeatElement`][Self::RepeatElement] event carrying `value`.
    #[inline]
    #[must_use]
    pub fn repeat(value: impl Into<Cow<'a, str>>) -> Self {
        Self::RepeatElement {
            value: value.into(),
        }
    }
}

impl EdifactEvent<'_> {
    /// Detach this event from the value it borrows from, cloning its text.
    #[must_use]
    pub fn into_owned(self) -> EdifactEvent<'static> {
        match self {
            Self::StartSegment { tag } => EdifactEvent::start(Cow::Owned(tag.into_owned())),
            Self::Element { value } => EdifactEvent::element(Cow::Owned(value.into_owned())),
            Self::ComponentElement { value } => {
                EdifactEvent::component(Cow::Owned(value.into_owned()))
            }
            Self::RepeatElement { value } => EdifactEvent::repeat(Cow::Owned(value.into_owned())),
            Self::EndSegment => EdifactEvent::EndSegment,
        }
    }
}

// ── emitter trait ─────────────────────────────────────────────────────────────

/// Trait for any sink that can consume [`EdifactEvent`]s.
pub trait EventEmitter {
    /// Consume one event.
    fn emit(&mut self, event: EdifactEvent<'_>) -> Result<(), EdifactError>;

    /// Return the decimal-mark byte used by the interchange (`b'.'` by default).
    ///
    /// Serializers that format numeric values (e.g. [`crate::ser::DecimalFloat`])
    /// call this to discover whether to emit `12.5` or `12,5`.
    ///
    /// The default implementation returns `b'.'`, which is correct for standard
    /// EDIFACT interchanges that do not declare a UNA service string or that use
    /// the ISO 9735 default.  Override this in emitters backed by a
    /// [`crate::Writer`] with a custom [`crate::ServiceStringAdvice`].
    #[inline]
    fn decimal_mark(&self) -> u8 {
        b'.'
    }
}

// ── VecEmitter ────────────────────────────────────────────────────────────────

/// Collects events into a `Vec<EdifactEvent<'static>>`.
///
/// Useful for testing and introspection.  Does not leak memory.
#[derive(Debug, Default)]
pub struct VecEmitter {
    /// Collected owned events.
    pub events: Vec<EdifactEvent<'static>>,
}

impl EventEmitter for VecEmitter {
    fn emit(&mut self, event: EdifactEvent<'_>) -> Result<(), EdifactError> {
        self.events.push(event.into_owned());
        Ok(())
    }
}

// ── WriterEmitter ─────────────────────────────────────────────────────────────

/// Internal protocol-state machine for [`WriterEmitter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmitterState {
    /// Between segments: no open segment.
    Idle,
    /// A [`EdifactEvent::StartSegment`] has been emitted; no element written yet.
    InSegment,
    /// An [`EdifactEvent::Element`] has been emitted; `ComponentElement` is valid.
    InElement,
}

/// Writes EDIFACT events directly to any [`Write`] implementation.
///
/// Each event is written to the underlying writer immediately, escaping in
/// place, so the emitter allocates nothing per event (asserted by the
/// `allocation_budget` test `writer_emitter_allocates_nothing_per_event`). Two
/// exceptions: a bound repertoire ([`with_charset`][Self::with_charset])
/// transcodes non-ASCII text into a temporary buffer, and the underlying
/// `Write` may allocate on its own (a `Vec<u8>` grows).
///
/// # Protocol
///
/// Events must arrive in the order produced by [`crate::EdifactSerialize`]:
/// `StartSegment` → zero or more (`Element` → zero or more `ComponentElement`) → `EndSegment`.
///
/// Any violation of this protocol returns
/// [`EdifactError::InvalidEventSequence`] immediately.  Violations are
/// detected in both debug and release builds.
///
/// # Errors and poisoning
///
/// The unit is one event. An event refused before its first byte — a protocol
/// violation, an invalid tag, an undeclared repetition separator — writes
/// nothing and leaves the emitter where it was. Any other failure poisons the
/// underlying [`Writer`][crate::Writer]: every later `emit` and
/// [`finish`][Self::finish] returns [`EdifactError::WriterPoisoned`].
pub struct WriterEmitter<W: Write> {
    writer: crate::Writer<W>,
    state: EmitterState,
}

impl<W: Write> WriterEmitter<W> {
    /// Create a new `WriterEmitter` with default EDIFACT delimiters.
    pub fn new(inner: W) -> Self {
        Self {
            writer: crate::Writer::new(inner),
            state: EmitterState::Idle,
        }
    }

    /// Create a new `WriterEmitter` with custom delimiters, writing a UNA header first.
    ///
    /// # Errors
    ///
    /// Returns [`EdifactError::InvalidUna`] when `ssa.is_valid()` is false.
    pub fn with_una(
        inner: W,
        ssa: crate::tokenizer::ServiceStringAdvice,
    ) -> Result<Self, crate::EdifactError> {
        Ok(Self {
            writer: crate::Writer::with_una(inner, ssa)?,
            state: EmitterState::Idle,
        })
    }

    /// Bind this emitter's writer to a character repertoire.
    ///
    /// The typed-serialization counterpart of
    /// [`Writer::with_charset`][crate::Writer::with_charset]. Without it a
    /// `#[derive(EdifactSerialize)]` struct goes out as UTF-8, which a
    /// `UNOC`…`UNOK` partner reads as mojibake (`ü` becomes two characters)
    /// rather than as an error.
    ///
    /// # Example
    ///
    /// ```
    /// use edifact::{Charset, EdifactEvent, EventEmitter, WriterEmitter};
    ///
    /// let mut emitter = WriterEmitter::new(Vec::new()).with_charset(Charset::UnoC);
    /// emitter.emit(EdifactEvent::start("NAD"))?;
    /// emitter.emit(EdifactEvent::element("Müller"))?;
    /// emitter.emit(EdifactEvent::EndSegment)?;
    /// assert_eq!(emitter.finish()?, b"NAD+M\xFCller'".to_vec());
    /// # Ok::<(), edifact::EdifactError>(())
    /// ```
    #[must_use]
    pub fn with_charset(mut self, charset: crate::Charset) -> Self {
        self.writer = self.writer.with_charset(charset);
        self
    }

    /// Flush and consume the emitter, returning the underlying writer.
    ///
    /// # Errors
    ///
    /// [`EdifactError::WriterPoisoned`] after a failure,
    /// [`EdifactError::InvalidEventSequence`] while a segment is open (emit
    /// `EndSegment` first), and [`EdifactError::Io`] when the flush fails. Use
    /// [`into_inner`][Self::into_inner] to get the sink back regardless.
    ///
    /// ```
    /// use edifact::{EdifactError, EdifactEvent, EventEmitter, WriterEmitter};
    ///
    /// let mut emitter = WriterEmitter::new(Vec::new());
    /// emitter.emit(EdifactEvent::start("BGM"))?;
    /// emitter.emit(EdifactEvent::element("220"))?;
    /// // No `EndSegment`: the segment is incomplete.
    /// assert!(matches!(
    ///     emitter.finish(),
    ///     Err(EdifactError::InvalidEventSequence { .. })
    /// ));
    /// # Ok::<(), EdifactError>(())
    /// ```
    pub fn finish(self) -> Result<W, EdifactError> {
        self.writer.check_healthy()?;
        if self.state != EmitterState::Idle {
            return Err(EdifactError::InvalidEventSequence {
                message: "finish() called with a segment still open; emit EndSegment first",
            });
        }
        self.writer.finish()
    }

    /// Return the sink as it is: no flush, no checks, open segment or not.
    #[must_use]
    pub fn into_inner(self) -> W {
        self.writer.into_inner()
    }

    /// Number of complete segments written so far.
    pub fn segment_count(&self) -> u64 {
        self.writer.segment_count()
    }

    /// Return the active [`ServiceStringAdvice`][crate::ServiceStringAdvice].
    ///
    /// Callers can use this to format values (e.g., floats) using the correct
    /// decimal-mark character configured in the UNA header.
    pub fn service_string_advice(&self) -> crate::tokenizer::ServiceStringAdvice {
        self.writer.service_string_advice()
    }
}

impl<W: Write> EventEmitter for WriterEmitter<W> {
    #[inline]
    fn decimal_mark(&self) -> u8 {
        self.writer.service_string_advice().decimal_mark
    }

    fn emit(&mut self, event: EdifactEvent<'_>) -> Result<(), EdifactError> {
        self.writer.check_healthy()?;
        // Each arm refuses a protocol violation before writing, and moves the
        // state only after the write succeeded.
        let next = match event {
            EdifactEvent::StartSegment { tag } => {
                if self.state != EmitterState::Idle {
                    return Err(EdifactError::InvalidEventSequence {
                        message: "StartSegment emitted while a segment is already open; emit EndSegment first",
                    });
                }
                self.writer.emit_tag(&tag)?;
                EmitterState::InSegment
            }
            EdifactEvent::Element { value } => {
                if self.state == EmitterState::Idle {
                    return Err(EdifactError::InvalidEventSequence {
                        message: "Element emitted outside of a segment; emit StartSegment first",
                    });
                }
                self.writer.emit_element(&value)?;
                EmitterState::InElement
            }
            EdifactEvent::ComponentElement { value } => {
                if self.state != EmitterState::InElement {
                    return Err(EdifactError::InvalidEventSequence {
                        message: "ComponentElement emitted without a preceding Element in the same segment",
                    });
                }
                self.writer.emit_component(&value)?;
                EmitterState::InElement
            }
            EdifactEvent::RepeatElement { value } => {
                if self.state != EmitterState::InElement {
                    return Err(EdifactError::InvalidEventSequence {
                        message: "RepeatElement emitted without a preceding Element in the same segment",
                    });
                }
                self.writer.emit_repetition(&value)?;
                EmitterState::InElement
            }
            EdifactEvent::EndSegment => {
                if self.state == EmitterState::Idle {
                    return Err(EdifactError::InvalidEventSequence {
                        message: "EndSegment emitted while no segment is open; emit StartSegment first",
                    });
                }
                self.writer.emit_segment_end()?;
                EmitterState::Idle
            }
        };
        self.state = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vec_emitter_no_memory_leak() {
        let mut e = VecEmitter::default();
        e.emit(EdifactEvent::start("BGM")).unwrap();
        e.emit(EdifactEvent::element("E03")).unwrap();
        e.emit(EdifactEvent::EndSegment).unwrap();
        assert_eq!(e.events[0], EdifactEvent::start("BGM".to_owned()));
        assert_eq!(e.events[1], EdifactEvent::element("E03".to_owned()));
    }

    #[test]
    fn writer_emitter_produces_valid_edifact() {
        let mut buf = Vec::new();
        {
            let mut e = WriterEmitter::new(&mut buf);
            e.emit(EdifactEvent::start("BGM")).unwrap();
            e.emit(EdifactEvent::element("E03")).unwrap();
            e.emit(EdifactEvent::element("11042")).unwrap();
            e.emit(EdifactEvent::EndSegment).unwrap();
            e.finish().unwrap();
        }
        assert_eq!(buf, b"BGM+E03+11042'");
    }

    #[test]
    fn writer_emitter_handles_components() {
        let mut buf = Vec::new();
        {
            let mut e = WriterEmitter::new(&mut buf);
            e.emit(EdifactEvent::start("NAD")).unwrap();
            e.emit(EdifactEvent::element("MS")).unwrap();
            e.emit(EdifactEvent::element("9900112233445")).unwrap();
            e.emit(EdifactEvent::component("")).unwrap();
            e.emit(EdifactEvent::component("293")).unwrap();
            e.emit(EdifactEvent::EndSegment).unwrap();
            e.finish().unwrap();
        }
        let s = std::str::from_utf8(&buf).unwrap();
        assert_eq!(s, "NAD+MS+9900112233445::293'");
    }

    #[test]
    fn repetitions_round_trip_through_the_event_layer() {
        // The parser splits repeating data elements, so the serializer has to be
        // able to produce them — otherwise a value that goes out through a typed
        // struct comes back collapsed into one occurrence.
        let ssa = crate::ServiceStringAdvice::from_bytes(b"UNA:+.?*'").unwrap();
        let mut buf = Vec::new();
        {
            let mut e = WriterEmitter::with_una(&mut buf, ssa).unwrap();
            e.emit(EdifactEvent::start("RFF")).unwrap();
            e.emit(EdifactEvent::element("ON")).unwrap();
            e.emit(EdifactEvent::component("1")).unwrap();
            e.emit(EdifactEvent::repeat("ON")).unwrap();
            e.emit(EdifactEvent::component("2")).unwrap();
            e.emit(EdifactEvent::EndSegment).unwrap();
            e.finish().unwrap();
        }
        assert_eq!(
            std::str::from_utf8(&buf).unwrap(),
            "UNA:+.?*'RFF+ON:1*ON:2'"
        );

        let segments: Vec<_> = crate::from_bytes(&buf)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let element = segments[0].get_element(0).unwrap();
        assert_eq!(element.repeat_count(), 2);
        assert_eq!(element.repetition(1).unwrap()[1].0, "2");
    }

    #[test]
    fn a_repetition_without_a_declared_separator_is_refused() {
        let mut e = WriterEmitter::new(Vec::<u8>::new());
        e.emit(EdifactEvent::start("RFF")).unwrap();
        e.emit(EdifactEvent::element("ON")).unwrap();
        let err = e.emit(EdifactEvent::repeat("ON")).unwrap_err();
        assert!(
            matches!(err, EdifactError::RepetitionSeparatorNotDeclared),
            "expected RepetitionSeparatorNotDeclared, got {err:?}"
        );
    }

    #[test]
    fn a_repetition_before_any_element_is_refused() {
        let ssa = crate::ServiceStringAdvice::from_bytes(b"UNA:+.?*'").unwrap();
        let mut e = WriterEmitter::with_una(Vec::<u8>::new(), ssa).unwrap();
        e.emit(EdifactEvent::start("RFF")).unwrap();
        let err = e.emit(EdifactEvent::repeat("ON")).unwrap_err();
        assert!(
            matches!(err, EdifactError::InvalidEventSequence { .. }),
            "expected InvalidEventSequence, got {err:?}"
        );
    }

    // ── protocol-violation tests (BUG 2.1) ───────────────────────────────────

    #[test]
    fn writer_emitter_element_before_start_segment_is_err() {
        let mut e = WriterEmitter::new(Vec::<u8>::new());
        let err = e.emit(EdifactEvent::element("X")).unwrap_err();
        assert!(
            matches!(err, crate::EdifactError::InvalidEventSequence { .. }),
            "expected InvalidEventSequence, got {err:?}"
        );
    }

    #[test]
    fn writer_emitter_component_before_element_is_err() {
        let mut e = WriterEmitter::new(Vec::<u8>::new());
        e.emit(EdifactEvent::start("BGM")).unwrap();
        let err = e.emit(EdifactEvent::component("X")).unwrap_err();
        assert!(
            matches!(err, crate::EdifactError::InvalidEventSequence { .. }),
            "expected InvalidEventSequence, got {err:?}"
        );
    }

    #[test]
    fn writer_emitter_double_start_segment_is_err() {
        let mut e = WriterEmitter::new(Vec::<u8>::new());
        e.emit(EdifactEvent::start("BGM")).unwrap();
        let err = e.emit(EdifactEvent::start("DTM")).unwrap_err();
        assert!(
            matches!(err, crate::EdifactError::InvalidEventSequence { .. }),
            "expected InvalidEventSequence, got {err:?}"
        );
    }

    #[test]
    fn writer_emitter_end_segment_without_start_is_err() {
        let mut e = WriterEmitter::new(Vec::<u8>::new());
        let err = e.emit(EdifactEvent::EndSegment).unwrap_err();
        assert!(
            matches!(err, crate::EdifactError::InvalidEventSequence { .. }),
            "expected InvalidEventSequence, got {err:?}"
        );
    }

    // ── honest finish and poisoning ──────────────────────────────────────────

    /// Fails every write from the `fail_at`-th (zero-based) on.
    struct FailingSink {
        buf: Vec<u8>,
        writes: usize,
        fail_at: usize,
    }

    impl Write for FailingSink {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            self.writes += 1;
            if self.writes > self.fail_at {
                return Err(std::io::Error::other("injected"));
            }
            self.buf.extend_from_slice(data);
            Ok(data.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn finish_with_open_segment_is_refused() {
        let mut buf = Vec::new();
        {
            let mut e = WriterEmitter::new(&mut buf);
            e.emit(EdifactEvent::start("BGM")).unwrap();
            e.emit(EdifactEvent::element("220")).unwrap();
            let err = e.finish().unwrap_err();
            assert!(
                matches!(err, EdifactError::InvalidEventSequence { .. }),
                "{err:?}"
            );
        }
        assert_eq!(buf, b"BGM+220");

        let mut e = WriterEmitter::new(Vec::new());
        e.emit(EdifactEvent::start("BGM")).unwrap();
        e.emit(EdifactEvent::element("220")).unwrap();
        assert_eq!(e.into_inner(), b"BGM+220");
    }

    #[test]
    fn refused_tag_does_not_advance_state() {
        let mut e = WriterEmitter::new(Vec::new());
        let err = e.emit(EdifactEvent::start("bad")).unwrap_err();
        assert!(matches!(err, EdifactError::InvalidSegmentTag(_)), "{err:?}");
        e.emit(EdifactEvent::start("BGM")).unwrap();
        e.emit(EdifactEvent::EndSegment).unwrap();
        assert_eq!(e.finish().unwrap(), b"BGM'");
    }

    #[test]
    fn refused_repeat_event_keeps_segment_completable() {
        let mut e = WriterEmitter::new(Vec::new());
        e.emit(EdifactEvent::start("RFF")).unwrap();
        e.emit(EdifactEvent::element("ON")).unwrap();
        assert!(matches!(
            e.emit(EdifactEvent::repeat("ON")),
            Err(EdifactError::RepetitionSeparatorNotDeclared)
        ));
        e.emit(EdifactEvent::element("1")).unwrap();
        e.emit(EdifactEvent::EndSegment).unwrap();
        assert!(!e.writer.is_poisoned());
        assert_eq!(e.finish().unwrap(), b"RFF+ON+1'");
    }

    #[test]
    fn emitter_is_poisoned_after_sink_failure() {
        let sink = FailingSink {
            buf: Vec::new(),
            writes: 0,
            fail_at: 1,
        };
        let mut e = WriterEmitter::new(sink);
        e.emit(EdifactEvent::start("BGM")).unwrap();
        let err = e.emit(EdifactEvent::element("220")).unwrap_err();
        assert!(matches!(err, EdifactError::Io(_)), "{err:?}");
        for event in [
            EdifactEvent::element("220"),
            EdifactEvent::EndSegment,
            EdifactEvent::start("DTM"),
        ] {
            let err = e.emit(event).unwrap_err();
            assert!(
                matches!(err, EdifactError::WriterPoisoned { .. }),
                "{err:?}"
            );
        }
        assert_eq!(e.into_inner().buf, b"BGM");
    }

    #[test]
    fn poisoned_emitter_refuses_to_finish() {
        let sink = FailingSink {
            buf: Vec::new(),
            writes: 0,
            fail_at: 0,
        };
        let mut e = WriterEmitter::new(sink);
        assert!(e.emit(EdifactEvent::start("BGM")).is_err());
        assert!(matches!(
            e.finish(),
            Err(EdifactError::WriterPoisoned { .. })
        ));
    }

    #[test]
    fn charset_failure_mid_event_poisons() {
        let mut e = WriterEmitter::new(Vec::new()).with_charset(crate::Charset::UnoC);
        e.emit(EdifactEvent::start("NAD")).unwrap();
        let err = e.emit(EdifactEvent::element("M€ller")).unwrap_err();
        assert!(
            matches!(err, EdifactError::CharacterNotInRepertoire { .. }),
            "{err:?}"
        );
        assert!(e.writer.is_poisoned());
        assert_eq!(e.into_inner(), b"NAD+");
    }
}
