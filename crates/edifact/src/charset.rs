//! EDIFACT character repertoires — the `UNB` S001 DE 0001 syntax identifier.
//!
//! An EDIFACT interchange is self-describing about its encoding: `UNB` S001
//! component 1 names the repertoire the payload is written in, and ISO 9735-1
//! §6 requires everything up to and including S001 to be written in the ISO/IEC
//! 646 basic code table — so the header can always be read before the encoding
//! is known.
//!
//! # Why this exists
//!
//! **UTF-8 is not a superset of `UNOC`.** `UNOC` is ISO 8859-1, where `ü` is the
//! single byte `0xFC` — which is not valid UTF-8. A German `ORDERS` carrying
//! `Müller` is a perfectly conformant `UNOC` interchange, and decoding it as
//! UTF-8 fails outright. The same holds for every `UNOD`…`UNOK` interchange.
//!
//! [`Charset`] converts those payloads to UTF-8 so the rest of the crate — which
//! is UTF-8 throughout — can parse them:
//!
//! ```
//! use edifact::{Charset, decode_interchange, from_bytes};
//!
//! // A UNOC interchange: `Müller` is `4D FC 6C 6C 65 72`, not valid UTF-8.
//! let mut raw = b"UNB+UNOC:3+S+R+200101:0900+1'NAD+BY+M".to_vec();
//! raw.push(0xFC);
//! raw.extend_from_slice(b"ller'UNZ+0+1'");
//!
//! // Parsing the raw bytes fails …
//! assert!(from_bytes(&raw).collect::<Result<Vec<_>, _>>().is_err());
//!
//! // … so decode first. The syntax identifier is read out of the UNB.
//! let utf8 = decode_interchange(&raw)?;
//! let segments: Vec<_> = from_bytes(&utf8).collect::<Result<Vec<_>, _>>()?;
//! assert_eq!(segments[1].element_str(1), Some("Müller"));
//! # Ok::<(), edifact::EdifactError>(())
//! ```
//!
//! # Decoding is permissive, validation is strict
//!
//! [`decode`][Charset::decode] treats `UNOA` and `UNOB` as ASCII rather than
//! enforcing their restricted repertoires, because real interchanges routinely
//! carry a character or two outside level A and refusing to *parse* them would
//! hide every other finding behind an encoding error. The repertoire is checked
//! separately by [`permits`][Charset::permits], which the envelope validator
//! surfaces as [`EdifactError::CharacterNotInRepertoire`] — a validation issue
//! you can see alongside the rest of the report, or suppress.

use crate::error::EdifactError;
use std::borrow::Cow;
use std::io::Read;

/// An EDIFACT character repertoire, named by `UNB` S001 DE 0001.
///
/// Every variant is a **single-byte, ASCII-transparent** encoding or UTF-8, which
/// is what lets the tokenizer scan for delimiters before decoding: bytes
/// `0x00..=0x7F` mean the same thing in all of them.
///
/// `UNOX` (ISO/IEC 2022 code extension) and `KECA` (a Korean identifier outside
/// ISO 9735) are deliberately absent:
/// both are stateful or multi-byte in a way that would invalidate byte-level
/// delimiter scanning, and pretending to support them would be worse than saying
/// so. [`Charset::from_syntax_identifier`] reports
/// [`EdifactError::UnsupportedCharset`] for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Charset {
    /// `UNOA` — ISO 9735 level A: upper-case letters, digits, space, and a fixed
    /// punctuation set. Decoded as ASCII; see [`permits`][Self::permits].
    UnoA,
    /// `UNOB` — ISO 9735 level B: level A plus lower-case letters.
    UnoB,
    /// `UNOC` — ISO 8859-1 (Latin-1).
    UnoC,
    /// `UNOD` — ISO 8859-2 (Latin-2, Central European).
    UnoD,
    /// `UNOE` — ISO 8859-5 (Latin/Cyrillic).
    UnoE,
    /// `UNOF` — ISO 8859-7 (Latin/Greek).
    UnoF,
    /// `UNOG` — ISO 8859-3 (Latin-3, South European).
    UnoG,
    /// `UNOH` — ISO 8859-4 (Latin-4, North European).
    UnoH,
    /// `UNOI` — ISO 8859-6 (Latin/Arabic).
    UnoI,
    /// `UNOJ` — ISO 8859-8 (Latin/Hebrew).
    UnoJ,
    /// `UNOK` — ISO 8859-9 (Latin-5, Turkish).
    UnoK,
    /// `UNOY` — ISO 10646-1 / UTF-8. The identity transform for this crate.
    UnoY,
}

/// Marks a code point the relevant ISO 8859 part leaves undefined.
const UNDEFINED: u16 = 0xFFFF;

/// How a repertoire maps bytes `0x80..=0xFF`.
#[derive(Debug, Clone, Copy)]
enum HighHalf {
    /// Nothing above `0x7F` is in the repertoire (the ASCII subsets, and UTF-8,
    /// which is decoded whole rather than byte by byte).
    None,
    /// The code point equals the byte value — ISO 8859-1.
    Identity,
    /// A part-specific table for `0xA0..=0xFF`; `0x80..=0x9F` stay C1 controls.
    Table(&'static [u16; 96]),
}

/// ISO 9735 level A punctuation, in addition to `A`–`Z`, `0`–`9`, and space.
///
/// The first group (`.` through `=`) plus the four service characters
/// (`'` `+` `:` `?`) is available unconditionally. The second group
/// (`!` `"` `%` `&` `*` `;` `<` `>`) is level A too, but ISO 9735 withholds it
/// from telex transmission — a transport distinction this crate does not model,
/// so those characters are permitted.
const LEVEL_A_PUNCTUATION: &[char] = &[
    '.', ',', '-', '(', ')', '/', '=', '\'', '+', ':', '?', '!', '"', '%', '&', '*', ';', '<', '>',
];

impl Charset {
    /// Resolve a `UNB` S001 DE 0001 syntax identifier.
    ///
    /// # Errors
    ///
    /// [`EdifactError::UnsupportedCharset`] for `UNOX` and `KECA`, which this
    /// crate cannot represent as a single-byte transform, and
    /// [`EdifactError::UnrecognisedSyntaxIdentifier`] for any other identifier.
    pub fn from_syntax_identifier(identifier: &str) -> Result<Self, EdifactError> {
        Ok(match identifier {
            "UNOA" => Self::UnoA,
            "UNOB" => Self::UnoB,
            "UNOC" => Self::UnoC,
            "UNOD" => Self::UnoD,
            "UNOE" => Self::UnoE,
            "UNOF" => Self::UnoF,
            "UNOG" => Self::UnoG,
            "UNOH" => Self::UnoH,
            "UNOI" => Self::UnoI,
            "UNOJ" => Self::UnoJ,
            "UNOK" => Self::UnoK,
            "UNOY" => Self::UnoY,
            "UNOX" | "KECA" => {
                return Err(EdifactError::UnsupportedCharset {
                    syntax_identifier: identifier.to_owned(),
                });
            }
            other => {
                return Err(EdifactError::UnrecognisedSyntaxIdentifier(other.to_owned()));
            }
        })
    }

