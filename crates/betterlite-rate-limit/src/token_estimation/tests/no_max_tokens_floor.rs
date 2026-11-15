#![cfg(test)]

use rstest::rstest;

use super::output_cap::{no_max_tokens_output_floor, output_floor};

#[rstest]
#[case::no_limit(None, 1024)]
#[case::at_the_baseline(Some(4096), 1024)]
#[case::above_the_baseline(Some(8192), 1024)]
#[case::just_below_the_baseline(Some(4095), 1023)]
#[case::small_limit(Some(100), 25)]
#[case::below_the_fraction(Some(3), 1)]
#[case::zero_limit(Some(0), 1)]
#[case::negative_limit(Some(-8), 1)]
fn the_floor_follows_the_smallest_configured_limit(
    #[case] limit: Option<i64>,
    #[case] expected: i64,
) {
    assert_eq!(no_max_tokens_output_floor(limit), expected);
}

#[test]
fn a_responses_request_never_floors_below_sixteen_tokens() {
    assert_eq!(output_floor(Some(8), Some("responses")), 16);
    assert_eq!(output_floor(None, Some("aresponses")), 1024);
}

#[test]
fn other_call_types_keep_the_plain_floor() {
    assert_eq!(output_floor(Some(8), Some("completion")), 2);
    assert_eq!(output_floor(None, None), 1024);
}
