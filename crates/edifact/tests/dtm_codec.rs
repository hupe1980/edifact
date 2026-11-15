//! UNCL 2379 codec: table cases per code, round-trip properties and builders.

use edifact::{Dtm, DtmError, DtmFormat, DtmValue, MessageBuilder, MessageIdentifier, Writer};

const CODES: [&str; 17] = [
    "102", "104", "106", "203", "204", "303", "304", "401", "501", "602", "610", "718", "719",
    "802", "803", "804", "805",
];

/// At least five valid values per code; each must format back unchanged.
const VALID: &[(&str, &[&str])] = &[
    (
        "102",
        &["20261001", "20240229", "20000229", "00010101", "99991231"],
    ),
    (
        "104",
        &["05040504", "06030604", "08010804", "01011205", "05040503"],
    ),
    ("106", &["0101", "0229", "1231", "0430", "0601"]),
    (
        "203",
        &[
            "202610011200",
            "201801012400",
            "000101010000",
            "999912312359",
            "202402291530",
        ],
    ),
    (
        "204",
        &[
            "20261001120045",
            "20180101240000",
            "00010101000000",
            "99991231235959",
            "20240229000059",
        ],
    ),
    (
        "303",
        &[
            "202610011200+00",
            "202610011200-00",
            "202610010000+02",
            "201801012400-05",
            "999912312359+14",
        ],
    ),
    (
        "304",
        &[
            "20261001120045+00",
            "20261001120045-14",
            "20180101240000+01",
            "00010101000000+00",
            "20240229235959-03",
        ],
    ),
    ("401", &["0000", "2359", "2400", "1200", "0530"]),
    (
        "501",
        &["08001700", "17000800", "00002400", "12001200", "05300600"],
    ),
    ("602", &["0001", "2026", "9999", "1900", "2000"]),
    ("610", &["202610", "000101", "999912", "202402", "190002"]),
    (
        "718",
        &[
            "2026100120261031",
            "2026103120261001",
            "2026100120261001",
            "9999123199991231",
            "0001010100010101",
        ],
    ),
    (
        "719",
        &[
            "201801010500201801020500",
            "201801020500201801010500",
            "201801010000201801012400",
            "999912312359999912312400",
            "000101010000000101010001",
        ],
    ),
    ("802", &["0", "1", "-1", "12", "2147483647"]),
    ("803", &["0", "52", "-52", "-2147483648", "7"]),
    ("804", &["0", "1", "365", "-30", "1000000"]),
    ("805", &["0", "1", "24", "-24", "8760"]),
];

