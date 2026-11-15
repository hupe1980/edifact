+++
title = "Character Sets"
description = "Decode UNOA-UNOK and UNOY interchanges, encode them back, and validate a payload against the repertoire its own UNB declares."
weight = 25
+++

# Character Sets

`UNB` S001 component 1, the **syntax identifier**, names the interchange's
repertoire. Service characters, tags and the identifier are ASCII in every
repertoire, so the header is readable before the encoding is known.

## Why decode

**UTF-8 is not a superset of `UNOC`.** In ISO 8859-1, `ü` is the single byte
`0xFC`, which is invalid UTF-8. A conformant `UNOC` interchange carrying
`Müller` fails the UTF-8 parser:

```rust
use edifact::from_bytes;

let mut raw = b"UNB+UNOC:3+S+R+260101:0900+IC1'NAD+BY+M".to_vec();
raw.push(0xFC);                              // `ü` in ISO 8859-1
raw.extend_from_slice(b"ller'UNZ+0+IC1'");

// Straight through the parser: E003, invalid text.
assert!(from_bytes(&raw).collect::<Result<Vec<_>, _>>().is_err());
```

The same holds for `UNOD`…`UNOK`. `edifact` is UTF-8 throughout: decode at
the boundary.

## Decoding a whole interchange

`decode_interchange` reads the repertoire from the interchange's own `UNB` and
converts the payload to UTF-8:

```rust
use edifact::{decode_interchange, from_bytes};

let mut raw = b"UNB+UNOC:3+S+R+260101:0900+IC1'NAD+BY+M".to_vec();
raw.push(0xFC);
raw.extend_from_slice(b"ller'UNZ+0+IC1'");

let utf8 = decode_interchange(&raw)?;
let segments: Vec<_> = from_bytes(&utf8).collect::<Result<Vec<_>, _>>()?;

assert_eq!(segments[1].element_str(1), Some("Müller"));
# Ok::<(), edifact::EdifactError>(())
```

Call it unconditionally: an ASCII payload, a `UNOY` payload and input without
`UNB` come back **borrowed**, nothing copied:

```rust
use edifact::decode_interchange;
use std::borrow::Cow;

let ascii = b"UNB+UNOC:3+S+R+260101:0900+IC1'BGM+220'UNZ+0+IC1'";
assert!(matches!(decode_interchange(ascii)?, Cow::Borrowed(_)));
# Ok::<(), edifact::EdifactError>(())
```

**Spans index the decoded buffer.** Transcoding turns `0xFC` into two bytes, so
every downstream `Span` points into the buffer `decode_interchange` returned.

## Byte order mark and leading whitespace

The repertoire is read after the prologue the parser skips — a UTF-8 byte order
mark and whitespace — which is kept verbatim:

```rust
use edifact::{Charset, from_bytes_decoded, sniff_charset};

let mut raw = b"\xEF\xBB\xBF\r\nUNB+UNOC:3+S+R+260101:0900+IC1'NAD+BY+M".to_vec();
raw.push(0xFC);
raw.extend_from_slice(b"ller'UNZ+0+IC1'");

assert_eq!(sniff_charset(&raw)?, Some(Charset::UnoC));
assert_eq!(from_bytes_decoded(&raw)?[1].element_str(1), Some("Müller"));
# Ok::<(), edifact::EdifactError>(())
```

