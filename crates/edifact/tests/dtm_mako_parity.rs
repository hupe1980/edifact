//! Inputs and results of the downstream DTM helpers this codec replaces,
//! replayed against the codec.

use edifact::{Dtm, DtmError, DtmFormat, DtmRelation, DtmValue};
use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset};

fn p(value: &str, code: &str) -> DtmValue {
    DtmValue::parse(value, code).unwrap_or_else(|e| panic!("{value}/{code}: {e}"))
}

fn utc(y: i32, m: Month, d: u8, h: u8, min: u8) -> OffsetDateTime {
    PrimitiveDateTime::new(
        Date::from_calendar_date(y, m, d).unwrap(),
        Time::from_hms(h, min, 0).unwrap(),
    )
    .assume_utc()
}

#[test]
fn dvgw_gas_day_period_and_counts() {
    let period = p("201801010500201801020500", "719");
    let at = |v: DtmValue| v.to_primitive_date_time().unwrap().assume_utc();
    assert_eq!(
        at(period.start().unwrap()),
        utc(2018, Month::January, 1, 5, 0)
    );
    assert_eq!(
        at(period.end().unwrap()),
        utc(2018, Month::January, 2, 5, 0)
    );
    assert!(period.is_forward());
    assert_eq!(period.to_string(), "201801010500201801020500");

    assert_eq!(
        at(p("201801011200", "203")),
        utc(2018, Month::January, 1, 12, 0)
    );
    assert_eq!(
        at(p("201801012400", "203")),
        utc(2018, Month::January, 2, 0, 0)
    );
    assert_eq!(p("0", "805").count(), Some(0));
    assert_eq!(p("1", "805").count(), Some(1));

    // A declared DTM+Z05 offset is applied, not assumed.
    let plus_one = UtcOffset::from_hms(1, 0, 0).unwrap();
    let local = p("202603010600", "203").to_primitive_date_time().unwrap();
    assert_eq!(
        local.assume_offset(plus_one).to_offset(UtcOffset::UTC),
        utc(2026, Month::March, 1, 5, 0)
    );

    // Rendering: an instant back to 203 / 719 halves.
    let start = utc(2026, Month::March, 1, 5, 0);
    let v = DtmValue::from_primitive(
        PrimitiveDateTime::new(start.date(), start.time()),
        DtmFormat::Ccyymmddhhmm,
    )
    .unwrap();
    assert_eq!(v.to_string(), "202603010500");
}

#[test]
fn dvgw_refusals() {
    for (value, code) in [
        ("2018-01-01", "102"),
        ("201801011200", "719"),
        ("20180132", "102"),
        ("", "203"),
        ("2018ü101", "102"),
    ] {
        assert!(DtmValue::parse(value, code).is_err(), "{value}/{code}");
    }
    let wide = "ü".repeat(12);
    assert!(DtmValue::parse(&wide, "203").is_err());
    assert!(DtmValue::parse(&wide, "719").is_err());
}

#[test]
fn processd_303_instants() {
    let at = |v: &str| p(v, "303").to_offset_date_time().unwrap();
    assert_eq!(at("202612312300+00"), utc(2026, Month::December, 31, 23, 0));
    // 00:00 at UTC+01 is 23:00 UTC the day before.
    assert_eq!(at("202701010000+01"), utc(2026, Month::December, 31, 23, 0));
    assert!(DtmValue::parse("20261001abcd+00", "303").is_err());
    assert!(DtmValue::parse("2026100100000000", "303").is_err());
    assert!(DtmValue::parse("202612312300+00", "102").is_err());
}

#[test]
fn makod_303_and_102() {
    // dtm303_to_rfc3339("202606010000+00") = 2026-06-01T00:00:00+00:00.
    let at = p("202606010000+00", "303").to_offset_date_time().unwrap();
    assert_eq!(
        at.date(),
        Date::from_calendar_date(2026, Month::June, 1).unwrap()
    );
    assert_eq!(at.time(), Time::MIDNIGHT);
    assert_eq!(at.offset(), UtcOffset::UTC);
    // A 102 or 203 value is refused as an instant, not read as UTC.
    assert!(p("20260601", "102").to_offset_date_time().is_err());
    assert!(p("202606010000", "203").to_offset_date_time().is_err());
    // parse_ccyymmdd / edifact_date_to_date.
    assert_eq!(
        p("20260601", "102").to_date().unwrap(),
        Date::from_calendar_date(2026, Month::June, 1).unwrap()
    );
    // The one intended difference: the lenient reader accepted a 303 value
    // without its zone as +00; the codec refuses it.
    assert!(matches!(
        DtmValue::parse("202606010000", "303"),
        Err(DtmError::InvalidValue { position: 12, .. })
    ));
}

#[test]
fn makod_dtm_instant_by_qualifier() {
    use edifact::SegmentSliceExt;
    let segments: Vec<_> =
        edifact::from_bytes(b"DTM+137:202610011200?+00:303'DTM+163:202610010000?+02:303'")
            .collect::<Result<_, _>>()
            .unwrap();
    let dtm = Dtm::from_segment(segments.qualified("DTM", "163").unwrap()).unwrap();
    assert_eq!(
        dtm.value.to_offset_date_time().unwrap(),
        utc(2026, Month::September, 30, 22, 0)
    );
}

#[test]
fn edi_energy_period_order_counterexamples() {
    // The string comparison reported both of these as inverted.
    assert_eq!(
        p("202610010000+02", "303").compare(&p("202609302300+00", "303")),
        DtmRelation::Before
    );
    assert_eq!(
        p("202610010000", "203").compare(&p("20261001", "102")),
        DtmRelation::Overlapping
    );
}

#[test]
fn edi_energy_builder_stamps() {
    // ccyymmddhhmm_utc: append +00 to a 203 stamp.
    assert_eq!(
        p("202610011200", "203").with_offset(0).unwrap().to_string(),
        "202610011200+00"
    );
    // now_303: a UTC instant with seconds, truncated to the minute.
    let now = OffsetDateTime::from_unix_timestamp(1_790_856_045).unwrap();
    assert_eq!(
        DtmValue::from_offset_date_time(now, DtmFormat::CcyymmddhhmmZzz)
            .unwrap()
            .to_string(),
        "202610011200+00"
    );
    // DauerEinheit codes.
    assert_eq!(DtmFormat::Months.code(), "802");
    assert_eq!(DtmFormat::Weeks.code(), "803");
    assert_eq!(DtmFormat::Days.code(), "804");
    // dtm_303_field: fields of a 303 stamp.
    let v = p("202610012200+00", "303");
    assert_eq!(
        (v.month(), v.day(), v.hour(), v.minute()),
        (Some(10), Some(1), Some(22), Some(0))
    );
}
