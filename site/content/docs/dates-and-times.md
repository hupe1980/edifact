+++
title = "Dates and times"
description = "Decode, compare, build and validate EDIFACT DTM values by their UNCL 2379 format code, with optional conversions to the time crate."
weight = 45
+++

A `DTM` segment carries C507: a qualifier (DE 2005), a value (DE 2380) and a
format code (DE 2379). The format code decides how to read the value:
`202610010000` is a minute under `203` and nonsense under `102`. The `dtm`
feature decodes the value by its code, writes it back byte for byte, and
compares values across formats.

```bash
cargo add edifact --features dtm        # codec, no dependencies
cargo add edifact --features dtm-time   # plus conversions to `time` 0.3
```

---

## Decoding

From a segment, found by qualifier:

```rust
use edifact::{Dtm, SegmentSliceExt};

let segments: Vec<_> = edifact::from_bytes(b"DTM+137:202610011200?+00:303'")
    .collect::<Result<_, _>>()?;

let dtm = Dtm::from_segment(segments.qualified("DTM", "137").unwrap())?;
assert_eq!(dtm.qualifier, "137");
assert_eq!(dtm.value.code(), "303");
assert_eq!((dtm.value.hour(), dtm.value.offset_hours()), (Some(12), Some(0)));
assert_eq!(dtm.value.to_string(), "202610011200+00"); // unescaped, as written
# Ok::<(), Box<dyn std::error::Error>>(())
```

From a bare value:

```rust
use edifact::{DtmError, DtmFormat, DtmValue};

let v = DtmValue::parse("20261001", "102")?;
assert_eq!((v.year(), v.month(), v.day()), (Some(2026), Some(10), Some(1)));

let err = DtmValue::parse("20180132", "102").unwrap_err();
assert_eq!(err.to_string(), "value does not match format 102 (CCYYMMDD): day out of range at position 6");

assert_eq!(
    DtmValue::parse("20261001", "101"),
    Err(DtmError::UnsupportedFormat { code: "101".into() }),
);
assert_eq!(DtmValue::parse_as("0229", DtmFormat::Mmdd)?.day(), Some(29));
# Ok::<(), DtmError>(())
```

Every malformed value is a `DtmError` with a position; nothing panics, whatever
the bytes. Every accepted value formats back to exactly its input.

## Supported codes

| Code | Layout | Code | Layout |
|---|---|---|---|
| `102` | `CCYYMMDD` | `501` | `HHMMHHMM` |
| `106` | `MMDD` | `602` | `CCYY` |
| `203` | `CCYYMMDDHHMM` | `610` | `CCYYMM` |
| `204` | `CCYYMMDDHHMMSS` | `718` | `CCYYMMDD-CCYYMMDD` |
| `303` | `CCYYMMDDHHMMZZZ` | `719` | `CCYYMMDDHHMM-CCYYMMDDHHMM` |
| `304` | `CCYYMMDDHHMMSSZZZ` | `802`–`805` | months, weeks, days, hours |
| `401` | `HHMM` | `104` | `MMWWMMWW`, weeks of months |

Other codes, two-digit years included, are `UnsupportedFormat`. Where UNCL 2379
is silent, the codec reads:

- `ZZZ` as a sign and two hour digits, at most 14 (`+00`, `-05`); `-00` stays
  `-00`.
- Periods without the hyphen (`201801010500201801020500`); a hyphen is refused.
  `718` includes its end day; `719` and `501` are `[start, end)`.
- `104` weeks as `01`–`05`, with no year; the end week is included.
- Hour `24` with zero minutes as end of day, kept as written.
- Counts as canonical `i32` (`0`, `-5`; not `+1`, `01`, `-0`).

```rust
use edifact::DtmValue;

let gas_day = DtmValue::parse("201801010500201801020500", "719")?;
assert_eq!(gas_day.start().unwrap().to_string(), "201801010500");
assert!(gas_day.is_forward());
assert!(!DtmValue::parse("2026103120261001", "718")?.is_forward()); // kept, not swapped
assert_eq!(DtmValue::parse("0", "805")?.count(), Some(0));
# Ok::<(), edifact::DtmError>(())
```

## Zones and floating values

`303` and `304` carry a zone. Every other value is *floating*: a wall clock
with no offset. The codec never assumes one. `with_offset` attaches an offset
to a `203`/`204` value, giving its `303`/`304` twin:

```rust
use edifact::DtmValue;

let v = DtmValue::parse("202610011200", "203")?.with_offset(0)?;
assert_eq!((v.code(), v.to_string().as_str()), ("303", "202610011200+00"));
# Ok::<(), edifact::DtmError>(())
```

## Comparing

`compare` treats each value as the interval it denotes (a `102` is a day, a
`203` a minute, a `719` its extent) and returns a `DtmRelation`: `Before`,
`After`, `Equal`, `Overlapping` or `Indeterminate`. It never compares strings.

`Indeterminate` covers counts, inverted or empty periods, values on different
timelines (`106`, `401`/`501`, dates), and floating vs zoned values whose
relation depends on the offset. A floating value is tried at −14 h and +14 h;
only if both agree on `Before` or `After` is that the answer.
`compare_assuming` supplies the offset instead.

`104` values compare only with other `104` values, on month and week; against
any other code the answer is `Indeterminate`, because which days a week holds
is the implementation guide's convention:

