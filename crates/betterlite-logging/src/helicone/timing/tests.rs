#![cfg(test)]

use chrono::DateTime;
use rstest::rstest;

use super::{Moment, moment};

// Expected values come from running LiteLLM's int((t.timestamp() - int(t.timestamp())) * 1000) in CPython.
#[rstest]
#[case::whole_second(1_700_000_000_000_000, 1_700_000_000, 0)]
#[case::point_three_rounds_down(1_700_000_001_300_000, 1_700_000_001, 299)]
#[case::point_one_two_three_is_122(1_700_000_000_123_000, 1_700_000_000, 122)]
#[case::point_nine_nine_nine_nine_nine_nine(1_700_000_000_999_999, 1_700_000_000, 999)]
#[case::one_microsecond(1_700_000_000_000_001, 1_700_000_000, 0)]
#[case::half_second(1_700_000_000_500_000, 1_700_000_000, 500)]
#[case::point_one_is_99(1_700_000_000_100_000, 1_700_000_000, 99)]
#[case::point_seven(1_700_000_000_700_000, 1_700_000_000, 700)]
#[case::quarter_second(1_700_000_000_250_000, 1_700_000_000, 250)]
#[case::epoch(0, 0, 0)]
#[case::small_timestamp_keeps_point_one_two_three(123_000, 0, 123)]
#[case::one_999(1_999_000, 1, 999)]
#[case::ten_ms_is_9(1_700_000_000_010_000, 1_700_000_000, 9)]
#[case::twenty_ms_is_19(1_700_000_000_020_000, 1_700_000_000, 19)]
#[case::thirty_ms_is_29(1_700_000_000_030_000, 1_700_000_000, 29)]
#[case::point_six_nine(1_700_000_000_690_000, 1_700_000_000, 690)]
#[case::arbitrary_micros(1_700_000_000_456_789, 1_700_000_000, 456)]
#[case::arbitrary_micros_up(1_700_000_000_123_456, 1_700_000_000, 123)]
#[case::negative_one_and_a_half(-1_500_000, -1, -500)]
#[case::negative_point_one_two_three(-123_000, 0, -123)]
#[case::negative_far_from_epoch(-1_700_000_000_123_000, -1_700_000_000, -122)]
fn timing_matches_litellms_double_math(
    #[case] micros: i64,
    #[case] seconds: i64,
    #[case] milliseconds: i64,
) {
    let time = DateTime::from_timestamp_micros(micros).unwrap();
    assert_eq!(
        moment(time).unwrap(),
        Moment {
            seconds,
            milliseconds
        }
    );
}

#[test]
fn timing_serializes_as_camel_case_pairs() {
    let timing = super::Timing {
        start_time: Moment {
            seconds: 1,
            milliseconds: 2,
        },
        end_time: Moment {
            seconds: 3,
            milliseconds: 4,
        },
    };
    assert_eq!(
        serde_json::to_string(&timing).unwrap(),
        r#"{"startTime":{"seconds":1,"milliseconds":2},"endTime":{"seconds":3,"milliseconds":4}}"#
    );
}
