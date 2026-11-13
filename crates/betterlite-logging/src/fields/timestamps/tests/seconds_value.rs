use rstest::rstest;
use rust_decimal::Decimal;

use crate::fields::timestamps::{Moment, timestamp_seconds};

use super::{moment, seconds};

#[rstest]
#[case::datetime(moment("2024-01-01T00:00:00+00:00"), seconds("1704067200.0"))]
#[case::float(Moment::Float(seconds("1704067200.25")), seconds("1704067200.25"))]
#[case::integer(Moment::Integer(1_704_067_200), seconds("1704067200.0"))]
fn logging_fields_behavior_tests_behavior(#[case] value: Moment, #[case] expected: Decimal) {
    assert_eq!(timestamp_seconds(&value), expected);
}