    /// The `UNB` S001 DE 0001 identifier for this repertoire.
    #[must_use]
    pub const fn syntax_identifier(self) -> &'static str {
        match self {
            Self::UnoA => "UNOA",
            Self::UnoB => "UNOB",
            Self::UnoC => "UNOC",
            Self::UnoD => "UNOD",
            Self::UnoE => "UNOE",
            Self::UnoF => "UNOF",
            Self::UnoG => "UNOG",
            Self::UnoH => "UNOH",
            Self::UnoI => "UNOI",
            Self::UnoJ => "UNOJ",
            Self::UnoK => "UNOK",
            Self::UnoY => "UNOY",
        }
    }

    /// `true` when the payload is already UTF-8 and needs no transcoding.
    ///
    /// True for `UNOY`, and for `UNOA`/`UNOB` whose repertoires are ASCII
    /// subsets — a conformant payload in either is valid UTF-8 unchanged.
    #[must_use]
    pub const fn is_utf8(self) -> bool {
        matches!(self, Self::UnoY | Self::UnoA | Self::UnoB)
    }

    /// How this repertoire maps bytes `0x80..=0xFF`.
    ///
    /// Modelled explicitly rather than as an `Option<table>`: "no table" is true
    /// of both the ASCII subsets — where nothing above `0x7F` exists at all — and
    /// of ISO 8859-1, where the code point *is* the byte. Collapsing the two let
    /// a `UNOA` writer happily emit `0xFC` for `ü`.
    const fn high_half(self) -> HighHalf {
        match self {
            // Level A and level B are subsets of ASCII: the high half is empty.
            Self::UnoA | Self::UnoB => HighHalf::None,
            // UTF-8 is not a single-byte encoding; the byte-wise path never runs.
            Self::UnoY => HighHalf::None,
            // ISO 8859-1 maps every byte to the code point of the same value.
            Self::UnoC => HighHalf::Identity,
            Self::UnoD => HighHalf::Table(&UNOD_HIGH),
            Self::UnoE => HighHalf::Table(&UNOE_HIGH),
            Self::UnoF => HighHalf::Table(&UNOF_HIGH),
            Self::UnoG => HighHalf::Table(&UNOG_HIGH),
            Self::UnoH => HighHalf::Table(&UNOH_HIGH),
            Self::UnoI => HighHalf::Table(&UNOI_HIGH),
            Self::UnoJ => HighHalf::Table(&UNOJ_HIGH),
            Self::UnoK => HighHalf::Table(&UNOK_HIGH),
        }
    }

    /// Map one non-ASCII byte to its code point.
    ///
    /// `0x80..=0x9F` is the C1 control range, which every ISO 8859 part maps to
    /// `U+0080..=U+009F`. Above that the part-specific table applies; a slot the
    /// standard leaves undefined yields `None`.
    fn decode_byte(self, byte: u8) -> Option<char> {
        debug_assert!(byte >= 0x80, "decode_byte is for the non-ASCII half only");
        match self.high_half() {
            HighHalf::None => None,
            HighHalf::Identity => char::from_u32(u32::from(byte)),
            HighHalf::Table(table) => {
                if byte < 0xA0 {
                    return char::from_u32(u32::from(byte));
                }
                match table[usize::from(byte - 0xA0)] {
                    UNDEFINED => None,
                    code => char::from_u32(u32::from(code)),
                }
            }
        }
    }

    /// Map one code point back to its byte, or `None` when unrepresentable.
    fn encode_char(self, ch: char) -> Option<u8> {
        let code = u32::from(ch);
        if code < 0x80 {
            return Some(code as u8);
        }
        match self.high_half() {
            HighHalf::None => None,
            HighHalf::Identity => u8::try_from(code).ok(),
            HighHalf::Table(table) => {
                if code < 0xA0 {
                    return Some(code as u8);
                }
                table
                    .iter()
                    .position(|&c| c != UNDEFINED && u32::from(c) == code)
                    .map(|i| (0xA0 + i) as u8)
            }
        }
    }

    /// Whether `ch` is in this repertoire.
    ///
    /// This is the strict ISO 9735 rule, and it is *not* applied while decoding —
    /// see the module documentation. `UNOY` permits every `char`.
    ///
    /// # Example
    ///
    /// ```
    /// use edifact::Charset;
    ///
    /// assert!(Charset::UnoA.permits('A'));
    /// assert!(!Charset::UnoA.permits('a'));   // level A is upper-case only
    /// assert!(Charset::UnoB.permits('a'));
    /// assert!(!Charset::UnoB.permits('ü'));   // level B is still ASCII
    /// assert!(Charset::UnoC.permits('ü'));    // ISO 8859-1
    /// assert!(!Charset::UnoC.permits('€'));   // not in Latin-1
    /// ```
    #[must_use]
    pub fn permits(self, ch: char) -> bool {
        match self {
            Self::UnoY => true,
            Self::UnoA => {
                ch.is_ascii_uppercase()
                    || ch.is_ascii_digit()
                    || ch == ' '
                    || LEVEL_A_PUNCTUATION.contains(&ch)
            }
            Self::UnoB => {
                ch.is_ascii_lowercase()
                    || ch.is_ascii_uppercase()
                    || ch.is_ascii_digit()
                    || ch == ' '
                    || LEVEL_A_PUNCTUATION.contains(&ch)
            }
            _ => self.encode_char(ch).is_some(),
        }
    }

