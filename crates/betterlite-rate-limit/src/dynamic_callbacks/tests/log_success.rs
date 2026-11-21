#![cfg(test)]

use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;
use betterlite_logging::fields::payload::records::StandardLoggingMetadata;
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;

use super::tests::support::{CallbackFixture, Capture, WINDOW, logging_metadata, policy, usage};
use crate::dynamic_callbacks::{LogSuccessEvent, log_success_event};
use crate::dynamic_priority::settings::PriorityPolicy;
use crate::reconciliation_operations::success_event::SuccessResponse;
use crate::usage_resolution::{ResponseObject, Usage};

const MODEL_GROUP: &str = "gpt-4";

fn event<'event>(
    metadata: &'event StandardLoggingMetadata,
    response: &'event ResponseObject,
    model_group: Option<&'event str>,
    rate_limit_type: TokenRateLimitType,
    window_size: i64,
) -> LogSuccessEvent<'event> {
    LogSuccessEvent {
        metadata,
        model_group,
        response: SuccessResponse::Modelled(response),
        rate_limit_type,
        window_size,
    }
}

async fn recorded(
    fixture: &CallbackFixture,
    policy: &PriorityPolicy,
    event: &LogSuccessEvent<'_>,
) -> Vec<(String, f64, Option<i64>)> {
    log_success_event(&fixture.adjustments, policy, event).await;
    fixture
        .usage
        .pipelines()
        .iter()
        .flatten()
        .map(|entry| (entry.key.clone(), entry.increment_value, entry.ttl))
        .collect()
}

fn triple(key: &str, amount: f64, ttl: Option<i64>) -> (String, f64, Option<i64>) {
    (key.to_owned(), amount, ttl)
}

#[tokio::test]
async fn increments_output_tokens_in_both_counters() {
    let fixture = CallbackFixture::new();
    let configured = policy(&[("high", 1.0)]);
    let metadata = logging_metadata(Some("high"));
    let response = ResponseObject::Usage(Usage {
        cached_tokens: Some(4),
        ..usage(10, 7, 17)
    });
    let recorded_ops = recorded(
        &fixture,
        &configured,
        &event(
            &metadata,
            &response,
            Some(MODEL_GROUP),
            TokenRateLimitType::Output,
            WINDOW,
        ),
    )
    .await;
    assert_eq!(
        recorded_ops,
        vec![
            triple("{model_saturation_check:gpt-4}:tokens", 7.0, Some(WINDOW)),
            triple("{priority_model:gpt-4:high}:tokens", 7.0, Some(WINDOW)),
        ]
    );
}

#[tokio::test]
async fn increments_input_tokens_without_the_cached_count() {
    let fixture = CallbackFixture::new();
    let configured = policy(&[("high", 1.0)]);
    let metadata = logging_metadata(Some("high"));
    let response = ResponseObject::Usage(Usage {
        cached_tokens: Some(4),
        ..usage(10, 3, 13)
    });
    let recorded_ops = recorded(
        &fixture,
        &configured,
        &event(
            &metadata,
            &response,
            Some(MODEL_GROUP),
            TokenRateLimitType::Input,
            WINDOW,
        ),
    )
    .await;
    assert_eq!(
        recorded_ops,
        vec![
            triple("{model_saturation_check:gpt-4}:tokens", 6.0, Some(WINDOW)),
            triple("{priority_model:gpt-4:high}:tokens", 6.0, Some(WINDOW)),
        ]
    );
}

#[tokio::test]
async fn increments_total_tokens_without_the_cached_count() {
    let fixture = CallbackFixture::new();
    let configured = policy(&[("high", 1.0)]);
    let metadata = logging_metadata(Some("high"));
    let response = ResponseObject::Usage(Usage {
        cached_tokens: Some(4),
        ..usage(10, 7, 17)
    });
    let recorded_ops = recorded(
        &fixture,
        &configured,
        &event(
            &metadata,
            &response,
            Some(MODEL_GROUP),
            TokenRateLimitType::Total,
            WINDOW,
        ),
    )
    .await;
    assert_eq!(
        recorded_ops,
        vec![
            triple("{model_saturation_check:gpt-4}:tokens", 13.0, Some(WINDOW)),
            triple("{priority_model:gpt-4:high}:tokens", 13.0, Some(WINDOW)),
        ]
    );
}

