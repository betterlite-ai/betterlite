use std::time::Duration;

use jiff::Timestamp;
use reqwest::StatusCode;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use rstest::rstest;

use crate::azure_content_safety::legacy_retry::{BACKOFF_FACTOR, is_retry, retry_delay};

// 2026-01-02T03:04:05Z
const NOW: i64 = 1_767_323_045;

fn headers(entries: &[(&'static str, &'static str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for &(name, value) in entries {
        map.insert(
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
    }
    map
}

#[rstest]
#[case::first_retry_waits_nothing(&[], 1, 0)]
#[case::second_retry_backs_off(&[], 2, 1_600)]
#[case::third_retry_doubles(&[], 3, 3_200)]
#[case::just_under_the_cap(&[], 8, 102_400)]
#[case::capped_at_two_minutes(&[], 9, 120_000)]
#[case::retry_after_seconds(&[("retry-after", "2")], 1, 2_000)]
#[case::fractional_retry_after_backs_off(&[("retry-after", "0.5")], 2, 1_600)]
#[case::retry_after_zero_backs_off(&[("retry-after", "0")], 2, 1_600)]
#[case::retry_after_zero_hides_the_ms_header(&[("retry-after", "0"), ("retry-after-ms", "100")], 1, 0)]
#[case::negative_retry_after_backs_off(&[("retry-after", "-3")], 2, 1_600)]
#[case::unreadable_retry_after_backs_off(&[("retry-after", "soon")], 2, 1_600)]
#[case::infinite_retry_after_backs_off(&[("retry-after", "inf")], 2, 1_600)]
#[case::retry_after_wins_over_ms(&[("retry-after", "2"), ("retry-after-ms", "100")], 1, 2_000)]
#[case::empty_retry_after_reads_the_ms_header(&[("retry-after", ""), ("retry-after-ms", "100")], 1, 100)]
#[case::retry_after_ms(&[("retry-after-ms", "1500")], 1, 1_500)]
#[case::x_ms_retry_after_ms(&[("x-ms-retry-after-ms", "250")], 1, 250)]
#[case::retry_after_date(&[("retry-after", "Fri, 02 Jan 2026 03:04:15 GMT")], 1, 10_000)]
#[case::past_retry_after_date_backs_off(&[("retry-after", "Fri, 02 Jan 2026 03:03:05 GMT")], 2, 1_600)]
fn waits_like_the_sdk_before_a_retry(
    #[case] entries: &[(&'static str, &'static str)],
    #[case] retries: u32,
    #[case] millis: u64,
) {
    let now = Timestamp::from_second(NOW).unwrap();
    assert_eq!(
        retry_delay(&headers(entries), retries, BACKOFF_FACTOR, now),
        Duration::from_millis(millis)
    );
}

#[rstest]
#[case::internal_server_error(500, &[], true)]
#[case::service_unavailable(503, &[], true)]
#[case::gateway_timeout(504, &[], true)]
#[case::bad_request(400, &[], false)]
#[case::request_timeout(408, &[], false)]
#[case::too_many_requests(429, &[], false)]
#[case::bad_gateway(502, &[], false)]
#[case::http_version_not_supported(505, &[], false)]
#[case::ok(200, &[], false)]
#[case::error_status_with_retry_after(400, &[("retry-after", "1")], true)]
#[case::status_below_400_with_retry_after(399, &[("retry-after", "1")], false)]
#[case::redirect_with_retry_after(302, &[("retry-after", "1")], false)]
#[case::empty_retry_after(404, &[("retry-after", "")], false)]
fn retries_a_post_like_the_sdk(
    #[case] status: u16,
    #[case] entries: &[(&'static str, &'static str)],
    #[case] expected: bool,
) {
    assert_eq!(
        is_retry(StatusCode::from_u16(status).unwrap(), &headers(entries)),
        expected
    );
}