    /// The first character of `text` that this repertoire cannot carry, with its
    /// byte offset within `text`.
    ///
    /// `None` when every character is permitted.
    #[must_use]
    pub fn first_violation(self, text: &str) -> Option<(usize, char)> {
        if self == Self::UnoY {
            return None;
        }
        text.char_indices().find(|&(_, ch)| !self.permits(ch))
    }

    /// Decode `bytes` from this repertoire into UTF-8 text.
    ///
    /// Returns [`Cow::Borrowed`] — and copies nothing — when `bytes` is pure
    /// ASCII, whatever the declared repertoire.
    ///
    /// Every byte is transcoded, a leading byte order mark included;
    /// [`decode_interchange`] keeps the interchange prologue verbatim.
    ///
    /// # Errors
    ///
    /// [`EdifactError::InvalidText`] when a byte falls in a slot the repertoire
    /// leaves undefined, or when a `UNOY` payload is not valid UTF-8. The
    /// reported offset is the byte position within `bytes`.
    pub fn decode(self, bytes: &[u8]) -> Result<Cow<'_, str>, EdifactError> {
        if bytes.is_ascii() {
            // SAFETY-FREE: an all-ASCII slice is valid UTF-8 by construction, and
            // every supported repertoire agrees with ASCII on 0x00..=0x7F.
            return std::str::from_utf8(bytes).map(Cow::Borrowed).map_err(|e| {
                EdifactError::InvalidText {
                    offset: e.valid_up_to(),
                }
            });
        }
        if self.is_utf8() {
            return std::str::from_utf8(bytes).map(Cow::Borrowed).map_err(|e| {
                EdifactError::InvalidText {
                    offset: e.valid_up_to(),
                }
            });
        }
        // Every ISO 8859 code point is below U+0800, so two UTF-8 bytes is the
        // worst case per input byte.
        let mut out = String::with_capacity(bytes.len() + bytes.len() / 2);
        for (offset, &byte) in bytes.iter().enumerate() {
            if byte < 0x80 {
                out.push(byte as char);
            } else {
                let ch = self
                    .decode_byte(byte)
                    .ok_or(EdifactError::InvalidText { offset })?;
                out.push(ch);
            }
        }
        Ok(Cow::Owned(out))
    }

    /// Encode UTF-8 `text` into this repertoire's bytes.
    ///
    /// Returns [`Cow::Borrowed`] when `text` is pure ASCII.
    ///
    /// # Errors
    ///
    /// [`EdifactError::CharacterNotInRepertoire`] for the first character the
    /// repertoire cannot carry.
    pub fn encode(self, text: &str) -> Result<Cow<'_, [u8]>, EdifactError> {
        if text.is_ascii() || self == Self::UnoY {
            return Ok(Cow::Borrowed(text.as_bytes()));
        }
        let mut out = Vec::with_capacity(text.len());
        for (offset, ch) in text.char_indices() {
            let byte = self
                .encode_char(ch)
                .ok_or(EdifactError::CharacterNotInRepertoire {
                    charset: self.syntax_identifier(),
                    character: ch,
                    offset,
                })?;
            out.push(byte);
        }
        Ok(Cow::Owned(out))
    }

    /// Transcode a whole interchange to UTF-8 bytes.
    ///
    /// Returns [`Cow::Borrowed`] when nothing needs changing, so the zero-copy
    /// path through [`from_bytes`][crate::from_bytes] is preserved for ASCII and
    /// `UNOY` payloads.
    ///
    /// Every byte is transcoded, a leading byte order mark included;
    /// [`decode_interchange`] keeps the interchange prologue verbatim.
    ///
    /// # Spans
    ///
    /// Byte offsets in the decoded buffer do **not** line up with the original
    /// when transcoding actually happened — one `0xFC` becomes two bytes. Every
    /// [`Span`][crate::Span] produced downstream indexes the **decoded** buffer,
    /// which is the one you hold and the one diagnostics render against.
    ///
    /// # Errors
    ///
    /// As [`decode`][Self::decode].
    pub fn transcode_to_utf8(self, bytes: &[u8]) -> Result<Cow<'_, [u8]>, EdifactError> {
        match self.decode(bytes)? {
            Cow::Borrowed(_) => Ok(Cow::Borrowed(bytes)),
            Cow::Owned(text) => Ok(Cow::Owned(text.into_bytes())),
        }
    }

    /// Wrap a reader so it yields UTF-8, transcoding from this repertoire.
    ///
    /// This is the streaming counterpart of
    /// [`transcode_to_utf8`][Self::transcode_to_utf8]: memory stays one buffer
    /// (see below) instead of the whole `UNOC`…`UNOK` interchange.
    ///
    /// Every byte is transcoded, a leading byte order mark included;
    /// [`decode_reader`] keeps the interchange prologue verbatim.
    ///
    /// # Resource bounds
    ///
    /// As [`DecodingReader`]: one 8 KiB buffer, no limit of its own; the
    /// parser's [`ReaderConfig`][crate::ReaderConfig] limits count decoded bytes.
    ///
    /// # Example
    ///
    /// ```
    /// use edifact::{Charset, from_reader};
    ///
    /// let mut raw = b"UNB+UNOC:3+S+R+200101:0900+1'NAD+BY+M".to_vec();
    /// raw.push(0xFC);
    /// raw.extend_from_slice(b"ller'UNZ+0+1'");
    ///
    /// let reader = Charset::UnoC.decoding_reader(std::io::Cursor::new(raw));
    /// let segments = from_reader(reader).collect::<Result<Vec<_>, _>>()?;
    /// assert_eq!(segments[1].element_str(1), Some("Müller"));
    /// # Ok::<(), edifact::EdifactError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// The reader fails on a byte the repertoire leaves undefined, with
    /// [`EdifactError::InvalidText`] at its offset in the raw input (see
    /// [`DecodingReader`]).
    pub fn decoding_reader<R: Read>(self, reader: R) -> DecodingReader<R> {
        DecodingReader {
            inner: reader,
            charset: self,
            src: Vec::new(),
            src_pos: 0,
            consumed: 0,
            spill: [0; 4],
            spill_len: 0,
            spill_pos: 0,
            verbatim: 0,
            beyond_window: None,
        }
    }
}