#[tokio::test]
async fn shares_the_default_pool_for_an_unreserved_priority() {
    let fixture = CallbackFixture::new();
    let configured = policy(&[("high", 1.0)]);
    let metadata = logging_metadata(Some("low"));
    let response = ResponseObject::Usage(usage(10, 7, 17));
    let recorded_ops = recorded(
        &fixture,
        &configured,
        &event(
            &metadata,
            &response,
            Some(MODEL_GROUP),
            TokenRateLimitType::Total,
            WINDOW,
        ),
    )
    .await;
    assert_eq!(
        recorded_ops,
        vec![
            triple("{model_saturation_check:gpt-4}:tokens", 17.0, Some(WINDOW)),
            triple(
                "{priority_model:gpt-4:default_pool}:tokens",
                17.0,
                Some(WINDOW)
            ),
        ]
    );
}

#[tokio::test]
async fn shares_the_default_pool_without_a_priority() {
    let fixture = CallbackFixture::new();
    let configured = policy(&[("high", 1.0)]);
    let metadata = logging_metadata(None);
    let response = ResponseObject::Usage(usage(10, 7, 17));
    let recorded_ops = recorded(
        &fixture,
        &configured,
        &event(
            &metadata,
            &response,
            Some(MODEL_GROUP),
            TokenRateLimitType::Total,
            WINDOW,
        ),
    )
    .await;
    assert_eq!(
        recorded_ops,
        vec![
            triple("{model_saturation_check:gpt-4}:tokens", 17.0, Some(WINDOW)),
            triple(
                "{priority_model:gpt-4:default_pool}:tokens",
                17.0,
                Some(WINDOW)
            ),
        ]
    );
}

#[tokio::test]
async fn does_nothing_without_a_model_group() {
    let fixture = CallbackFixture::new();
    let configured = policy(&[("high", 1.0)]);
    let metadata = logging_metadata(Some("high"));
    let response = ResponseObject::Usage(usage(10, 7, 17));
    let recorded_ops = recorded(
        &fixture,
        &configured,
        &event(
            &metadata,
            &response,
            None,
            TokenRateLimitType::Total,
            WINDOW,
        ),
    )
    .await;
    assert!(recorded_ops.is_empty());
}

#[tokio::test]
async fn does_nothing_without_usage() {
    let fixture = CallbackFixture::new();
    let configured = policy(&[("high", 1.0)]);
    let metadata = logging_metadata(Some("high"));
    let response = ResponseObject::WithUsage(None);
    let recorded_ops = recorded(
        &fixture,
        &configured,
        &event(
            &metadata,
            &response,
            Some(MODEL_GROUP),
            TokenRateLimitType::Total,
            WINDOW,
        ),
    )
    .await;
    assert!(recorded_ops.is_empty());
}

#[tokio::test]
async fn does_nothing_for_an_unmodelled_response() {
    let fixture = CallbackFixture::new();
    let configured = policy(&[("high", 1.0)]);
    let metadata = logging_metadata(Some("high"));
    let success_event = LogSuccessEvent {
        metadata: &metadata,
        model_group: Some(MODEL_GROUP),
        response: SuccessResponse::Unmodelled,
        rate_limit_type: TokenRateLimitType::Total,
        window_size: WINDOW,
    };
    log_success_event(&fixture.adjustments, &configured, &success_event).await;
    assert!(fixture.usage.pipelines().is_empty());
}

#[tokio::test]
async fn does_nothing_for_a_zero_token_count() {
    let fixture = CallbackFixture::new();
    let configured = policy(&[("high", 1.0)]);
    let metadata = logging_metadata(Some("high"));
    let response = ResponseObject::Usage(usage(0, 0, 0));
    let recorded_ops = recorded(
        &fixture,
        &configured,
        &event(
            &metadata,
            &response,
            Some(MODEL_GROUP),
            TokenRateLimitType::Total,
            WINDOW,
        ),
    )
    .await;
    assert!(recorded_ops.is_empty());
}