/// At least five invalid values per code, with the expected error position.
const INVALID: &[(&str, &[(&str, usize)])] = &[
    (
        "104",
        &[
            ("1001", 4),
            ("13010101", 0),
            ("05000501", 2),
            ("05060506", 2),
            ("0504050A", 7),
            ("0504-0504", 4),
        ],
    ),
    (
        "102",
        &[
            ("20180132", 6),
            ("201801011200", 8),
            ("2018-01-01", 4),
            ("2018ü101", 4),
            ("20181301", 4),
            ("20180100", 6),
            ("00000101", 0),
            ("20260229", 6),
            ("19000229", 6),
            ("20260230", 6),
            ("", 0),
        ],
    ),
    (
        "106",
        &[
            ("0230", 2),
            ("1301", 0),
            ("0001", 0),
            ("0100", 2),
            ("01011", 4),
            ("ü101", 0),
        ],
    ),
    (
        "203",
        &[
            ("201801012500", 8),
            ("201801011260", 10),
            ("201801012401", 10),
            ("20180101120", 11),
            ("2018010112000", 12),
            ("20180101 200", 8),
        ],
    ),
    (
        "204",
        &[
            ("20180101120060", 12),
            ("20180101240001", 12),
            ("20180101240100", 10),
            ("2018010112000", 13),
            ("201801011200000", 14),
        ],
    ),
    (
        "303",
        &[
            ("202610011200+15", 13),
            ("202610011200+0530", 15),
            ("202610011200 00", 12),
            ("202610011200", 12),
            ("202610011200+0", 14),
            ("202610011200Z00", 12),
            ("202610011200?+00", 12),
        ],
    ),
    (
        "304",
        &[
            ("20261001120045+15", 15),
            ("20261001120060+00", 12),
            ("20261001120045", 14),
            ("20261001120045*00", 14),
            ("20261001120045+1a", 16),
        ],
    ),
    (
        "401",
        &[
            ("2401", 2),
            ("2500", 0),
            ("1260", 2),
            ("120", 3),
            ("12000", 4),
        ],
    ),
    (
        "501",
        &[
            ("0800-1700", 4),
            ("08001760", 6),
            ("2500 800", 4),
            ("25000800", 0),
            ("0800170", 7),
            ("080017000", 8),
        ],
    ),
    (
        "602",
        &[("0000", 0), ("202", 3), ("20266", 4), ("20a6", 2), ("ü", 0)],
    ),
    (
        "610",
        &[
            ("202613", 4),
            ("202600", 4),
            ("000001", 0),
            ("2026-10", 4),
            ("20261", 5),
        ],
    ),
    (
        "718",
        &[
            ("20261001-20261031", 8),
            ("2026100120261032", 14),
            ("202610012026103", 15),
            ("20261001202610311", 16),
            ("2026100120261331", 12),
        ],
    ),
    (
        "719",
        &[
            ("201801010500-201801020500", 12),
            ("201801010500201801022500", 20),
            ("20180101050020180102050", 23),
            ("201801010500201801020560", 22),
            ("2018010105002018010205001", 24),
        ],
    ),
    (
        "802",
        &[
            ("+1", 0),
            ("01", 0),
            ("-0", 0),
            ("1.5", 1),
            ("2147483648", 0),
            ("-2147483649", 0),
            ("", 0),
            ("-", 1),
        ],
    ),
    (
        "803",
        &[("+1", 0), ("00", 0), ("-01", 1), (" 1", 0), ("1 ", 1)],
    ),
    (
        "804",
        &[
            ("+0", 0),
            ("1e3", 1),
            ("-0", 0),
            ("99999999999999999999999", 0),
            ("ü", 0),
        ],
    ),
    (
        "805",
        &[("+24", 0), ("024", 0), ("2.5", 1), ("", 0), ("x", 0)],
    ),
];

#[test]
fn every_code_has_five_valid_and_five_invalid_cases() {
    for code in CODES {
        let valid = VALID.iter().find(|(c, _)| *c == code).unwrap().1;
        let invalid = INVALID.iter().find(|(c, _)| *c == code).unwrap().1;
        assert!(valid.len() >= 5, "{code}: {} valid", valid.len());
        assert!(invalid.len() >= 5, "{code}: {} invalid", invalid.len());
    }
}

#[test]
fn valid_values_decode_and_format_back() {
    for (code, values) in VALID {
        for value in *values {
            let v = DtmValue::parse(value, code).unwrap_or_else(|e| panic!("{value}/{code}: {e}"));
            assert_eq!(v.to_string(), *value, "{code}");
            assert_eq!(v.code(), *code);
            assert_eq!(DtmValue::parse(&v.to_string(), code), Ok(v));
        }
    }
}

#[test]
fn invalid_values_are_typed_errors_with_position() {
    for (code, cases) in INVALID {
        let format = DtmFormat::from_code(code).unwrap();
        for (value, position) in *cases {
            match DtmValue::parse(value, code) {
                Err(DtmError::InvalidValue {
                    format: f,
                    position: p,
                    ..
                }) => {
                    assert_eq!(f, format, "{value}/{code}");
                    assert_eq!(p, *position, "{value}/{code}");
                }
                other => panic!("{value}/{code}: {other:?}"),
            }
        }
    }
}

#[test]
fn unsupported_codes_are_refused_by_name() {
    for code in ["999", "101", "201", "105", "502", "616", "", "1020"] {
        assert_eq!(
            DtmValue::parse("20261001", code),
            Err(DtmError::UnsupportedFormat { code: code.into() })
        );
    }
}