impl std::fmt::Display for Charset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.syntax_identifier())
    }
}

/// Adapter that transcodes a single-byte EDIFACT repertoire to UTF-8 on the fly.
///
/// Built by [`Charset::decoding_reader`].
///
/// # Resource bounds
///
/// Holds one 8 KiB source buffer and at most one partly written character;
/// it applies no limit of its own. Bound the parse that reads from it with a
/// [`ReaderConfig`][crate::ReaderConfig] — its limits then count **decoded**
/// bytes, and a Latin-1 interchange can be up to twice as long once decoded.
///
/// # Errors
///
/// A byte the repertoire leaves undefined fails the read with an
/// [`std::io::Error`] carrying [`EdifactError::InvalidText`] at the byte's
/// offset in the raw input; [`EdifactError::from`] unwraps it. Later reads
/// repeat the error.
pub struct DecodingReader<R> {
    inner: R,
    charset: Charset,
    /// Source bytes read but not yet transcoded.
    src: Vec<u8>,
    src_pos: usize,
    /// Source bytes taken so far: the raw offset of the next byte.
    consumed: usize,
    /// A character that did not fit in the caller's buffer on the previous call.
    spill: [u8; 4],
    spill_len: u8,
    spill_pos: u8,
    /// Bytes at the head of the stream copied untranscoded: the prologue.
    verbatim: usize,
    /// While the stream is prologue past the sniff window: the offset of the
    /// next byte. Any byte that is not whitespace is refused.
    beyond_window: Option<usize>,
}

impl<R: Read> Read for DecodingReader<R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        let mut written = 0;

        // Finish the character that straddled the previous call's buffer end.
        while self.spill_pos < self.spill_len && written < out.len() {
            out[written] = self.spill[usize::from(self.spill_pos)];
            self.spill_pos += 1;
            written += 1;
        }
        if self.spill_pos == self.spill_len {
            self.spill_pos = 0;
            self.spill_len = 0;
        }

        loop {
            if written == out.len() {
                return Ok(written);
            }
            if self.src_pos == self.src.len() {
                self.src.clear();
                self.src_pos = 0;
                self.src.resize(8192, 0);
                let n = self.inner.read(&mut self.src)?;
                self.src.truncate(n);
                if n == 0 {
                    return Ok(written);
                }
            }

            let byte = self.src[self.src_pos];
            self.src_pos += 1;
            self.consumed += 1;

            if self.verbatim > 0 {
                self.verbatim -= 1;
                out[written] = byte;
                written += 1;
                continue;
            }
            if let Some(offset) = &mut self.beyond_window {
                if !matches!(byte, b' ' | b'\t' | b'\r' | b'\n') {
                    // Stay on the byte, so every later read refuses it too.
                    self.src_pos -= 1;
                    self.consumed -= 1;
                    if written > 0 {
                        return Ok(written);
                    }
                    return Err(std::io::Error::other(sniff_window_exceeded(*offset)));
                }
                *offset += 1;
            }

            // `UNOY` is already UTF-8, and `UNOA`/`UNOB` are ASCII subsets whose
            // conformant payloads are too — there is nothing to map, and mapping
            // would fail: their high half is empty by definition.  Passing the
            // byte through is both correct and what makes `decode_reader` safe to
            // wrap around an interchange that turns out not to need decoding.
            if byte < 0x80 || self.charset.is_utf8() {
                out[written] = byte;
                written += 1;
                continue;
            }

            let Some(ch) = self.charset.decode_byte(byte) else {
                // Stay on the byte, so every later read refuses it too.
                self.src_pos -= 1;
                self.consumed -= 1;
                if written > 0 {
                    return Ok(written);
                }
                return Err(std::io::Error::other(EdifactError::InvalidText {
                    offset: self.consumed,
                }));
            };
            let mut buf = [0u8; 4];
            let encoded = ch.encode_utf8(&mut buf).as_bytes();
            let room = out.len() - written;
            let direct = room.min(encoded.len());
            out[written..written + direct].copy_from_slice(&encoded[..direct]);
            written += direct;
            // Carry whatever did not fit into the next call.
            if direct < encoded.len() {
                let rest = &encoded[direct..];
                self.spill[..rest.len()].copy_from_slice(rest);
                self.spill_len = rest.len() as u8;
                self.spill_pos = 0;
                return Ok(written);
            }
        }
    }
}

