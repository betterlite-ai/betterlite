#![cfg(test)]

use chrono::DateTime;
use rstest::rstest;
use rust_decimal::Decimal;
use std::str::FromStr as _;

use super::{Moment, TimestampError, Timestamps, cleanup_timestamps};

mod elapsed;
mod seconds_value;

fn moment(value: &str) -> Moment {
    Moment::DateTime(DateTime::parse_from_rfc3339(value).unwrap())
}

fn seconds(value: &str) -> Decimal {
    Decimal::from_str(value).unwrap()
}

#[rstest]
#[case::datetimes(
    Some(moment("2024-01-01T00:00:00+00:00")),
    Some(moment("2024-01-01T00:00:01+00:00")),
    Some(moment("2024-01-01T00:00:00.500+00:00")),
    Ok(Timestamps {
        start_time: seconds("1704067200.0"),
        end_time: seconds("1704067201.0"),
        completion_start_time: seconds("1704067200.5"),
    })
)]
#[case::floats(
    Some(Moment::Float(seconds("1704067200.5"))),
    Some(Moment::Float(seconds("1704067201.25"))),
    Some(Moment::Float(seconds("1704067200.75"))),
    Ok(Timestamps {
        start_time: seconds("1704067200.5"),
        end_time: seconds("1704067201.25"),
        completion_start_time: seconds("1704067200.75"),
    })
)]
#[case::missing_completion_falls_back_to_end(
    Some(Moment::Float(seconds("1704067200.0"))),
    Some(Moment::Float(seconds("1704067201.0"))),
    None,
    Ok(Timestamps {
        start_time: seconds("1704067200.0"),
        end_time: seconds("1704067201.0"),
        completion_start_time: seconds("1704067201.0"),
    })
)]
#[case::integer_completion_falls_back_to_end(
    Some(Moment::Float(seconds("1704067200.0"))),
    Some(Moment::Float(seconds("1704067201.0"))),
    Some(Moment::Integer(1_704_067_200)),
    Ok(Timestamps {
        start_time: seconds("1704067200.0"),
        end_time: seconds("1704067201.0"),
        completion_start_time: seconds("1704067201.0"),
    })
)]
#[case::missing_start_is_an_error(
    None,
    Some(Moment::Float(seconds("1704067201.0"))),
    None,
    Err(TimestampError::StartTimeRequired)
)]
#[case::missing_end_is_an_error(
    Some(Moment::Float(seconds("1704067200.0"))),
    None,
    None,
    Err(TimestampError::EndTimeRequired)
)]
#[case::integer_start_is_an_error(
    Some(Moment::Integer(1_704_067_200)),
    Some(Moment::Float(seconds("1704067201.0"))),
    None,
    Err(TimestampError::StartTimeRequired)
)]
#[case::integer_end_is_an_error(
    Some(Moment::Float(seconds("1704067200.0"))),
    Some(Moment::Integer(1_704_067_201)),
    None,
    Err(TimestampError::EndTimeRequired)
)]
fn logging_fields_behavior_tests_behavior(
    #[case] start: Option<Moment>,
    #[case] end: Option<Moment>,
    #[case] completion_start: Option<Moment>,
    #[case] expected: Result<Timestamps, TimestampError>,
) {
    let actual = cleanup_timestamps(start.as_ref(), end.as_ref(), completion_start.as_ref());
    assert_eq!(actual, expected);
}
