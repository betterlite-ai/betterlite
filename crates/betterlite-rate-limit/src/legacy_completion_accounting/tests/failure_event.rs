use rstest::rstest;

use super::support::{Call, Logs, counter, fixture, minute_key};
use crate::legacy_completion_accounting::failure_event::FailureEvent;
use crate::rate_limiters::legacy_rate_limit_counters::usage_cache::LegacyCacheValue;

const GLOBAL: &str = "global_max_parallel_requests";

fn failure<'event>(
    user_api_key: Option<&'event str>,
    exception: &'event str,
) -> FailureEvent<'event> {
    FailureEvent {
        user_api_key,
        global_max_parallel_requests: Some(10),
        exception,
        parent_otel_span: None,
    }
}

#[rstest]
#[case::no_key(
    None,
    "litellm.APIError: upstream timed out",
    "user_api_key: [set=false]"
)]
#[case::limit_message(
    Some("sk-key"),
    "Crossed TPM / RPM / Max Parallel Request Limit",
    "user_api_key: [set=true]"
)]
#[case::limit_message_inside_text(
    Some("sk-key"),
    "litellm.RateLimitError: Crossed TPM / RPM / Max Parallel Request Limit. Hit limit for key",
    "user_api_key: [set=true]"
)]
#[tokio::test]
async fn skip_leaves_every_counter_alone(
    #[case] user_api_key: Option<&str>,
    #[case] exception: &str,
    #[case] printed: &str,
) {
    let fixture = fixture(&[], &[]);
    fixture
        .handler
        .log_failure_event(fixture.cache.as_ref(), &failure(user_api_key, exception))
        .await;
    assert_eq!(fixture.cache.calls(), Vec::new());
    assert_eq!(
        fixture.console.lines(),
        vec![
            String::from("Inside Max Parallel Request Failure Hook"),
            printed.to_owned(),
        ]
    );
}

#[rstest]
#[case::limit_set(Some(10))]
#[case::limit_zero(Some(0))]
#[case::limit_unset(None)]
#[tokio::test]
async fn global_slot_read_then_decremented_local_only(#[case] global_limit: Option<i64>) {
    let fixture = fixture(&[], &[]);
    let mut event = failure(Some("sk-key"), "litellm.APIError: upstream timed out");
    event.global_max_parallel_requests = global_limit;
    fixture
        .handler
        .log_failure_event(fixture.cache.as_ref(), &event)
        .await;
    let key = minute_key("sk-key");
    let mut expected = Vec::new();
    if global_limit.is_some() {
        expected.push(Call::Get {
            key: String::from(GLOBAL),
            local_only: true,
        });
        expected.push(Call::Increment {
            key: String::from(GLOBAL),
            delta: -1,
            local_only: true,
        });
    }
    expected.push(Call::Get {
        key: key.clone(),
        local_only: false,
    });
    expected.push(Call::Set {
        key,
        value: counter(0, 0, 0),
        ttl: 60,
    });
    assert_eq!(fixture.cache.calls(), expected);
}

#[rstest]
#[case::cache_miss(None, counter(0, 0, 0))]
#[case::existing(Some(counter(3, 40, 7)), counter(2, 40, 7))]
#[case::floor_at_zero(Some(counter(0, 5, 1)), counter(0, 5, 1))]
#[tokio::test]
async fn key_counter_gives_back_one_request_only(
    #[case] stored: Option<LegacyCacheValue>,
    #[case] expected: LegacyCacheValue,
) {
    let key = minute_key("sk-key");
    let stored_entries: Vec<(&str, LegacyCacheValue)> = stored
        .map(|value| (key.as_str(), value))
        .into_iter()
        .collect();
    let fixture = fixture(&stored_entries, &[]);
    let mut event = failure(Some("sk-key"), "litellm.APIError: upstream timed out");
    event.global_max_parallel_requests = None;
    fixture
        .handler
        .log_failure_event(fixture.cache.as_ref(), &event)
        .await;
    let calls = fixture.cache.calls();
    assert_eq!(
        calls.last(),
        Some(&Call::Set {
            key: key.clone(),
            value: expected,
            ttl: 60,
        })
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| matches!(call, Call::Set { .. }))
            .count(),
        1
    );
    let LegacyCacheValue::Counter(new_val) = expected else {
        panic!("the expectation is a counter");
    };
    assert_eq!(
        fixture.console.lines().last(),
        Some(&format!("updated_value in failure call: {new_val:?}"))
    );
}

#[rstest]
#[case::failed_read(&[], &["sk-key::2026-11-21-09-05::request_count"], "the usage cache call for sk-key::2026-11-21-09-05::request_count failed")]
#[case::failed_write(&[], &["set sk-key::2026-11-21-09-05::request_count"], "the usage cache call for sk-key::2026-11-21-09-05::request_count failed")]
#[case::failed_global_read(&[], &["global_max_parallel_requests"], "the usage cache call for global_max_parallel_requests failed")]
#[case::bare_number_at_key(&[("sk-key::2026-11-21-09-05::request_count", LegacyCacheValue::Count(2))], &[], "sk-key::2026-11-21-09-05::request_count holds a bare number, not a request counter")]
#[case::request_overflow(&[("sk-key::2026-11-21-09-05::request_count", counter(i64::MIN, 0, 0))], &[], "the counter under sk-key::2026-11-21-09-05::request_count does not fit a 64-bit integer")]
#[tokio::test]
async fn logged_error_goes_to_the_error_log(
    #[case] stored: &[(&str, LegacyCacheValue)],
    #[case] failing: &[&str],
    #[case] message: &str,
) {
    let logs = Logs::capture();
    let fixture = fixture(stored, failing);
    fixture
        .handler
        .log_failure_event(
            fixture.cache.as_ref(),
            &failure(Some("sk-key"), "litellm.APIError: upstream timed out"),
        )
        .await;
    let text = logs.text();
    let expected = format!("Inside Parallel Request Limiter: An exception occurred - {message}");
    let logged = text.lines().find(|line| line.contains(&expected));
    assert!(
        logged.is_some_and(|line| line.starts_with("ERROR")),
        "{text}"
    );
    assert!(
        !fixture
            .console
            .lines()
            .iter()
            .any(|line| line.contains(message))
    );
}

#[tokio::test]
async fn logged_error_stops_before_the_key_counter() {
    let _logs = Logs::capture();
    let fixture = fixture(&[], &[GLOBAL]);
    fixture
        .handler
        .log_failure_event(
            fixture.cache.as_ref(),
            &failure(Some("sk-key"), "litellm.APIError: upstream timed out"),
        )
        .await;
    assert_eq!(
        fixture.cache.calls(),
        vec![Call::Get {
            key: String::from(GLOBAL),
            local_only: true,
        }]
    );
}