/// Read the `UNB` syntax identifier (S001 DE 0001) straight out of the raw bytes.
///
/// Deliberately byte-level rather than a call into the parser: the whole point is
/// to learn the encoding *before* decoding anything, and running the parser over
/// a `UNOC` interchange can fail on a sender name two elements later. Service
/// characters, segment tags, and DE 0001 itself are ASCII in every repertoire
/// (ISO 9735-1 §6), so this scan is always safe.
///
/// The scan starts after the prologue the parser skips — a UTF-8 byte order
/// mark and any ASCII whitespace — and an optional `UNA`.
///
/// Returns `Ok(None)` when the input carries no `UNB` — an interchange fragment,
/// a bare message, or nothing but prologue.
///
/// # Sniff window
///
/// The prologue may be at most 4 KiB, and the `UNB` syntax identifier must end
/// within the 4 KiB after it. Input that has more before its first segment, or
/// whose identifier the window cuts, is refused rather than passed through
/// undecoded.
///
/// # Errors
///
/// As [`Charset::from_syntax_identifier`], plus [`EdifactError::InvalidUna`] when
/// a `UNA` header is present but malformed, and [`EdifactError::LimitExceeded`]
/// with `limit: "sniff_window"` when the syntax identifier lies outside the
/// sniff window.
///
/// # Example
///
/// ```
/// use edifact::{Charset, sniff_charset};
///
/// let raw = b"\xEF\xBB\xBF\r\nUNB+UNOC:3+S+R+260101:0900+IC1'UNZ+0+IC1'";
/// assert_eq!(sniff_charset(raw)?, Some(Charset::UnoC));
/// # Ok::<(), edifact::EdifactError>(())
/// ```
pub fn sniff_charset(input: &[u8]) -> Result<Option<Charset>, EdifactError> {
    match sniff(input, true)? {
        Sniff::Found { charset, .. } => Ok(Some(charset)),
        Sniff::NoUnb | Sniff::Undecided => Ok(None),
    }
}

/// The size of the sniff window: the prologue may be this long, and the `UNB`
/// syntax identifier must end within this many bytes after it.
///
/// A `UNA` is nine bytes and a `UNB` is a few hundred at the very most, so this
/// is generous by an order of magnitude while still being a fixed, small cost.
const SNIFF_PROBE_BYTES: usize = 4096;

/// What a sniff over the head of an interchange found.
enum Sniff {
    /// `UNB` names `charset`; the first `prologue` bytes are prologue.
    Found { charset: Charset, prologue: usize },
    /// No `UNB`: the input is a fragment, a bare message or only prologue.
    NoUnb,
    /// The head seen so far is prologue (or `UNA` and whitespace) past the
    /// window's end, and more input may follow: any byte that is not
    /// whitespace is refused.
    Undecided,
}

/// [`sniff_charset`] over `input`, which is the whole interchange when
/// `complete`, or else the head of a stream at least `2 * SNIFF_PROBE_BYTES`
/// long.
fn sniff(input: &[u8], complete: bool) -> Result<Sniff, EdifactError> {
    let only_prologue = || {
        if complete {
            Sniff::NoUnb
        } else {
            Sniff::Undecided
        }
    };
    let (prologue, head) = crate::tokenizer::classify_head(input);
    if prologue > SNIFF_PROBE_BYTES {
        return if prologue == input.len() {
            Ok(only_prologue())
        } else {
            Err(sniff_window_exceeded(prologue))
        };
    }
    let ssa = crate::tokenizer::ServiceStringAdvice::from_bytes(input)?;
    let mut pos = prologue;
    if let crate::tokenizer::Head::Una(_) = head {
        pos += 9;
        while pos < input.len() && matches!(input[pos], b' ' | b'\t' | b'\r' | b'\n') {
            pos += 1;
        }
    }
    if pos == input.len() {
        return Ok(only_prologue());
    }
    let window_end = prologue + SNIFF_PROBE_BYTES;
    if pos >= window_end {
        return Err(sniff_window_exceeded(pos));
    }
    // The window ends before the input does: what it cuts is unknown.
    let cut = input.len() >= window_end;
    let window = &input[..input.len().min(window_end)];

    let unb = [b'U', b'N', b'B', ssa.element_sep];
    let seen = &window[pos..window.len().min(pos + 4)];
    if !unb.starts_with(seen) {
        return Ok(Sniff::NoUnb);
    }
    if seen.len() < unb.len() {
        return if cut {
            Err(sniff_window_exceeded(pos))
        } else {
            Ok(Sniff::NoUnb)
        };
    }
    let start = pos + 4;
    let end = match window[start..]
        .iter()
        .position(|&b| b == ssa.component_sep || b == ssa.element_sep || b == ssa.segment_term)
    {
        Some(i) => start + i,
        None if cut => return Err(sniff_window_exceeded(start)),
        None => window.len(),
    };
    let identifier = std::str::from_utf8(&window[start..end])
        .map_err(|_| EdifactError::InvalidText { offset: start })?;
    if identifier.is_empty() {
        return Ok(Sniff::NoUnb);
    }
    Ok(Sniff::Found {
        charset: Charset::from_syntax_identifier(identifier)?,
        prologue,
    })
}

/// The refusal for a syntax identifier outside the sniff window.
fn sniff_window_exceeded(offset: usize) -> EdifactError {
    EdifactError::LimitExceeded {
        limit: "sniff_window",
        max: SNIFF_PROBE_BYTES as u64,
        offset,
    }
}