The **sniff window** is fixed: at most 4 KiB of prologue, then the `UNB` syntax
identifier within the next 4 KiB (a `UNA` and whitespace may precede it). A
header outside the window is
[`E036`](@/docs/error-reference.md#e036-limitexceeded) with
`limit: "sniff_window"` on every decoding entry point, never passed through
undecoded. Input that is only prologue passes through. When `max_input_bytes`
ends inside the prologue, the `*_decoded_with_config` functions report that
limit instead.

`Charset::decode`, `transcode_to_utf8` and `decoding_reader` transcode every
byte they are given, a leading byte order mark included.

## Decoding a stream

`decode_reader` reads at most 8 KiB (the sniff window) to find the `UNB`, then
streams the rest through one 8 KiB buffer. It applies no limit of its own;
`from_reader_with_config` limits count decoded bytes:

```rust
use edifact::{decode_reader, from_reader};

let mut raw = b"UNB+UNOC:3+S+R+260101:0900+IC1'NAD+BY+M".to_vec();
raw.push(0xFC);
raw.extend_from_slice(b"ller'UNZ+0+IC1'");

let segments: Vec<_> = from_reader(decode_reader(std::io::Cursor::new(raw))?)
    .collect::<Result<_, _>>()?;

assert_eq!(segments[1].element_str(1), Some("Müller"));
# Ok::<(), edifact::EdifactError>(())
```

UTF-8 streams and streams without `UNB` pass through unchanged. When the
repertoire is known in advance, `Charset::decoding_reader` skips the sniff:

```rust
use edifact::{Charset, from_reader};

# let mut raw = b"UNB+UNOC:3+S+R+260101:0900+IC1'NAD+BY+M".to_vec();
# raw.push(0xFC);
# raw.extend_from_slice(b"ller'UNZ+0+IC1'");
let reader = Charset::UnoC.decoding_reader(std::io::Cursor::new(raw));
let segments: Vec<_> = from_reader(reader).collect::<Result<_, _>>()?;
# assert_eq!(segments[1].element_str(1), Some("Müller"));
# Ok::<(), edifact::EdifactError>(())
```

## Writing

A `UNOC` header over a UTF-8 body arrives as mojibake. Bind the writer to the
repertoire:

```rust
use edifact::{Charset, Writer};

let mut writer = Writer::new(Vec::new()).with_charset(Charset::UnoC);
writer.write_composites("NAD", &[&["BY"], &["Müller"]])?;

// `ü` goes out as the single Latin-1 byte 0xFC.
assert_eq!(writer.finish()?, b"NAD+BY+M\xFCller'".to_vec());
# Ok::<(), edifact::EdifactError>(())
```

A character the repertoire cannot carry is refused
([`E038`](@/docs/error-reference.md#e038-characternotinrepertoire)):

```rust
use edifact::{Charset, EdifactError, Writer};

let mut writer = Writer::new(Vec::new()).with_charset(Charset::UnoA);
// Level A is upper-case only.
let err = writer.write_composites("NAD", &[&["BY"], &["Müller"]]).unwrap_err();
assert!(matches!(err, EdifactError::CharacterNotInRepertoire { .. }));
```

A `UNB` declaring a repertoire other than the writer's is refused
([`E041`](@/docs/error-reference.md#e041-characterrepertoiremismatch)).
`begin_interchange` binds an unbound writer to its header's repertoire.

## Validating the payload against the header

Partners reject a `UNOA` interchange carrying a lower-case letter.
`CharsetValidator` finds it before you send:

```rust
use edifact::{ValidationContext, from_bytes};

// UNOA is upper-case only, but the party name is mixed case.
let segments: Vec<_> =
    from_bytes(b"UNB+UNOA:3+S+R+260101:0900+IC1'NAD+BY+Acme Ltd'UNZ+0+IC1'")
        .collect::<Result<_, _>>()?;

let report = ValidationContext::builder()
    .with_charset_validation()   // reads the repertoire from UNB S001
    .build()
    .validate(&segments);

let issue = report.errors().iter().find(|i| i.error_code() == Some("E038")).unwrap();
assert_eq!(issue.segment_tag.as_deref(), Some("NAD"));
# Ok::<(), edifact::EdifactError>(())
```

`with_charset_validation_for(Charset::UnoA)` pins a repertoire — for slices
without `UNB`, or a partner stricter than its header.

**Decoding is permissive; validation is strict.** `decode_interchange` treats
`UNOA` and `UNOB` as ASCII, so one stray character does not hide every other
finding behind a parse error. The repertoire check is a separate, suppressible
issue.

## The repertoires

| Identifier | Encoding | Notes |
|---|---|---|
| `UNOA` | ISO 9735 level A | `A`–`Z`, `0`–`9`, space, `.,-()/='+:?!"%&*;<>` |
| `UNOB` | ISO 9735 level B | Level A plus `a`–`z` |
| `UNOC` | ISO 8859-1 | Latin-1 |
| `UNOD` | ISO 8859-2 | Latin-2, Central European |
| `UNOE` | ISO 8859-5 | Latin/Cyrillic |
| `UNOF` | ISO 8859-7 | Latin/Greek |
| `UNOG` | ISO 8859-3 | Latin-3, South European |
| `UNOH` | ISO 8859-4 | Latin-4, North European |
| `UNOI` | ISO 8859-6 | Latin/Arabic |
| `UNOJ` | ISO 8859-8 | Latin/Hebrew |
| `UNOK` | ISO 8859-9 | Latin-5, Turkish |
| `UNOY` | ISO 10646-1 | UTF-8 — the identity transform |

`UNOX` (ISO/IEC 2022) and `KECA` are **not** supported: their stateful or
multi-byte encodings break byte-level delimiter scanning. They report
[`E039`](@/docs/error-reference.md#e039-unsupportedcharset).

Syntax versions `1`–`4` are supported in `UNB` S001 DE 0002; any other value is
[`E059`](@/docs/error-reference.md#e059-unsupportedsyntaxversion) from envelope
validation.

A byte the repertoire leaves undefined (ISO 8859-6 has 45) is an error, never a
replacement character. Every decoding entry point
(`Charset::decode`, `decoding_reader`, `decode_reader`, `from_bytes_decoded`,
`from_reader_decoded`) reports [`E003`](@/docs/error-reference.md#e003-invalidtext)
at the byte's offset in the raw input; the streaming paths yield the segments
before it first.

```rust
use edifact::{Charset, EdifactError};
use std::io::Read;

// 0xA1 is undefined in ISO 8859-6.
let err = Charset::UnoI.decode(b"AB\xA1CD").unwrap_err();
assert!(matches!(err, EdifactError::InvalidText { offset: 2 }));

let mut out = Vec::new();
let err = Charset::UnoI
    .decoding_reader(&b"AB\xA1CD"[..])
    .read_to_end(&mut out)
    .unwrap_err();
assert!(matches!(EdifactError::from(err), EdifactError::InvalidText { offset: 2 }));
```

## Checking a value before you build a message

`Charset::permits` and `Charset::first_violation` check without writing — for
when the fix is to transliterate:

```rust
use edifact::Charset;

assert!(Charset::UnoA.permits('A'));
assert!(!Charset::UnoA.permits('a'));

// Byte offset of the first character that would be rejected.
assert_eq!(Charset::UnoA.first_violation("ORDER"), None);
assert_eq!(Charset::UnoA.first_violation("ORDer"), Some((3, 'e')));
```

---

## Further reading

- [Core Concepts](@/docs/core-concepts.md) — the wire format and the UNA service string
- [Writing](@/docs/writing.md) — the `Writer` and delimiter escaping
- [Validation](@/docs/validation.md) — the layered validation pipeline
- [Error Reference](@/docs/error-reference.md) — `E038`, `E039`, `E041`

