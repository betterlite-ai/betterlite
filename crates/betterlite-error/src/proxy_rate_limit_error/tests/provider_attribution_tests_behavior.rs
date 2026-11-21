#![cfg(test)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_redact::structured::Payload;
use rstest::rstest;

use crate::proxy_rate_limit_error::{
    ProxyRateLimitProjectionError, RateLimitResetWindow, V3RateLimitExceeded,
    map_v3_rate_limit_type, v3_rate_limit_error,
};
use crate::rate_limit::{
    ProxyRateLimitError, ProxyRateLimitErrorInit, RateLimitCategory, RateLimitDimension,
};

fn payload(text: &str) -> Payload {
    serde_json::from_str(text).expect("the fixture is JSON")
}

fn new_error(detail: Payload) -> ProxyRateLimitError {
    ProxyRateLimitError::new(ProxyRateLimitErrorInit {
        detail,
        headers: None,
        category: None,
        rate_limit_type: None,
        model: None,
        llm_provider: None,
    })
    .expect("no headers means nothing to reject")
}

fn moment(seconds: u64) -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds))
        .expect("the moment fits a SystemTime")
}

fn exceeded() -> V3RateLimitExceeded<'static> {
    V3RateLimitExceeded {
        descriptor_key: "model_per_project_itpm",
        descriptor_value: "proj:gpt-4o",
        rate_limit_type: "tokens",
        current_limit: 100,
        limit_remaining: -3,
    }
}

fn window() -> RateLimitResetWindow {
    RateLimitResetWindow {
        now: moment(1_718_452_800),
        window_size: 60,
    }
}

fn projection(
    exceeded: &V3RateLimitExceeded<'_>,
    window: RateLimitResetWindow,
) -> ProxyRateLimitError {
    v3_rate_limit_error(
        exceeded,
        window,
        String::from("gpt-4o-mini"),
        String::from("openai"),
    )
    .expect("the window is representable and the headers are valid")
}

#[rstest]
#[case::tokens(Some("tokens"), Some(RateLimitDimension::Tokens))]
#[case::max_parallel(
    Some("max_parallel_requests"),
    Some(RateLimitDimension::ConcurrentRequests)
)]
#[case::requests(Some("requests"), Some(RateLimitDimension::Requests))]
#[case::empty(Some(""), None)]
#[case::absent(None, None)]
#[case::misspelled(Some("max_parallel_request"), None)]
#[case::capitalized(Some("Tokens"), None)]
#[case::budget(Some("budget"), None)]
#[case::max_iterations(Some("max_iterations"), None)]
fn provider_attribution_tests_behavior_maps_v3_rate_limit_types(
    #[case] v3_value: Option<&str>,
    #[case] expected: Option<RateLimitDimension>,
) {
    assert_eq!(map_v3_rate_limit_type(v3_value), expected);
}

#[rstest]
#[case::null("null", "")]
#[case::text(r#""slow down""#, "slow down")]
#[case::error_text(r#"{"error": "flat"}"#, "flat")]
#[case::message_text(r#"{"message": "slow down"}"#, "slow down")]
#[case::nested_error(r#"{"error": {"message": "inner"}}"#, "inner")]
#[case::nested_message(r#"{"message": {"message": "deep"}}"#, "deep")]
#[case::error_before_message(r#"{"error": "first", "message": "second"}"#, "first")]
#[case::number("7", "7")]
#[case::boolean("true", "true")]
#[case::array(r#"["a"]"#, r#"["a"]"#)]
fn provider_attribution_tests_behavior_coerces_the_detail_into_the_message(
    #[case] detail: &str,
    #[case] expected: &str,
) {
    assert_eq!(new_error(payload(detail)).message, expected);
}