/// Wrap a reader so it yields UTF-8, reading the repertoire from the stream's own
/// `UNB`.
///
/// The streaming counterpart of [`decode_interchange`], and the one to reach for
/// when the repertoire is not known in advance:
/// [`Charset::decoding_reader`] requires naming it, because a `Read` cannot be
/// rewound after peeking. This buffers the head of the stream, reads `UNB` S001
/// out of it as [`sniff_charset`] does, and chains it back in front of the rest —
/// so nothing is lost, and the memory held does not grow with the interchange.
///
/// The prologue (byte order mark and whitespace) passes through untranscoded.
/// A stream with no `UNB` is passed through unchanged.
///
/// # Resource bounds
///
/// Reads at most 8 KiB before returning — the sniff window of a 4 KiB prologue
/// and the 4 KiB after it — then holds that head and one 8 KiB decoding buffer.
/// It applies no limit of its own: bound the parse
/// with [`from_reader_with_config`][crate::from_reader_with_config], whose
/// limits then count **decoded** bytes (up to twice the raw length for
/// Latin-1).
///
/// # Example
///
/// ```
/// use edifact::{decode_reader, from_reader};
///
/// let mut raw = b"\xEF\xBB\xBFUNB+UNOC:3+S+R+260101:0900+IC1'NAD+BY+M".to_vec();
/// raw.push(0xFC); // `ü` in ISO 8859-1
/// raw.extend_from_slice(b"ller'UNZ+0+IC1'");
///
/// // The repertoire is discovered, not declared by the caller.
/// let segments: Vec<_> = from_reader(decode_reader(std::io::Cursor::new(raw))?)
///     .collect::<Result<_, _>>()?;
/// assert_eq!(segments[1].element_str(1), Some("Müller"));
/// # Ok::<(), edifact::EdifactError>(())
/// ```
///
/// # Errors
///
/// As [`sniff_charset`], plus any I/O error raised while reading the head. The
/// returned reader fails on a byte the repertoire leaves undefined with
/// [`EdifactError::InvalidText`] at its offset in the raw input. When
/// the head is prologue that runs past the sniff window, the sniff-window
/// [`EdifactError::LimitExceeded`] arrives from the returned reader instead, at
/// the first byte that is not whitespace.
#[allow(clippy::type_complexity)]
pub fn decode_reader<R: Read>(
    mut reader: R,
) -> Result<DecodingReader<std::io::Chain<std::io::Cursor<Vec<u8>>, R>>, EdifactError> {
    let mut head = vec![0u8; 2 * SNIFF_PROBE_BYTES];
    let mut filled = 0;
    while filled < head.len() {
        // `read` is free to return short; loop until the head is full or the
        // stream ends, or a `UNB` split across two reads would go unnoticed.
        match reader.read(&mut head[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    let complete = filled < head.len();
    head.truncate(filled);

    let sniffed = sniff(&head, complete)?;
    let seen = head.len();
    let mut decoding = Charset::UnoY.decoding_reader(std::io::Cursor::new(head).chain(reader));
    match sniffed {
        Sniff::Found { charset, prologue } => {
            decoding.charset = charset;
            decoding.verbatim = prologue;
        }
        // Nothing declares a repertoire; pass the bytes through.
        Sniff::NoUnb => {}
        // The head is checked; the guard takes over after it.
        Sniff::Undecided => {
            decoding.verbatim = seen;
            decoding.beyond_window = Some(seen);
        }
    }
    Ok(decoding)
}

/// Decode a whole interchange to UTF-8, reading the repertoire from its own `UNB`.
///
/// The one call to reach for when handling third-party EDIFACT: it is a no-op
/// (and copies nothing) for ASCII and `UNOY` input, and converts `UNOC`…`UNOK`
/// payloads that the rest of the crate would otherwise reject as invalid UTF-8.
///
/// The prologue (byte order mark and whitespace) is kept verbatim, so spans of
/// an ASCII payload equal those [`from_bytes`][crate::from_bytes] reports for the
/// raw input. An input with no `UNB` is returned unchanged, so wrapping a bare
/// message in this call is harmless.
///
/// # Errors
///
/// As [`sniff_charset`] and [`Charset::decode`]; offsets index `input`.
pub fn decode_interchange(input: &[u8]) -> Result<Cow<'_, [u8]>, EdifactError> {
    let Sniff::Found { charset, prologue } = sniff(input, true)? else {
        return Ok(Cow::Borrowed(input));
    };
    transcode_after_prologue(input, charset, prologue)
}

/// [`decode_interchange`] as the decoding parse entry points apply it.
///
/// A payload that is UTF-8 unchanged (`UNOA`, `UNOB`, `UNOY`) is left for the
/// parser to validate, as [`DecodingReader`] leaves it, so the slice and reader
/// paths report the same first error.
pub(crate) fn decode_for_parse(input: &[u8]) -> Result<Cow<'_, [u8]>, EdifactError> {
    match sniff(input, true)? {
        Sniff::Found { charset, prologue } if !charset.is_utf8() => {
            transcode_after_prologue(input, charset, prologue)
        }
        _ => Ok(Cow::Borrowed(input)),
    }
}

/// Transcode `input` after its first `prologue` bytes, which are kept verbatim.
fn transcode_after_prologue(
    input: &[u8],
    charset: Charset,
    prologue: usize,
) -> Result<Cow<'_, [u8]>, EdifactError> {
    let (head, rest) = input.split_at(prologue);
    match charset
        .transcode_to_utf8(rest)
        .map_err(|e| e.rebased(prologue))?
    {
        Cow::Borrowed(_) => Ok(Cow::Borrowed(input)),
        Cow::Owned(text) => Ok(Cow::Owned([head, &text].concat())),
    }
}

/// ISO 8859-2 (Latin-2, Central European) — code points for bytes `0xA0..=0xFF`.
static UNOD_HIGH: [u16; 96] = [
    0x00A0, 0x0104, 0x02D8, 0x0141, 0x00A4, 0x013D, 0x015A, 0x00A7, 0x00A8, 0x0160, 0x015E, 0x0164,
    0x0179, 0x00AD, 0x017D, 0x017B, 0x00B0, 0x0105, 0x02DB, 0x0142, 0x00B4, 0x013E, 0x015B, 0x02C7,
    0x00B8, 0x0161, 0x015F, 0x0165, 0x017A, 0x02DD, 0x017E, 0x017C, 0x0154, 0x00C1, 0x00C2, 0x0102,
    0x00C4, 0x0139, 0x0106, 0x00C7, 0x010C, 0x00C9, 0x0118, 0x00CB, 0x011A, 0x00CD, 0x00CE, 0x010E,
    0x0110, 0x0143, 0x0147, 0x00D3, 0x00D4, 0x0150, 0x00D6, 0x00D7, 0x0158, 0x016E, 0x00DA, 0x0170,
    0x00DC, 0x00DD, 0x0162, 0x00DF, 0x0155, 0x00E1, 0x00E2, 0x0103, 0x00E4, 0x013A, 0x0107, 0x00E7,
    0x010D, 0x00E9, 0x0119, 0x00EB, 0x011B, 0x00ED, 0x00EE, 0x010F, 0x0111, 0x0144, 0x0148, 0x00F3,
    0x00F4, 0x0151, 0x00F6, 0x00F7, 0x0159, 0x016F, 0x00FA, 0x0171, 0x00FC, 0x00FD, 0x0163, 0x02D9,
];

