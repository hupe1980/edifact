//! Format-aware comparison: interval semantics, floating vs zoned, range edges.

use DtmRelation::{After, Before, Equal, Indeterminate, Overlapping};
use edifact::{DtmRelation, DtmValue};

fn p(value: &str, code: &str) -> DtmValue {
    DtmValue::parse(value, code).unwrap_or_else(|e| panic!("{value}/{code}: {e}"))
}

/// `(a, a_code, b, b_code, expected)`.
const TABLE: &[(&str, &str, &str, &str, DtmRelation)] = &[
    // String comparison gets both of these wrong.
    ("202610010000+02", "303", "202609302300+00", "303", Before),
    ("202610010000", "203", "20261001", "102", Overlapping),
    // Floating vs zoned within 28 h: depends on the zone.
    (
        "202610011200",
        "203",
        "202610011200+00",
        "303",
        Indeterminate,
    ),
    (
        "202610011200",
        "203",
        "202610011100+00",
        "303",
        Indeterminate,
    ),
    ("20261001", "102", "202610011200+00", "303", Indeterminate),
    // Floating vs zoned months apart: every offset agrees.
    ("20260115", "102", "202610011200+00", "303", Before),
    ("202612011200", "203", "202610011200+00", "303", After),
    ("2025", "602", "202610011200-05", "303", Before),
    // Periods and dates.
    (
        "201801010500201801020500",
        "719",
        "20180101",
        "102",
        Overlapping,
    ),
    ("201801010500201801020500", "719", "20180103", "102", Before),
    ("201801010500201801020500", "719", "20171231", "102", After),
    (
        "2026100120261031",
        "718",
        "202610311200",
        "203",
        Overlapping,
    ),
    ("2026100120261031", "718", "202611010000", "203", Before),
    ("2026100120261031", "718", "202610", "610", Equal),
    ("2026100120261001", "718", "20261001", "102", Equal),
    ("2026100120261031", "718", "2026100120261031", "718", Equal),
    (
        "2026100120261015",
        "718",
        "2026101020261031",
        "718",
        Overlapping,
    ),
    ("2026100120261015", "718", "2026101620261031", "718", Before),
    ("202610010000202610020000", "719", "20261001", "102", Equal),
    ("202610", "610", "2026", "602", Overlapping),
    ("202601", "610", "2026", "602", Overlapping),
    ("2026", "602", "2027", "602", Before),
    ("202609", "610", "20261001", "102", Before),
    // Same timeline, minute and second precision.
    ("201801012400", "203", "201801020000", "203", Equal),
    ("20180101240000", "204", "201801020000", "203", Overlapping),
    ("202610011200", "203", "202610011201", "203", Before),
    ("202610011200", "203", "20261001120000", "204", Overlapping),
    ("20261001120000", "204", "20261001120001", "204", Before),
    ("202610011200+00", "303", "202610011400+02", "303", Equal),
    ("202610011200+00", "303", "202610011200-00", "303", Equal),
    (
        "20261001120030+00",
        "304",
        "202610011200+00",
        "303",
        Overlapping,
    ),
    // Other timelines.
    ("0229", "106", "0301", "106", Before),
    ("0101", "106", "0101", "106", Equal),
    ("0101", "106", "20260101", "102", Indeterminate),
    ("0800", "401", "08001700", "501", Overlapping),
    ("1700", "401", "08001700", "501", After),
    ("2400", "401", "0000", "401", After),
    ("08001200", "501", "12001700", "501", Before),
    ("0800", "401", "202610010800", "203", Indeterminate),
    // Counts and inverted or empty periods.
    ("1", "805", "1", "805", Indeterminate),
    ("0", "804", "20261001", "102", Indeterminate),
    ("2026103120261001", "718", "20261015", "102", Indeterminate),
    (
        "201801020500201801010500",
        "719",
        "20180101",
        "102",
        Indeterminate,
    ),
    (
        "201801010500201801010500",
        "719",
        "20180101",
        "102",
        Indeterminate,
    ),
    ("17000800", "501", "1200", "401", Indeterminate),
];

#[test]
fn comparison_table() {
    assert!(TABLE.len() >= 30);
    for &(a, ac, b, bc, expected) in TABLE {
        let (a, b) = (p(a, ac), p(b, bc));
        assert_eq!(a.compare(&b), expected, "{a} vs {b}");
        assert_eq!(b.compare(&a), expected.reverse(), "{b} vs {a}");
    }
}

#[test]
fn explicit_offset_decides_floating_vs_zoned() {
    let floating = p("202610011200", "203");
    let zoned = p("202610011200+00", "303");
    assert_eq!(floating.compare(&zoned), Indeterminate);
    assert_eq!(floating.compare_assuming(&zoned, 0), Equal);
    assert_eq!(zoned.compare_assuming(&floating, 0), Equal);
    // 12:00 at +02 is 10:00 UTC.
    assert_eq!(floating.compare_assuming(&zoned, 2), Before);
    assert_eq!(floating.compare_assuming(&zoned, -2), After);
    // Both floating: the offset does not matter.
    let other = p("202610011300", "203");
    assert_eq!(floating.compare_assuming(&other, 5), Before);
}