#[tokio::test]
async fn does_nothing_above_the_exact_float_range() {
    let capture = Capture::start();
    let fixture = CallbackFixture::new();
    let configured = policy(&[("high", 1.0)]);
    let metadata = logging_metadata(Some("high"));
    let response = ResponseObject::Usage(usage(0, 0, 0x0020_0000_0000_0001_u64));
    let recorded_ops = recorded(
        &fixture,
        &configured,
        &event(
            &metadata,
            &response,
            Some(MODEL_GROUP),
            TokenRateLimitType::Total,
            WINDOW,
        ),
    )
    .await;
    assert!(recorded_ops.is_empty());
    assert!(
        capture
            .messages()
            .iter()
            .any(|message| message.contains("Error in the dynamic rate limiter success event"))
    );
}

#[tokio::test]
async fn keeps_the_configured_window_as_the_ttl() {
    let fixture = CallbackFixture::new();
    let configured = policy(&[("high", 1.0)]);
    let metadata = logging_metadata(Some("high"));
    let response = ResponseObject::Usage(usage(10, 7, 17));
    let recorded_ops = recorded(
        &fixture,
        &configured,
        &event(
            &metadata,
            &response,
            Some(MODEL_GROUP),
            TokenRateLimitType::Total,
            120,
        ),
    )
    .await;
    assert_eq!(
        recorded_ops,
        vec![
            triple("{model_saturation_check:gpt-4}:tokens", 17.0, Some(120)),
            triple("{priority_model:gpt-4:high}:tokens", 17.0, Some(120)),
        ]
    );
}

#[tokio::test]
async fn logs_a_known_priority_as_it_is() {
    let capture = Capture::start();
    let fixture = CallbackFixture::new();
    let configured = policy(&[("high", 1.0)]);
    let metadata = logging_metadata(Some("high"));
    let response = ResponseObject::Usage(usage(10, 7, 17));
    assert!(
        !recorded(
            &fixture,
            &configured,
            &event(
                &metadata,
                &response,
                Some(MODEL_GROUP),
                TokenRateLimitType::Total,
                WINDOW,
            ),
        )
        .await
        .is_empty()
    );
    assert!(capture.messages().iter().any(|message| {
        message.contains("Incremented dynamic rate limiter tokens")
            && message.contains("priority=high")
    }));
}

#[tokio::test]
async fn redacts_an_unknown_priority_in_the_log() {
    let capture = Capture::start();
    let fixture = CallbackFixture::new();
    let configured = policy(&[("high", 1.0)]);
    let metadata = logging_metadata(Some("internal-team-name"));
    let response = ResponseObject::Usage(usage(10, 7, 17));
    assert!(
        !recorded(
            &fixture,
            &configured,
            &event(
                &metadata,
                &response,
                Some(MODEL_GROUP),
                TokenRateLimitType::Total,
                WINDOW,
            ),
        )
        .await
        .is_empty()
    );
    let messages = capture.messages();
    assert!(
        messages
            .iter()
            .any(|message| message.contains("priority=REDACTED"))
    );
    assert!(
        messages
            .iter()
            .all(|message| !message.contains("internal-team-name"))
    );
}

#[tokio::test]
async fn redacts_a_missing_priority_in_the_log() {
    let capture = Capture::start();
    let fixture = CallbackFixture::new();
    let configured = policy(&[("high", 1.0)]);
    let metadata = logging_metadata(None);
    let response = ResponseObject::Usage(usage(10, 7, 17));
    assert!(
        !recorded(
            &fixture,
            &configured,
            &event(
                &metadata,
                &response,
                Some(MODEL_GROUP),
                TokenRateLimitType::Total,
                WINDOW,
            ),
        )
        .await
        .is_empty()
    );
    assert!(
        capture
            .messages()
            .iter()
            .any(|message| message.contains("priority=REDACTED"))
    );
}