/// ISO 8859-5 (Latin/Cyrillic) — code points for bytes `0xA0..=0xFF`.
static UNOE_HIGH: [u16; 96] = [
    0x00A0, 0x0401, 0x0402, 0x0403, 0x0404, 0x0405, 0x0406, 0x0407, 0x0408, 0x0409, 0x040A, 0x040B,
    0x040C, 0x00AD, 0x040E, 0x040F, 0x0410, 0x0411, 0x0412, 0x0413, 0x0414, 0x0415, 0x0416, 0x0417,
    0x0418, 0x0419, 0x041A, 0x041B, 0x041C, 0x041D, 0x041E, 0x041F, 0x0420, 0x0421, 0x0422, 0x0423,
    0x0424, 0x0425, 0x0426, 0x0427, 0x0428, 0x0429, 0x042A, 0x042B, 0x042C, 0x042D, 0x042E, 0x042F,
    0x0430, 0x0431, 0x0432, 0x0433, 0x0434, 0x0435, 0x0436, 0x0437, 0x0438, 0x0439, 0x043A, 0x043B,
    0x043C, 0x043D, 0x043E, 0x043F, 0x0440, 0x0441, 0x0442, 0x0443, 0x0444, 0x0445, 0x0446, 0x0447,
    0x0448, 0x0449, 0x044A, 0x044B, 0x044C, 0x044D, 0x044E, 0x044F, 0x2116, 0x0451, 0x0452, 0x0453,
    0x0454, 0x0455, 0x0456, 0x0457, 0x0458, 0x0459, 0x045A, 0x045B, 0x045C, 0x00A7, 0x045E, 0x045F,
];

/// ISO 8859-7 (Latin/Greek) — code points for bytes `0xA0..=0xFF`.
static UNOF_HIGH: [u16; 96] = [
    0x00A0, 0x2018, 0x2019, 0x00A3, 0x20AC, 0x20AF, 0x00A6, 0x00A7, 0x00A8, 0x00A9, 0x037A, 0x00AB,
    0x00AC, 0x00AD, 0xFFFF, 0x2015, 0x00B0, 0x00B1, 0x00B2, 0x00B3, 0x0384, 0x0385, 0x0386, 0x00B7,
    0x0388, 0x0389, 0x038A, 0x00BB, 0x038C, 0x00BD, 0x038E, 0x038F, 0x0390, 0x0391, 0x0392, 0x0393,
    0x0394, 0x0395, 0x0396, 0x0397, 0x0398, 0x0399, 0x039A, 0x039B, 0x039C, 0x039D, 0x039E, 0x039F,
    0x03A0, 0x03A1, 0xFFFF, 0x03A3, 0x03A4, 0x03A5, 0x03A6, 0x03A7, 0x03A8, 0x03A9, 0x03AA, 0x03AB,
    0x03AC, 0x03AD, 0x03AE, 0x03AF, 0x03B0, 0x03B1, 0x03B2, 0x03B3, 0x03B4, 0x03B5, 0x03B6, 0x03B7,
    0x03B8, 0x03B9, 0x03BA, 0x03BB, 0x03BC, 0x03BD, 0x03BE, 0x03BF, 0x03C0, 0x03C1, 0x03C2, 0x03C3,
    0x03C4, 0x03C5, 0x03C6, 0x03C7, 0x03C8, 0x03C9, 0x03CA, 0x03CB, 0x03CC, 0x03CD, 0x03CE, 0xFFFF,
];

/// ISO 8859-3 (Latin-3, South European) — code points for bytes `0xA0..=0xFF`.
static UNOG_HIGH: [u16; 96] = [
    0x00A0, 0x0126, 0x02D8, 0x00A3, 0x00A4, 0xFFFF, 0x0124, 0x00A7, 0x00A8, 0x0130, 0x015E, 0x011E,
    0x0134, 0x00AD, 0xFFFF, 0x017B, 0x00B0, 0x0127, 0x00B2, 0x00B3, 0x00B4, 0x00B5, 0x0125, 0x00B7,
    0x00B8, 0x0131, 0x015F, 0x011F, 0x0135, 0x00BD, 0xFFFF, 0x017C, 0x00C0, 0x00C1, 0x00C2, 0xFFFF,
    0x00C4, 0x010A, 0x0108, 0x00C7, 0x00C8, 0x00C9, 0x00CA, 0x00CB, 0x00CC, 0x00CD, 0x00CE, 0x00CF,
    0xFFFF, 0x00D1, 0x00D2, 0x00D3, 0x00D4, 0x0120, 0x00D6, 0x00D7, 0x011C, 0x00D9, 0x00DA, 0x00DB,
    0x00DC, 0x016C, 0x015C, 0x00DF, 0x00E0, 0x00E1, 0x00E2, 0xFFFF, 0x00E4, 0x010B, 0x0109, 0x00E7,
    0x00E8, 0x00E9, 0x00EA, 0x00EB, 0x00EC, 0x00ED, 0x00EE, 0x00EF, 0xFFFF, 0x00F1, 0x00F2, 0x00F3,
    0x00F4, 0x0121, 0x00F6, 0x00F7, 0x011D, 0x00F9, 0x00FA, 0x00FB, 0x00FC, 0x016D, 0x015D, 0x02D9,
];