#[test]
fn spec_scenarios() {
    let v = DtmValue::parse("202610011200+00", "303").unwrap();
    assert_eq!(
        (v.year(), v.month(), v.day(), v.hour(), v.minute()),
        (Some(2026), Some(10), Some(1), Some(12), Some(0))
    );
    assert_eq!(v.offset_hours(), Some(0));
    assert_eq!(v.to_string(), "202610011200+00");

    let p = DtmValue::parse("201801010500201801020500", "719").unwrap();
    assert_eq!(p.start().unwrap().to_string(), "201801010500");
    assert_eq!(p.end().unwrap().to_string(), "201801020500");
    assert_eq!(p.start().unwrap().code(), "203");
    assert_eq!(p.to_string(), "201801010500201801020500");

    let err = DtmValue::parse("20180132", "102").unwrap_err();
    assert_eq!(
        err.to_string(),
        "value does not match format 102 (CCYYMMDD): day out of range at position 6"
    );
    let hyphen = DtmValue::parse("201801010500-201801020500", "719").unwrap_err();
    assert!(hyphen.to_string().contains("hyphen"), "{hyphen}");

    assert_eq!(
        DtmValue::parse("201801012400", "203").unwrap().to_string(),
        "201801012400"
    );
}

#[test]
fn as_written_details_are_kept() {
    let plus = DtmValue::parse("202610011200+00", "303").unwrap();
    let minus = DtmValue::parse("202610011200-00", "303").unwrap();
    assert_ne!(plus, minus);
    assert_eq!(minus.to_string(), "202610011200-00");
    assert_eq!(minus.offset_hours(), Some(0));
    assert_eq!(
        DtmValue::parse("202610010000-05", "303")
            .unwrap()
            .offset_hours(),
        Some(-5)
    );
    // `2400` is not `==` to the next `0000`; `compare` says Equal.
    assert_ne!(
        DtmValue::parse("201801012400", "203").unwrap(),
        DtmValue::parse("201801020000", "203").unwrap()
    );
}

#[test]
fn accessors_follow_the_layout() {
    let p = |v, c| DtmValue::parse(v, c).unwrap();
    let mmdd = p("0229", "106");
    assert_eq!(
        (mmdd.year(), mmdd.month(), mmdd.day()),
        (None, Some(2), Some(29))
    );
    let hhmm = p("2400", "401");
    assert_eq!(
        (hhmm.day(), hhmm.hour(), hhmm.minute()),
        (None, Some(24), Some(0))
    );
    let ym = p("202610", "610");
    assert_eq!(
        (ym.year(), ym.month(), ym.day()),
        (Some(2026), Some(10), None)
    );
    let y = p("2026", "602");
    assert_eq!((y.year(), y.month()), (Some(2026), None));
    let s = p("20261001120045", "204");
    assert_eq!((s.second(), s.offset_hours()), (Some(45), None));
    assert_eq!(p("20261001", "102").hour(), None);
    assert_eq!(p("0", "805").count(), Some(0));
    assert_eq!(p("20261001", "102").count(), None);
    assert_eq!(p("20261001", "102").start(), None);
    assert_eq!(p("08001700", "501").end().unwrap().to_string(), "1700");
    assert_eq!(p("2026100120261031", "718").end().unwrap().code(), "102");
}

#[test]
fn periods_report_direction() {
    let p = |v, c| DtmValue::parse(v, c).unwrap();
    assert!(p("201801010500201801020500", "719").is_forward());
    assert!(!p("201801020500201801010500", "719").is_forward());
    assert!(!p("201801010500201801010500", "719").is_forward());
    assert!(p("2026100120261001", "718").is_forward()); // one whole day
    assert!(!p("2026100220261001", "718").is_forward());
    assert!(p("08001700", "501").is_forward());
    assert!(!p("17000800", "501").is_forward());
    assert!(p("20261001", "102").is_forward());
    // Inverted periods are kept as written, never swapped.
    assert_eq!(p("2026100220261001", "718").to_string(), "2026100220261001");
}

#[test]
fn non_ascii_at_every_width_is_refused_without_panic() {
    for code in CODES {
        for n in 0..30 {
            for filler in ["ü", "€", "1ü", "𝟙"] {
                let value = filler.repeat(n);
                let r = DtmValue::parse(&value, code);
                assert!(r.is_err(), "{value:?}/{code}");
            }
        }
    }
}