/// The period-order check from the `compare` docs and the guide, verbatim.
fn period_is_ordered(start: &DtmValue, end: &DtmValue) -> bool {
    match start.compare(end) {
        DtmRelation::Before => true,
        // After, Equal, Overlapping, Indeterminate
        _ => false,
    }
}

#[test]
fn deny_by_default_rejects_every_undecided_pair() {
    let pairs = [
        (p("202610011200", "203"), p("202610011100+00", "303")),
        (p("201801020500201801010500", "719"), p("20180103", "102")),
        (p("1", "804"), p("20261001", "102")),
        (p("0101", "106"), p("20260101", "102")),
    ];
    for (start, end) in &pairs {
        assert_eq!(start.compare(end), Indeterminate, "{start} vs {end}");
        assert!(!period_is_ordered(start, end), "{start} vs {end}");
    }
    assert!(!period_is_ordered(
        &p("202610010000", "203"),
        &p("20261001", "102")
    ));
    assert!(!period_is_ordered(
        &p("202609302300+00", "303"),
        &p("202610010000+02", "303")
    ));
    assert!(period_is_ordered(
        &p("202610010000+02", "303"),
        &p("202609302300+00", "303")
    ));
}

#[test]
fn range_edge_relations() {
    assert_eq!(
        p("999912312400", "203").compare(&p("99991231", "102")),
        After
    );
    assert_eq!(
        p("9999120199991231", "718").compare(&p("999912", "610")),
        Equal
    );
    assert_eq!(
        p("9999123199991231", "718").compare(&p("999912", "610")),
        Overlapping
    );
    assert_eq!(p("9999", "602").compare(&p("999912", "610")), Overlapping);
    assert_eq!(
        p("000101010000", "203").compare(&p("000101010000+14", "303")),
        Indeterminate
    );
    assert_eq!(
        p("000101010000", "203").compare_assuming(&p("000101010000+14", "303"), 14),
        Equal
    );
    assert_eq!(
        p("999912312359+14", "303").compare(&p("00010101", "102")),
        After
    );
    assert_eq!(
        p("000101010000-14", "303").compare(&p("99991231", "102")),
        Before
    );
    let floating = p("202610011200", "203");
    let zoned = p("202610011200+00", "303");
    for offset in [i8::MIN, -15, 15, i8::MAX] {
        assert_eq!(floating.compare_assuming(&zoned, offset), Indeterminate);
    }
    for offset in -14..=14 {
        assert_ne!(floating.compare_assuming(&zoned, offset), Indeterminate);
    }
}

#[test]
fn range_edges_do_not_panic_at_any_offset() {
    let edges = [
        p("99991231", "102"),
        p("999912312400", "203"),
        p("99991231240000", "204"),
        p("999912312400+14", "303"),
        p("999912312400-14", "303"),
        p("00010101000000-14", "304"),
        p("00010101000000+14", "304"),
        p("9999", "602"),
        p("0001", "602"),
        p("999912", "610"),
        p("9999123199991231", "718"),
        p("000101010000999912312400", "719"),
        p("2400", "401"),
        p("00002400", "501"),
        p("1231", "106"),
        p("-2147483648", "805"),
    ];
    for a in &edges {
        assert!(a.is_forward() || a.format().is_period());
        for b in &edges {
            let r = a.compare(b);
            assert_eq!(b.compare(a), r.reverse());
            for o in i8::MIN..=i8::MAX {
                assert_eq!(b.compare_assuming(a, o), a.compare_assuming(b, o).reverse());
            }
        }
    }
}

#[test]
fn code_104_compares_only_with_104() {
    let cases = [
        ("06010602", "06030604", Before),
        ("06030604", "06010602", After),
        ("06010603", "06030604", Overlapping),
        ("05040504", "05040504", Equal),
        ("05040503", "05040504", Indeterminate),
        ("05040504", "05040503", Indeterminate),
    ];
    for (a, b, expected) in cases {
        assert_eq!(p(a, "104").compare(&p(b, "104")), expected, "{a} vs {b}");
        assert_eq!(
            p(a, "104").compare_assuming(&p(b, "104"), 0),
            expected,
            "{a} vs {b}"
        );
    }
    let week = p("05040504", "104");
    for (other, code) in [
        ("20260522", "102"),
        ("0522", "106"),
        ("202605221200+00", "303"),
        ("1", "802"),
    ] {
        let other = p(other, code);
        assert_eq!(week.compare(&other), Indeterminate, "104 vs {code}");
        assert_eq!(other.compare(&week), Indeterminate, "{code} vs 104");
        assert_eq!(week.compare_assuming(&other, 0), Indeterminate);
    }
}