/// ISO 8859-4 (Latin-4, North European) — code points for bytes `0xA0..=0xFF`.
static UNOH_HIGH: [u16; 96] = [
    0x00A0, 0x0104, 0x0138, 0x0156, 0x00A4, 0x0128, 0x013B, 0x00A7, 0x00A8, 0x0160, 0x0112, 0x0122,
    0x0166, 0x00AD, 0x017D, 0x00AF, 0x00B0, 0x0105, 0x02DB, 0x0157, 0x00B4, 0x0129, 0x013C, 0x02C7,
    0x00B8, 0x0161, 0x0113, 0x0123, 0x0167, 0x014A, 0x017E, 0x014B, 0x0100, 0x00C1, 0x00C2, 0x00C3,
    0x00C4, 0x00C5, 0x00C6, 0x012E, 0x010C, 0x00C9, 0x0118, 0x00CB, 0x0116, 0x00CD, 0x00CE, 0x012A,
    0x0110, 0x0145, 0x014C, 0x0136, 0x00D4, 0x00D5, 0x00D6, 0x00D7, 0x00D8, 0x0172, 0x00DA, 0x00DB,
    0x00DC, 0x0168, 0x016A, 0x00DF, 0x0101, 0x00E1, 0x00E2, 0x00E3, 0x00E4, 0x00E5, 0x00E6, 0x012F,
    0x010D, 0x00E9, 0x0119, 0x00EB, 0x0117, 0x00ED, 0x00EE, 0x012B, 0x0111, 0x0146, 0x014D, 0x0137,
    0x00F4, 0x00F5, 0x00F6, 0x00F7, 0x00F8, 0x0173, 0x00FA, 0x00FB, 0x00FC, 0x0169, 0x016B, 0x02D9,
];

/// ISO 8859-6 (Latin/Arabic) — code points for bytes `0xA0..=0xFF`.
static UNOI_HIGH: [u16; 96] = [
    0x00A0, 0xFFFF, 0xFFFF, 0xFFFF, 0x00A4, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF,
    0x060C, 0x00AD, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF,
    0xFFFF, 0xFFFF, 0xFFFF, 0x061B, 0xFFFF, 0xFFFF, 0xFFFF, 0x061F, 0xFFFF, 0x0621, 0x0622, 0x0623,
    0x0624, 0x0625, 0x0626, 0x0627, 0x0628, 0x0629, 0x062A, 0x062B, 0x062C, 0x062D, 0x062E, 0x062F,
    0x0630, 0x0631, 0x0632, 0x0633, 0x0634, 0x0635, 0x0636, 0x0637, 0x0638, 0x0639, 0x063A, 0xFFFF,
    0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0x0640, 0x0641, 0x0642, 0x0643, 0x0644, 0x0645, 0x0646, 0x0647,
    0x0648, 0x0649, 0x064A, 0x064B, 0x064C, 0x064D, 0x064E, 0x064F, 0x0650, 0x0651, 0x0652, 0xFFFF,
    0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF,
];

/// ISO 8859-8 (Latin/Hebrew) — code points for bytes `0xA0..=0xFF`.
static UNOJ_HIGH: [u16; 96] = [
    0x00A0, 0xFFFF, 0x00A2, 0x00A3, 0x00A4, 0x00A5, 0x00A6, 0x00A7, 0x00A8, 0x00A9, 0x00D7, 0x00AB,
    0x00AC, 0x00AD, 0x00AE, 0x00AF, 0x00B0, 0x00B1, 0x00B2, 0x00B3, 0x00B4, 0x00B5, 0x00B6, 0x00B7,
    0x00B8, 0x00B9, 0x00F7, 0x00BB, 0x00BC, 0x00BD, 0x00BE, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF,
    0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF,
    0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF,
    0xFFFF, 0xFFFF, 0xFFFF, 0x2017, 0x05D0, 0x05D1, 0x05D2, 0x05D3, 0x05D4, 0x05D5, 0x05D6, 0x05D7,
    0x05D8, 0x05D9, 0x05DA, 0x05DB, 0x05DC, 0x05DD, 0x05DE, 0x05DF, 0x05E0, 0x05E1, 0x05E2, 0x05E3,
    0x05E4, 0x05E5, 0x05E6, 0x05E7, 0x05E8, 0x05E9, 0x05EA, 0xFFFF, 0xFFFF, 0x200E, 0x200F, 0xFFFF,
];

/// ISO 8859-9 (Latin-5, Turkish) — code points for bytes `0xA0..=0xFF`.
static UNOK_HIGH: [u16; 96] = [
    0x00A0, 0x00A1, 0x00A2, 0x00A3, 0x00A4, 0x00A5, 0x00A6, 0x00A7, 0x00A8, 0x00A9, 0x00AA, 0x00AB,
    0x00AC, 0x00AD, 0x00AE, 0x00AF, 0x00B0, 0x00B1, 0x00B2, 0x00B3, 0x00B4, 0x00B5, 0x00B6, 0x00B7,
    0x00B8, 0x00B9, 0x00BA, 0x00BB, 0x00BC, 0x00BD, 0x00BE, 0x00BF, 0x00C0, 0x00C1, 0x00C2, 0x00C3,
    0x00C4, 0x00C5, 0x00C6, 0x00C7, 0x00C8, 0x00C9, 0x00CA, 0x00CB, 0x00CC, 0x00CD, 0x00CE, 0x00CF,
    0x011E, 0x00D1, 0x00D2, 0x00D3, 0x00D4, 0x00D5, 0x00D6, 0x00D7, 0x00D8, 0x00D9, 0x00DA, 0x00DB,
    0x00DC, 0x0130, 0x015E, 0x00DF, 0x00E0, 0x00E1, 0x00E2, 0x00E3, 0x00E4, 0x00E5, 0x00E6, 0x00E7,
    0x00E8, 0x00E9, 0x00EA, 0x00EB, 0x00EC, 0x00ED, 0x00EE, 0x00EF, 0x011F, 0x00F1, 0x00F2, 0x00F3,
    0x00F4, 0x00F5, 0x00F6, 0x00F7, 0x00F8, 0x00F9, 0x00FA, 0x00FB, 0x00FC, 0x0131, 0x015F, 0x00FF,
];
