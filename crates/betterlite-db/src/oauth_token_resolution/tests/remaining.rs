use rstest::rstest;

use crate::oauth_token_resolution::resolve::remaining_token_seconds;

use super::support::now;

#[rstest]
#[case::absent(None, None)]
#[case::empty(Some(""), None)]
#[case::unparsable(Some("yesterday"), None)]
#[case::past(Some("2024-01-02T03:04:04+00:00"), None)]
#[case::at_the_deadline(Some("2024-01-02T03:04:05+00:00"), None)]
#[case::one_second(Some("2024-01-02T03:04:06+00:00"), Some(1))]
#[case::utc_offset(Some("2024-01-02T06:04:05+02:00"), Some(3600))]
#[case::zulu(Some("2024-01-02T04:04:05Z"), Some(3600))]
#[case::naive_is_utc(Some("2024-01-02T04:04:05"), Some(3600))]
#[case::subsecond(Some("2024-01-02T04:04:05.900000+00:00"), Some(3600))]
fn remaining_seconds_counts_down_to_the_expiry(
    #[case] expires: Option<&str>,
    #[case] expected: Option<i64>,
) {
    assert_eq!(remaining_token_seconds(expires, now()), expected);
}
