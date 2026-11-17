use std::sync::{Arc, Mutex};

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::{
    FailingObserver, RecordingObserver, SilentObserver, bad_request, capture_logs, errors_header,
    header, info, json, object, plain, redactor, request,
};
use crate::fallback_observations::details::{
    fallback_error_detail, truncate_fallback_error_detail,
};
use crate::fallback_observations::error_info::FALLBACK_ERRORS_HEADER;
use crate::fallback_observations::headers::{ATTEMPTED_FALLBACKS_HEADER, attach_fallback_headers};
use crate::fallback_observations::messages::{fallback_outcome_message, no_fallback_group_message};
use crate::fallback_observations::observer::{
    FallbackEvent, FallbackObserver, FallbackOutcome, log_fallback_event,
};
use crate::router_core::headers::Response;

#[test]
fn no_fallback_group_message_lists_the_lookup_and_configured_groups() {
    let fallbacks = vec![
        object(r#"{"gpt-4": ["claude-3"], "claude-3": ["claude-2"]}"#),
        object(r#"{"gpt-4": ["gpt-4o"], "claude-2": ["gpt-3.5"]}"#),
    ];
    assert_eq!(
        no_fallback_group_message(&["gpt-4".to_owned(), "openai/gpt-4".to_owned()], &fallbacks),
        "\n\nLiteLLM: model group 'gpt-4 -> openai/gpt-4' failed with the error above and no fallback model group was found for it, so the request was not retried on another model. Fallbacks are configured for: gpt-4, claude-3, claude-2, gpt-3.5. Add a fallbacks entry for that model group (Router fallbacks or proxy router_settings.fallbacks) to retry on another model."
    );
}

#[test]
fn no_fallback_group_message_without_fallbacks() {
    assert_eq!(
        no_fallback_group_message(&["gpt-4".to_owned()], &[]),
        "\n\nLiteLLM: model group 'gpt-4' failed with the error above and no fallback model group was found for it, so the request was not retried on another model. No fallbacks are configured. Add a fallbacks entry for that model group (Router fallbacks or proxy router_settings.fallbacks) to retry on another model."
    );
}

#[test]
fn outcome_without_targets_attempts_nothing() {
    let expected =
        "\n\nLiteLLM: model group 'None' failed with the error above. No fallback was attempted.";
    assert_eq!(fallback_outcome_message(None, None, "boom"), expected);
    assert_eq!(fallback_outcome_message(None, Some(&[]), ""), expected);
}

#[test]
fn outcome_with_configured_targets_names_them() {
    let targets = [json(r#""claude-3""#), json(r#""claude-2""#)];
    assert_eq!(
        fallback_outcome_message(Some("gpt-4"), Some(&targets), ""),
        "\n\nLiteLLM: model group 'gpt-4' failed with the error above. Fallback model group(s) configured: claude-3, claude-2."
    );
}

#[test]
fn outcome_with_a_failure_detail_appends_it() {
    let targets = [json(r#""claude-3""#)];
    assert_eq!(
        fallback_outcome_message(Some("gpt-4"), Some(&targets), "fallback exploded"),
        "\n\nLiteLLM: model group 'gpt-4' failed with the error above. Fallback to claude-3 also failed: fallback exploded"
    );
}

#[test]
fn outcome_masks_secret_targets() {
    let secret = "sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let target = object(&format!(
        r#"{{"model": "claude-3", "api_key": "{secret}"}}"#
    ));
    let message = fallback_outcome_message(Some("gpt-4"), Some(&[Json::Object(target)]), "");
    let visible: String = secret.chars().take(4).collect();
    let stars = "*".repeat(secret.chars().count().saturating_sub(4));
    let masked = format!(r#"{{"api_key":"{visible}{stars}","model":"claude-3"}}"#);
    assert_eq!(
        message,
        format!(
            "\n\nLiteLLM: model group 'gpt-4' failed with the error above. Fallback model group(s) configured: {masked}."
        )
    );
}

#[test]
fn a_long_detail_is_truncated_with_the_suffix() {
    assert_eq!(
        truncate_fallback_error_detail(&"x".repeat(2500)),
        format!("{}... [truncated 500 characters]", "x".repeat(2000))
    );
}

#[test]
fn a_secret_is_redacted_before_the_detail_is_truncated() {
    let message = format!("{} sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "x".repeat(1999));
    let redacted = format!("{} REDACTED", "x".repeat(1999));
    let kept: String = redacted.chars().take(2000).collect();
    assert_eq!(
        fallback_error_detail(&redactor(), &plain(&message)),
        format!("{kept}... [truncated 8 characters]")
    );
}

#[test]
fn a_short_detail_is_redacted_without_truncation() {
    let exception = bad_request(
        "call failed with key sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        None,
        None,
    );
    assert_eq!(
        fallback_error_detail(&redactor(), &exception),
        "call failed with key REDACTED"
    );
}

#[test]
fn a_dict_response_gets_the_attempted_count() {
    let response = attach_fallback_headers(Response::Object(JsonObject::new()), 1, None);
    assert_eq!(
        header(&response, ATTEMPTED_FALLBACKS_HEADER),
        Some(json("1"))
    );
    assert_eq!(header(&response, FALLBACK_ERRORS_HEADER), None);
}

#[test]
fn a_dict_response_merges_new_errors_after_existing_ones() {
    let body = object(
        r#"{"_hidden_params":{"additional_headers":{"x-litellm-fallback-errors":[{"message":"first","type":"APIConnectionError","param":null,"code":null}]}}}"#,
    );
    let errors = [info("second", "Timeout", None, Some("408"))];
    let response = attach_fallback_headers(Response::Object(body), 2, Some(&errors));
    assert_eq!(
        header(&response, ATTEMPTED_FALLBACKS_HEADER),
        Some(json("2"))
    );
    assert_eq!(
        errors_header(&response),
        json(
            r#"[{"message":"first","type":"APIConnectionError","param":null,"code":null},{"message":"second","type":"Timeout","param":null,"code":"408"}]"#
        )
    );
}

#[test]
fn existing_errors_read_from_a_json_string_are_merged() {
    let body = object(
        r#"{"_hidden_params":{"additional_headers":{"x-litellm-fallback-errors":"[{\"message\":\"first\",\"type\":\"APIConnectionError\",\"param\":null,\"code\":null}]"}}}"#,
    );
    let errors = [info("second", "Timeout", None, Some("408"))];
    let response = attach_fallback_headers(Response::Object(body), 1, Some(&errors));
    assert_eq!(
        errors_header(&response),
        json(
            r#"[{"message":"first","type":"APIConnectionError","param":null,"code":null},{"message":"second","type":"Timeout","param":null,"code":"408"}]"#
        )
    );
}

#[test]
fn an_attributed_response_keeps_its_other_hidden_params() {
    let body = object(r#"{"additional_headers":{"keep":"me"},"model_id":"dep-1"}"#);
    let errors = [info("boom", "Exception", None, None)];
    let response = attach_fallback_headers(Response::Attributed(body), 3, Some(&errors));
    assert!(matches!(&response, Response::Attributed(_)));
    assert_eq!(header(&response, "keep"), Some(json(r#""me""#)));
    assert_eq!(
        header(&response, ATTEMPTED_FALLBACKS_HEADER),
        Some(json("3"))
    );
    assert_eq!(
        errors_header(&response),
        json(r#"[{"message":"boom","type":"Exception","param":null,"code":null}]"#)
    );
}

#[test]
fn a_stream_response_is_returned_untouched() {
    let errors = [info("boom", "Exception", None, None)];
    assert_eq!(
        attach_fallback_headers(Response::Stream, 1, Some(&errors)),
        Response::Stream
    );
}

#[test]
fn an_opaque_response_is_returned_untouched() {
    let errors = [info("boom", "Exception", None, None)];
    assert_eq!(
        attach_fallback_headers(Response::Opaque, 1, Some(&errors)),
        Response::Opaque
    );
}

#[test]
fn an_absent_response_stays_absent() {
    assert_eq!(
        attach_fallback_headers(Response::Absent, 0, None),
        Response::Absent
    );
}

#[tokio::test]
async fn a_success_event_reaches_every_observer_in_order() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let observers: Vec<Arc<dyn FallbackObserver>> = vec![
        Arc::new(RecordingObserver::new(0, calls.clone())),
        Arc::new(RecordingObserver::new(1, calls.clone())),
    ];
    let request = request();
    let exception = plain("primary exploded");
    log_fallback_event(
        &observers,
        FallbackOutcome::Success,
        FallbackEvent {
            original_model_group: "gpt-4",
            request: &request,
            exception: &exception,
        },
    )
    .await;
    assert_eq!(
        calls.lock().unwrap().clone(),
        vec![
            "0:success:gpt-4:gpt-4:primary exploded".to_owned(),
            "1:success:gpt-4:gpt-4:primary exploded".to_owned(),
        ]
    );
}

#[tokio::test]
async fn a_failure_event_uses_the_failure_hook() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let observers: Vec<Arc<dyn FallbackObserver>> =
        vec![Arc::new(RecordingObserver::new(7, calls.clone()))];
    let request = request();
    let exception = plain("all targets failed");
    log_fallback_event(
        &observers,
        FallbackOutcome::Failure,
        FallbackEvent {
            original_model_group: "gpt-4",
            request: &request,
            exception: &exception,
        },
    )
    .await;
    assert_eq!(
        calls.lock().unwrap().clone(),
        vec!["7:failure:gpt-4:gpt-4:all targets failed".to_owned()]
    );
}

#[tokio::test]
async fn a_failing_observer_is_logged_and_skipped() {
    let (captured, _guard) = capture_logs();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let observers: Vec<Arc<dyn FallbackObserver>> = vec![
        Arc::new(FailingObserver {
            message: "boom".to_owned(),
        }),
        Arc::new(RecordingObserver::new(1, calls.clone())),
    ];
    let request = request();
    let exception = plain("primary exploded");
    log_fallback_event(
        &observers,
        FallbackOutcome::Success,
        FallbackEvent {
            original_model_group: "gpt-4",
            request: &request,
            exception: &exception,
        },
    )
    .await;
    assert_eq!(
        calls.lock().unwrap().clone(),
        vec!["1:success:gpt-4:gpt-4:primary exploded".to_owned()]
    );
    assert_eq!(
        captured.messages(),
        vec!["Error in log_success_fallback_event: boom".to_owned()]
    );
}

#[tokio::test]
async fn the_default_hooks_do_nothing() {
    let (captured, _guard) = capture_logs();
    let observers: Vec<Arc<dyn FallbackObserver>> = vec![Arc::new(SilentObserver)];
    let request = request();
    let exception = plain("primary exploded");
    log_fallback_event(
        &observers,
        FallbackOutcome::Failure,
        FallbackEvent {
            original_model_group: "gpt-4",
            request: &request,
            exception: &exception,
        },
    )
    .await;
    assert!(captured.messages().is_empty());
}

#[tokio::test]
async fn an_empty_observer_list_does_nothing() {
    let (captured, _guard) = capture_logs();
    let request = request();
    let exception = plain("primary exploded");
    log_fallback_event(
        &[],
        FallbackOutcome::Success,
        FallbackEvent {
            original_model_group: "gpt-4",
            request: &request,
            exception: &exception,
        },
    )
    .await;
    assert!(captured.messages().is_empty());
}