```rust
use edifact::{DtmRelation, DtmValue};

let june = |v| DtmValue::parse(v, "104").unwrap();
assert_eq!(june("06010602").compare(&june("06030604")), DtmRelation::Before);
let day = DtmValue::parse("20260601", "102")?;
assert_eq!(june("06010602").compare(&day), DtmRelation::Indeterminate);
# Ok::<(), edifact::DtmError>(())
```

A period check accepts only `Before`; everything else is a refusal:

```rust
use edifact::{DtmRelation, DtmValue};

fn period_is_ordered(start: &DtmValue, end: &DtmValue) -> bool {
    match start.compare(end) {
        DtmRelation::Before => true,
        // After, Equal, Overlapping, Indeterminate
        _ => false,
    }
}

let p = |v, c| DtmValue::parse(v, c).unwrap();

// 22:00 UTC is before 23:00 UTC, though "202610010000+02" > "202609302300+00".
assert!(period_is_ordered(&p("202610010000+02", "303"), &p("202609302300+00", "303")));
// The minute lies inside the day: Overlapping, refused.
assert!(!period_is_ordered(&p("202610010000", "203"), &p("20261001", "102")));

// Floating vs zoned, an hour apart: depends on the zone.
let (start, end) = (p("202610011200", "203"), p("202610011100+00", "303"));
assert_eq!(start.compare(&end), DtmRelation::Indeterminate);
assert!(!period_is_ordered(&start, &end));
// With the guide's "all times are UTC" rule:
assert_eq!(start.compare_assuming(&end, 0), DtmRelation::After);
```

`==` on `DtmValue` is "same as written": `…2400` is not `==` to the next
`…0000`, but `compare` says `Equal`.

## Building

`Dtm::components()` gives `[qualifier, value, code]` for `write_composites`;
`Dtm::to_segment()` gives an owned `DTM` segment. The writer escapes `+`:

```rust
use edifact::{Dtm, DtmValue, MessageBuilder, MessageIdentifier, Writer};

let dtm = Dtm::new("137", DtmValue::parse("202610011200", "203")?.with_offset(0)?);

let mut w = Writer::new(Vec::new());
w.write_composites("DTM", &[dtm.components()])?;
assert_eq!(w.finish()?, b"DTM+137:202610011200?+00:303'");

let mut msg = MessageBuilder::new(MessageIdentifier::new("1", "UTILMD", "D", "11A", "UN"))?;
msg.write_composites("DTM", &[dtm.components()])?;
msg.write_segment(&dtm.to_segment())?;
let segments = msg.finish()?;
assert_eq!(segments[1], segments[2]);
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Validating

`DtmValidator` reports a value that contradicts its code as an Error `E027`
(CONTRL `12`) on component 1, and an unsupported code as a Warning on
component 2: that value was not checked. A missing value or code gives no
finding. Which qualifier needs which format is the guide's rule.

```rust
use edifact::{DtmValidator, SyntaxError, ValidationContext, ValidationLayer};

let segments: Vec<_> = edifact::from_bytes(b"DTM+137:20260701:303'DTM+2:x:999'")
    .collect::<Result<_, _>>()?;
let report = ValidationContext::builder()
    .with_validator(ValidationLayer::CodeList, DtmValidator)
    .build()
    .validate(&segments);

assert_eq!(report.errors()[0].error_code(), Some("E027"));
assert_eq!(SyntaxError::for_issue(&report.errors()[0]).code(), "12");
assert_eq!(report.warnings()[0].component_index, Some(2));
# Ok::<(), edifact::EdifactError>(())
```

## `time` conversions

With `dtm-time`:

| Method | Formats | Result |
|---|---|---|
| `to_date` | `102` | `time::Date` |
| `to_primitive_date_time` | `203`, `204` | `PrimitiveDateTime` (`2400` → next midnight) |
| `to_offset_date_time` | `303`, `304` | `OffsetDateTime` |
| `DtmValue::try_from(Date)` | → `102` | |
| `from_primitive(dt, format)` | → `102`, `203`, `204` | truncated to the format |
| `from_offset_date_time(dt, format)` | → `303`, `304` | whole-hour offset within ±14 h |

```rust
use edifact::{DtmFormat, DtmValue};
use time::{OffsetDateTime, UtcOffset};

let at = DtmValue::parse("202610011200+02", "303")?.to_offset_date_time()?;
assert_eq!(at.to_offset(UtcOffset::UTC).hour(), 10);

// A floating value needs an offset from you.
let local = DtmValue::parse("202610011200", "203")?;
assert!(local.to_offset_date_time().is_err());
let at = local.to_primitive_date_time()?.assume_offset(UtcOffset::UTC);
assert_eq!(at.hour(), 12);

// A send timestamp: pass `OffsetDateTime::now_utc()`.
let now = OffsetDateTime::from_unix_timestamp(1_790_856_045).unwrap(); // 12:00:45 UTC
let v = DtmValue::from_offset_date_time(now, DtmFormat::CcyymmddhhmmZzz)?;
assert_eq!(v.to_string(), "202610011200+00");
# Ok::<(), edifact::DtmError>(())
```

## Out of scope

- Time-zone databases, DST and named zones.
- Two-digit-year and year-week codes (`101`, `616`, …).
- Guessing a format code from a value.
- Domain rules: which qualifier needs which format, or when a business day
  begins.