#[test]
fn provider_attribution_tests_behavior_json_encodes_a_mapping_without_a_message() {
    let error = new_error(payload(r#"{"foo": "bar"}"#));
    assert_eq!(error.message, r#"{"foo":"bar"}"#);
    assert_eq!(error.detail, payload(r#"{"foo": "bar"}"#));
}

#[rstest]
#[case::one_minute(1_718_452_800, 60, "2024-06-15 12:01:00 UTC")]
#[case::epoch_start(0, 0, "1970-01-01 00:00:00 UTC")]
#[case::leap_day(1_709_164_800, 0, "2024-02-29 00:00:00 UTC")]
#[case::negative_window(1_718_452_860, -60, "2024-06-15 12:00:00 UTC")]
fn provider_attribution_tests_behavior_formats_the_reset_timestamp(
    #[case] now_seconds: u64,
    #[case] window_size: i64,
    #[case] expected: &str,
) {
    let window = RateLimitResetWindow {
        now: moment(now_seconds),
        window_size,
    };
    let error = projection(&exceeded(), window);
    assert_eq!(
        error.headers.expect("the projection sets its headers")["reset_at"],
        expected
    );
}

#[test]
fn provider_attribution_tests_behavior_drops_the_fraction_of_a_second() {
    let now = moment(1_718_452_800)
        .checked_add(Duration::from_millis(900))
        .expect("the moment fits a SystemTime");
    let error = projection(
        &exceeded(),
        RateLimitResetWindow {
            now,
            window_size: 60,
        },
    );
    assert_eq!(
        error.headers.expect("the projection sets its headers")["reset_at"],
        "2024-06-15 12:01:00 UTC"
    );
}

#[test]
fn provider_attribution_tests_behavior_writes_the_over_limit_detail() {
    let error = projection(&exceeded(), window());
    assert_eq!(
        error.message,
        "Rate limit exceeded for model_per_project_itpm: proj:gpt-4o. Limit type: tokens. Current limit: 100, Remaining: 0. Limit resets at: 2024-06-15 12:01:00 UTC"
    );
    assert_eq!(error.detail, Payload::String(error.message.clone()));
}

#[test]
fn provider_attribution_tests_behavior_sets_the_three_rate_limit_headers() {
    let error = projection(&exceeded(), window());
    let headers = error.headers.expect("the projection sets its headers");
    assert_eq!(headers["retry-after"], "60");
    assert_eq!(headers["rate_limit_type"], "tokens");
    assert_eq!(headers["reset_at"], "2024-06-15 12:01:00 UTC");
}

#[test]
fn provider_attribution_tests_behavior_keeps_a_positive_remaining_count() {
    let mut changed = exceeded();
    changed.limit_remaining = 7;
    let error = projection(&changed, window());
    assert!(
        error
            .message
            .contains("Current limit: 100, Remaining: 7. Limit resets at:")
    );
}

#[rstest]
#[case::tokens("tokens", Some(RateLimitDimension::Tokens))]
#[case::concurrent("max_parallel_requests", Some(RateLimitDimension::ConcurrentRequests))]
#[case::budget("budget", None)]
fn provider_attribution_tests_behavior_carries_the_public_dimension(
    #[case] internal: &str,
    #[case] expected: Option<RateLimitDimension>,
) {
    let mut changed = exceeded();
    changed.rate_limit_type = internal;
    let error = projection(&changed, window());
    assert_eq!(error.rate_limit_type, expected);
    assert_eq!(
        error.headers.expect("the projection sets its headers")["rate_limit_type"],
        internal
    );
}

#[test]
fn provider_attribution_tests_behavior_projects_the_litellm_category_and_provider() {
    let error = projection(&exceeded(), window());
    assert_eq!(error.category, RateLimitCategory::LitellmRateLimit);
    assert_eq!(error.model, "gpt-4o-mini");
    assert_eq!(error.llm_provider, "openai");
    assert_eq!(
        ProxyRateLimitError::PROMETHEUS_EXCEPTION_CLASS,
        "HTTPException"
    );
}

#[test]
fn provider_attribution_tests_behavior_refuses_a_clock_before_the_epoch() {
    let before = UNIX_EPOCH
        .checked_sub(Duration::from_secs(1))
        .expect("time moves forward");
    let window = RateLimitResetWindow {
        now: before,
        window_size: 60,
    };
    let error = v3_rate_limit_error(&exceeded(), window, String::new(), String::new())
        .expect_err("a timestamp before 1970 has no reset time");
    assert!(matches!(
        error,
        ProxyRateLimitProjectionError::ClockBeforeEpoch
    ));
}

#[test]
fn provider_attribution_tests_behavior_refuses_a_reset_time_past_the_calendar() {
    let window = RateLimitResetWindow {
        now: moment(9_000_000_000_000),
        window_size: 0,
    };
    let error = v3_rate_limit_error(&exceeded(), window, String::new(), String::new())
        .expect_err("chrono's calendar ends long before this second");
    assert!(matches!(
        error,
        ProxyRateLimitProjectionError::ResetOutOfRange
    ));
}

#[test]
fn provider_attribution_tests_behavior_refuses_a_window_past_the_epoch_second() {
    let latest = UNIX_EPOCH
        .checked_add(Duration::from_secs(i64::MAX.cast_unsigned()))
        .expect("the largest second fits a SystemTime");
    let window = RateLimitResetWindow {
        now: latest,
        window_size: 1,
    };
    let error = v3_rate_limit_error(&exceeded(), window, String::new(), String::new())
        .expect_err("the sum overflows the epoch second");
    assert!(matches!(
        error,
        ProxyRateLimitProjectionError::ResetOutOfRange
    ));
}
