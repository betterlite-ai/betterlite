use betterlite_core::api::request_principal_metadata::PrincipalFacts;
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use rstest::rstest;
use serde_json::Number;

use super::support::{Call, counter, fixture, minute_key};
use crate::legacy_completion_accounting::error::AccountingError;
use crate::legacy_completion_accounting::remaining_headers::{
    PostCallHook, post_call_success_hook,
};
use crate::rate_limiters::legacy_rate_limit_counters::usage_cache::LegacyCacheValue;

fn principal(fields: &str) -> PrincipalFacts {
    serde_json::from_str(fields).unwrap()
}

fn headers(entries: &[(&str, i64)]) -> AdditionalHeaders {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), Json::Number(Number::from(*value))))
        .collect()
}

const fn hook<'call>(
    principal: &'call PrincipalFacts,
    limits: (Option<i64>, Option<i64>),
    additional_headers: Option<&'call mut AdditionalHeaders>,
) -> PostCallHook<'call> {
    PostCallHook {
        principal,
        rpm_limit: limits.0,
        tpm_limit: limits.1,
        parent_otel_span: None,
        additional_headers,
    }
}

#[rstest]
#[case::both_limits((Some(10), Some(100)), counter(1, 30, 4), &[("x-ratelimit-remaining-requests", 6), ("x-ratelimit-limit-requests", 10), ("x-ratelimit-remaining-tokens", 70), ("x-ratelimit-limit-tokens", 100)])]
#[case::rpm_only((Some(10), None), counter(1, 30, 4), &[("x-ratelimit-remaining-requests", 6), ("x-ratelimit-limit-requests", 10)])]
#[case::tpm_only((None, Some(100)), counter(1, 30, 4), &[("x-ratelimit-remaining-tokens", 70), ("x-ratelimit-limit-tokens", 100)])]
#[case::over_the_limit((Some(10), Some(20)), counter(1, 25, 12), &[("x-ratelimit-remaining-requests", -2), ("x-ratelimit-limit-requests", 10), ("x-ratelimit-remaining-tokens", -5), ("x-ratelimit-limit-tokens", 20)])]
#[case::no_limits((None, None), counter(1, 30, 4), &[])]
#[tokio::test]
async fn headers_written_from_the_key_counter(
    #[case] limits: (Option<i64>, Option<i64>),
    #[case] stored: LegacyCacheValue,
    #[case] written: &[(&str, i64)],
) {
    let fixture = fixture(&[("sk-key::2026-11-21-09-05::request_count", stored)], &[]);
    let key = principal(r#"{"api_key": "sk-key"}"#);
    let mut response_headers = headers(&[("x-request-id", 7), ("x-ratelimit-limit-requests", 999)]);
    let answer = post_call_success_hook(
        &fixture.handler,
        fixture.cache.as_ref(),
        hook(&key, limits, Some(&mut response_headers)),
    )
    .await;
    answer.unwrap();
    let mut expected = headers(&[("x-request-id", 7), ("x-ratelimit-limit-requests", 999)]);
    expected.extend(headers(written));
    assert_eq!(
        response_headers.into_iter().collect::<Vec<_>>(),
        expected.into_iter().collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn headers_untouched_without_a_counter() {
    let fixture = fixture(&[], &[]);
    let key = principal(r#"{"api_key": "sk-key"}"#);
    let mut response_headers = headers(&[("x-request-id", 7)]);
    let answer = post_call_success_hook(
        &fixture.handler,
        fixture.cache.as_ref(),
        hook(&key, (Some(10), Some(100)), Some(&mut response_headers)),
    )
    .await;
    answer.unwrap();
    assert_eq!(response_headers, headers(&[("x-request-id", 7)]));
}

#[tokio::test]
async fn headers_skipped_without_hidden_params_after_the_read() {
    let fixture = fixture(
        &[("sk-key::2026-11-21-09-05::request_count", counter(1, 30, 4))],
        &[],
    );
    let key = principal(r#"{"api_key": "sk-key"}"#);
    let answer = post_call_success_hook(
        &fixture.handler,
        fixture.cache.as_ref(),
        hook(&key, (Some(10), Some(100)), None),
    )
    .await;
    answer.unwrap();
    assert_eq!(
        fixture.cache.calls(),
        vec![Call::Get {
            key: minute_key("sk-key"),
            local_only: false,
        }]
    );
}

#[rstest]
#[case::session_token_alias(
    r#"{"api_key": "hashed", "is_session_token": true, "key_alias": "dashboard"}"#,
    "dashboard"
)]
#[case::alias_of_a_plain_key(
    r#"{"api_key": "hashed", "is_session_token": false, "key_alias": "dashboard"}"#,
    "hashed"
)]
#[case::session_token_without_alias(r#"{"api_key": "hashed", "is_session_token": true}"#, "hashed")]
#[tokio::test]
async fn headers_read_under_the_logged_api_key(#[case] fields: &str, #[case] logged: &str) {
    let fixture = fixture(&[], &[]);
    let key = principal(fields);
    let answer = post_call_success_hook(
        &fixture.handler,
        fixture.cache.as_ref(),
        hook(&key, (None, None), None),
    )
    .await;
    answer.unwrap();
    assert_eq!(
        fixture.cache.calls(),
        vec![Call::Get {
            key: minute_key(logged),
            local_only: false,
        }]
    );
}

#[rstest]
#[case::failed_read(&[], &["sk-key::2026-11-21-09-05::request_count"])]
#[case::bare_number(&[("sk-key::2026-11-21-09-05::request_count", LegacyCacheValue::Count(4))], &[])]
#[tokio::test]
async fn headers_cache_error_reaches_the_caller(
    #[case] stored: &[(&str, LegacyCacheValue)],
    #[case] failing: &[&str],
) {
    let fixture = fixture(stored, failing);
    let key = principal(r#"{"api_key": "sk-key"}"#);
    let mut response_headers = headers(&[]);
    let answer = post_call_success_hook(
        &fixture.handler,
        fixture.cache.as_ref(),
        hook(&key, (Some(10), Some(100)), Some(&mut response_headers)),
    )
    .await;
    assert!(matches!(
        answer,
        Err(AccountingError::Cache { .. } | AccountingError::NotACounter { .. })
    ));
    assert_eq!(response_headers, headers(&[]));
}

#[rstest]
#[case::rpm_overflow((Some(i64::MIN), None), counter(1, 0, 1))]
#[case::tpm_overflow((None, Some(i64::MIN)), counter(1, 1, 0))]
#[tokio::test]
async fn headers_overflow_is_an_error(
    #[case] limits: (Option<i64>, Option<i64>),
    #[case] stored: LegacyCacheValue,
) {
    let fixture = fixture(&[("sk-key::2026-11-21-09-05::request_count", stored)], &[]);
    let key = principal(r#"{"api_key": "sk-key"}"#);
    let mut response_headers = headers(&[]);
    let answer = post_call_success_hook(
        &fixture.handler,
        fixture.cache.as_ref(),
        hook(&key, limits, Some(&mut response_headers)),
    )
    .await;
    assert!(matches!(answer, Err(AccountingError::Overflow { .. })));
}

#[tokio::test]
async fn none_key_reads_the_none_text_key() {
    let fixture = fixture(
        &[("None::2026-11-21-09-05::request_count", counter(1, 30, 4))],
        &[],
    );
    let key = principal("{}");
    let mut response_headers = headers(&[]);
    let answer = post_call_success_hook(
        &fixture.handler,
        fixture.cache.as_ref(),
        hook(&key, (Some(10), None), Some(&mut response_headers)),
    )
    .await;
    answer.unwrap();
    assert_eq!(
        fixture.cache.calls(),
        vec![Call::Get {
            key: String::from("None::2026-11-21-09-05::request_count"),
            local_only: false,
        }]
    );
    assert_eq!(
        response_headers,
        headers(&[
            ("x-ratelimit-remaining-requests", 6),
            ("x-ratelimit-limit-requests", 10)
        ])
    );
}