// ── round-trip properties ─────────────────────────────────────────────────────

/// SplitMix64: deterministic, no dev-dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo + 1)
    }
}

fn days_in_month(y: u64, m: u64) -> u64 {
    match m {
        2 if (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn date(r: &mut Rng) -> String {
    let y = r.range(1, 9999);
    let m = r.range(1, 12);
    let d = r.range(1, days_in_month(y, m));
    format!("{y:04}{m:02}{d:02}")
}

fn hhmm(r: &mut Rng, seconds: bool) -> String {
    // One in 20 is end-of-day `24`.
    if r.range(0, 19) == 0 {
        return if seconds { "240000" } else { "2400" }.into();
    }
    let s = format!("{:02}{:02}", r.range(0, 23), r.range(0, 59));
    if seconds {
        format!("{s}{:02}", r.range(0, 59))
    } else {
        s
    }
}

fn zone(r: &mut Rng) -> String {
    let sign = if r.range(0, 1) == 0 { '+' } else { '-' };
    format!("{sign}{:02}", r.range(0, 14))
}

fn generate(code: &str, r: &mut Rng) -> String {
    match code {
        "102" => date(r),
        "104" => {
            let (m, n) = (r.range(1, 12), r.range(1, 12));
            format!("{m:02}{:02}{n:02}{:02}", r.range(1, 5), r.range(1, 5))
        }
        "106" => {
            let m = r.range(1, 12);
            format!("{m:02}{:02}", r.range(1, days_in_month(2000, m)))
        }
        "203" => format!("{}{}", date(r), hhmm(r, false)),
        "204" => format!("{}{}", date(r), hhmm(r, true)),
        "303" => format!("{}{}{}", date(r), hhmm(r, false), zone(r)),
        "304" => format!("{}{}{}", date(r), hhmm(r, true), zone(r)),
        "401" => hhmm(r, false),
        "501" => format!("{}{}", hhmm(r, false), hhmm(r, false)),
        "602" => format!("{:04}", r.range(1, 9999)),
        "610" => format!("{:04}{:02}", r.range(1, 9999), r.range(1, 12)),
        "718" => format!("{}{}", date(r), date(r)),
        "719" => format!("{}{}{}{}", date(r), hhmm(r, false), date(r), hhmm(r, false)),
        _ => (r.next() as u32 as i32).to_string(),
    }
}

#[test]
fn round_trip_every_code() {
    let mut r = Rng(0x2379);
    for code in CODES {
        for _ in 0..10_000 {
            let s = generate(code, &mut r);
            let v = DtmValue::parse(&s, code).unwrap_or_else(|e| panic!("{s}/{code}: {e}"));
            assert_eq!(v.to_string(), s, "format∘decode");
            assert_eq!(
                DtmValue::parse(&v.to_string(), code),
                Ok(v),
                "decode∘format"
            );
        }
    }
}

// ── builders ──────────────────────────────────────────────────────────────────

fn stamp_303() -> DtmValue {
    DtmValue::parse("202610011200", "203")
        .unwrap()
        .with_offset(0)
        .unwrap()
}

#[test]
fn builder_components_write_the_wire_form() {
    let dtm = Dtm::new("137", stamp_303());
    assert_eq!(dtm.components(), ["137", "202610011200+00", "303"]);
    let mut w = Writer::new(Vec::new());
    w.write_composites("DTM", &[dtm.components()]).unwrap();
    assert_eq!(w.finish().unwrap(), b"DTM+137:202610011200?+00:303'");
}

#[test]
fn builder_to_segment_writes_the_same_bytes() {
    let dtm = Dtm::new("137", stamp_303());
    let mut w = Writer::new(Vec::new());
    w.write_segment(&dtm.to_segment()).unwrap();
    assert_eq!(w.finish().unwrap(), b"DTM+137:202610011200?+00:303'");

    let id = MessageIdentifier::new("1", "UTILMD", "D", "11A", "UN");
    let mut by_components = MessageBuilder::new(id).unwrap();
    by_components
        .write_composites("DTM", &[dtm.components()])
        .unwrap();
    let mut by_segment = MessageBuilder::new(id).unwrap();
    by_segment.write_segment(&dtm.to_segment()).unwrap();
    assert_eq!(
        by_components.finish().unwrap(),
        by_segment.finish().unwrap()
    );
}

#[test]
fn builder_from_segment_round_trips() {
    let segments: Vec<_> = edifact::from_bytes(b"DTM+137:202610011200?+00:303'")
        .collect::<Result<_, _>>()
        .unwrap();
    let dtm = Dtm::from_segment(&segments[0]).unwrap();
    assert_eq!(dtm, Dtm::new("137", stamp_303()));
    assert_eq!(
        edifact::segments_to_bytes(&[dtm.to_segment()]).unwrap(),
        b"DTM+137:202610011200?+00:303'"
    );
}

#[test]
fn builder_from_segment_errors() {
    let parse = |bytes: &[u8]| -> Vec<edifact::OwnedSegment> {
        edifact::from_bytes(bytes)
            .map(|s| s.map(edifact::Segment::into_owned))
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let bgm = parse(b"BGM+220'");
    assert_eq!(
        Dtm::from_segment(&bgm[0]),
        Err(DtmError::NotDtm { tag: "BGM".into() })
    );
    for (bytes, component) in [
        (&b"DTM+137'"[..], 1),
        (b"DTM+137:20260701'", 2),
        (b"DTM+137::102'", 1),
        (b"DTM+:20260701:102'", 0),
    ] {
        let seg = parse(bytes);
        let err = Dtm::from_segment(&seg[0]).unwrap_err();
        assert_eq!(err, DtmError::MissingComponent { component });
        assert_eq!(err.component(), Some(component));
    }
    let bad = parse(b"DTM+137:20260701:303'");
    let err = Dtm::from_segment(&bad[0]).unwrap_err();
    assert!(matches!(err, DtmError::InvalidValue { .. }));
    assert_eq!(err.component(), Some(1));
    let unsupported = parse(b"DTM+137:x:999'");
    assert_eq!(
        Dtm::from_segment(&unsupported[0]).unwrap_err().component(),
        Some(2)
    );
}

#[test]
fn builder_with_offset() {
    let p = |v, c| DtmValue::parse(v, c).unwrap();
    assert_eq!(stamp_303().to_string(), "202610011200+00");
    let s = p("20261001120045", "204").with_offset(-5).unwrap();
    assert_eq!(
        (s.code(), s.to_string()),
        ("304", "20261001120045-05".into())
    );
    assert_eq!(
        p("20261001", "102").with_offset(0),
        Err(DtmError::NotConvertible {
            format: DtmFormat::Ccyymmdd,
            target: "303/304"
        })
    );
    assert_eq!(
        p("202610011200", "203").with_offset(15),
        Err(DtmError::InvalidOffset { seconds: 54_000 })
    );
    assert_eq!(
        p("202610011200", "203").with_offset(-128),
        Err(DtmError::InvalidOffset { seconds: -460_800 })
    );
    assert!(p("202610011200+00", "303").with_offset(1).is_err());
}

#[test]
fn code_104_decodes_month_week_periods() {
    // An inverted pair (`05040503`) decodes too; it compares `Indeterminate`.
    for value in ["05040504", "06030604", "08010804", "01011205", "05040503"] {
        let v = DtmValue::parse(value, "104").unwrap_or_else(|e| panic!("{value}: {e}"));
        assert_eq!(v.format(), DtmFormat::MmwwMmww);
        assert_eq!(v.to_string(), value);
    }
    assert_eq!(DtmFormat::MmwwMmww.code(), "104");
    assert_eq!(DtmFormat::MmwwMmww.layout(), "MMWWMMWW");
}

#[test]
fn code_104_refuses_malformed_values() {
    // Length, month 13, week 00, week 06, a letter, a hyphen.
    let cases = INVALID.iter().find(|(c, _)| *c == "104").unwrap().1;
    for (value, position) in cases {
        match DtmValue::parse(value, "104") {
            Err(DtmError::InvalidValue {
                format: DtmFormat::MmwwMmww,
                position: at,
                ..
            }) => assert_eq!(at, *position, "{value}"),
            other => panic!("{value}: {other:?}"),
        }
    }
}
