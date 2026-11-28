#![cfg(test)]

use rstest::rstest;

use super::{ParsedDate, parsedate_tz, python_int, zone_offset};

#[rstest]
#[case::underscore_between_digits("1_0", Some(10))]
#[case::plus_sign("+7", Some(7))]
#[case::minus_sign("-5", Some(-5))]
#[case::leading_zeroes("007", Some(7))]
#[case::twenty_five_digits("9999999999999999999999999", Some(i64::MAX))]
#[case::double_underscore("1__0", None)]
#[case::leading_underscore("_10", None)]
#[case::trailing_underscore("10_", None)]
#[case::bare_plus("+", None)]
#[case::empty("", None)]
#[case::hex("0x10", None)]
#[case::fraction("3.5", None)]
#[case::negative_at_the_signed_limit("-9223372036854775807", Some(-0x7FFF_FFFF_FFFF_FFFF_i64))]
fn python_int_reads_python_integers(#[case] text: &str, #[case] expected: Option<i64>) {
    assert_eq!(python_int(text), expected);
}

#[rstest]
#[case::eastern("EST", Some(-18_000))]
#[case::numeric_zone("+0530", Some(19_800))]
#[case::negative_numeric_zone("-0530", Some(-19_800))]
#[case::utc_name("GMT", Some(0))]
#[case::zero_offset("+0000", Some(0))]
#[case::negative_zero("-0000", None)]
#[case::unknown_name("XYZ", None)]
#[case::letter_in_the_number("+12a4", None)]
#[case::missing("", None)]
#[case::twenty_digits("+99999999999999999999", Some(i64::MAX))]
#[case::atlantic_standard("AST", Some(-14_400))]
#[case::atlantic_daylight("ADT", Some(-10_800))]
#[case::eastern_daylight("EDT", Some(-14_400))]
#[case::central_standard("CST", Some(-21_600))]
#[case::central_daylight("CDT", Some(-18_000))]
#[case::mountain_standard("MST", Some(-25_200))]
#[case::mountain_daylight("MDT", Some(-21_600))]
#[case::pacific_standard("PST", Some(-28_800))]
#[case::pacific_daylight("PDT", Some(-25_200))]
fn zone_offset_follows_parsedate_tz(#[case] zone: &str, #[case] expected: Option<i64>) {
    assert_eq!(zone_offset(zone), expected);
}

#[rstest]
#[case::rfc_2822(
    "Wed, 21 Oct 2015 07:28:00 GMT",
    ParsedDate {
        year: 2015,
        month: 10,
        day: 21,
        hour: 7,
        minute: 28,
        second: 0,
        zone_offset: Some(0),
    }
)]
#[case::rfc_850(
    "Sunday, 06-Nov-94 08:49:37 GMT",
    ParsedDate {
        year: 1994,
        month: 11,
        day: 6,
        hour: 8,
        minute: 49,
        second: 37,
        zone_offset: Some(0),
    }
)]
#[case::asctime(
    "Sun Nov  6 08:49:37 1994",
    ParsedDate {
        year: 1994,
        month: 11,
        day: 6,
        hour: 8,
        minute: 49,
        second: 37,
        zone_offset: None,
    }
)]
#[case::month_first(
    "Oct 21 2015 07:28:00 GMT",
    ParsedDate {
        year: 2015,
        month: 10,
        day: 21,
        hour: 7,
        minute: 28,
        second: 0,
        zone_offset: Some(0),
    }
)]
#[case::glued_zone(
    "21 Oct 2015 07:28:00+0200",
    ParsedDate {
        year: 2015,
        month: 10,
        day: 21,
        hour: 7,
        minute: 28,
        second: 0,
        zone_offset: Some(7_200),
    }
)]
#[case::signed_hour(
    "21 Oct 2015 +07:28",
    ParsedDate {
        year: 2015,
        month: 10,
        day: 21,
        hour: 7,
        minute: 28,
        second: 0,
        zone_offset: None,
    }
)]
#[case::no_seconds(
    "Wed, 21 Oct 2015 07:28 GMT",
    ParsedDate {
        year: 2015,
        month: 10,
        day: 21,
        hour: 7,
        minute: 28,
        second: 0,
        zone_offset: Some(0),
    }
)]
#[case::colon_first_year(
    "21 Oct :5 07:28 2015",
    ParsedDate {
        year: 2015,
        month: 10,
        day: 21,
        hour: 7,
        minute: 28,
        second: 0,
        zone_offset: None,
    }
)]
#[case::two_digit_year_68(
    "21 Oct 68 07:28:00 GMT",
    ParsedDate {
        year: 2068,
        month: 10,
        day: 21,
        hour: 7,
        minute: 28,
        second: 0,
        zone_offset: Some(0),
    }
)]
#[case::two_digit_year_69(
    "21 Oct 69 07:28:00 GMT",
    ParsedDate {
        year: 1969,
        month: 10,
        day: 21,
        hour: 7,
        minute: 28,
        second: 0,
        zone_offset: Some(0),
    }
)]
#[case::year_100(
    "21 Oct 100 07:28:00 GMT",
    ParsedDate {
        year: 100,
        month: 10,
        day: 21,
        hour: 7,
        minute: 28,
        second: 0,
        zone_offset: Some(0),
    }
)]
#[case::comma_after_the_day(
    "Wed, 21, Oct 2015 07:28:00 GMT",
    ParsedDate {
        year: 2015,
        month: 10,
        day: 21,
        hour: 7,
        minute: 28,
        second: 0,
        zone_offset: Some(0),
    }
)]
fn parsedate_tz_reads_the_fields(#[case] text: &str, #[case] expected: ParsedDate) {
    assert_eq!(parsedate_tz(text), Some(expected));
}
