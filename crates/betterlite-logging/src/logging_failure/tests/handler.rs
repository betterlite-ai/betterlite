use std::sync::Arc;

use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::exporter_options::redaction::REDACTED_TEXT;
use crate::logging_failure::events::payload_to_map;
use crate::logging_failure::handlers::failure_handler;
use crate::logging_failure::{FailureDeps, FailureRecord, FailureReport};
use crate::logging_state::LoggingState;
use crate::logging_state::correlation::{CorrelationContext, CorrelationSettings, SecretRedaction};
use crate::logging_state::types::CallbackPolicy;

use super::support::{
    FakeLogger, Harness, build, build_in, call, callbacks, init, params, registry, report,
    run_sync_handler, text,
};

fn claim(state: &LoggingState, name: &str) -> bool {
    state.model_call_details.get(name) == Some(&Json::Bool(true))
}

#[test]
fn sync_request_calls_the_sync_callback() {
    let logger = Arc::new(FakeLogger::new("first"));
    let registry = registry(&[Arc::clone(&logger)]);
    let harness = Harness::new();
    let mut state = build(init());
    let mut record = FailureRecord::default();
    let mut context = CorrelationContext::default();
    run_sync_handler(&mut state, &mut record, &mut context, &registry, &harness);
    assert_eq!(logger.calls(), ["sync"]);
    assert_eq!(harness.payloads.calls(), 1);
    assert!(claim(&state, "has_logged_sync_failure"));
}

#[test]
fn async_sdk_requests_skip_the_sync_callback() {
    let logger = Arc::new(FakeLogger::new("first"));
    let registry = registry(&[Arc::clone(&logger)]);
    let harness = Harness::new();
    let mut logging_init = init();
    logging_init.litellm_params = params(&[("acompletion", Json::Bool(true))]);
    let mut state = build(logging_init);
    let mut record = FailureRecord::default();
    let mut context = CorrelationContext::default();
    run_sync_handler(&mut state, &mut record, &mut context, &registry, &harness);
    assert!(logger.calls().is_empty());
    assert!(claim(&state, "has_logged_sync_failure"));
}

#[test]
fn passthrough_requests_skip_the_sync_callback() {
    let logger = Arc::new(FakeLogger::new("first"));
    let registry = registry(&[Arc::clone(&logger)]);
    let harness = Harness::new();
    let mut logging_init = init();
    logging_init.call_type = "pass_through_endpoint".to_owned();
    let mut state = build(logging_init);
    let mut record = FailureRecord::default();
    let mut context = CorrelationContext::default();
    run_sync_handler(&mut state, &mut record, &mut context, &registry, &harness);
    assert!(logger.calls().is_empty());
}

#[test]
fn no_log_requests_skip_the_callback_but_take_the_claim() {
    let logger = Arc::new(FakeLogger::new("first"));
    let registry = registry(&[Arc::clone(&logger)]);
    let harness = Harness::new();
    let mut logging_init = init();
    logging_init.litellm_params = params(&[("no-log", Json::Bool(true))]);
    let mut state = build(logging_init);
    let mut record = FailureRecord::default();
    let mut context = CorrelationContext::default();
    run_sync_handler(&mut state, &mut record, &mut context, &registry, &harness);
    assert!(logger.calls().is_empty());
    assert!(claim(&state, "has_logged_sync_failure"));
}

#[test]
fn global_disable_no_log_wins() {
    let logger = Arc::new(FakeLogger::new("first"));
    let registry = registry(&[Arc::clone(&logger)]);
    let harness = Harness::new();
    let mut logging_init = init();
    logging_init.litellm_params = params(&[("no-log", Json::Bool(true))]);
    logging_init.callback_policy = CallbackPolicy {
        global_disable_no_log_param: true,
    };
    let mut state = build(logging_init);
    let mut record = FailureRecord::default();
    let mut context = CorrelationContext::default();
    run_sync_handler(&mut state, &mut record, &mut context, &registry, &harness);
    assert_eq!(logger.calls(), ["sync"]);
}

#[test]
fn the_first_failure_is_logged_once() {
    let logger = Arc::new(FakeLogger::new("first"));
    let registry = registry(&[Arc::clone(&logger)]);
    let harness = Harness::new();
    let mut state = build(init());
    let mut record = FailureRecord::default();
    let mut context = CorrelationContext::default();
    run_sync_handler(&mut state, &mut record, &mut context, &registry, &harness);
    run_sync_handler(&mut state, &mut record, &mut context, &registry, &harness);
    assert_eq!(logger.calls(), ["sync"]);
    assert_eq!(harness.payloads.calls(), 1);
}

#[test]
fn stream_failures_are_logged_each_time() {
    let logger = Arc::new(FakeLogger::new("first"));
    let registry = registry(&[Arc::clone(&logger)]);
    let harness = Harness::new();
    let mut logging_init = init();
    logging_init.stream = Some(true);
    let mut state = build(logging_init);
    let mut record = FailureRecord::default();
    let mut context = CorrelationContext::default();
    run_sync_handler(&mut state, &mut record, &mut context, &registry, &harness);
    run_sync_handler(&mut state, &mut record, &mut context, &registry, &harness);
    assert_eq!(logger.calls(), ["sync", "sync"]);
    assert!(!claim(&state, "has_logged_sync_failure"));
}

#[test]
fn a_failing_callback_does_not_stop_the_next() {
    let first = Arc::new(FakeLogger::new("first").failing_sync());
    let second = Arc::new(FakeLogger::new("second"));
    let registry = registry(&[Arc::clone(&first), Arc::clone(&second)]);
    let harness = Harness::new();
    let mut state = build(init());
    let mut record = FailureRecord::default();
    let mut context = CorrelationContext::default();
    run_sync_handler(&mut state, &mut record, &mut context, &registry, &harness);
    assert_eq!(first.calls(), ["sync"]);
    assert_eq!(second.calls(), ["sync"]);
}

#[test]
fn a_turned_off_logger_gets_placeholder_messages() {
    let logger = Arc::new(FakeLogger::new("private").turning_off_messages());
    let registry = registry(&[Arc::clone(&logger)]);
    let harness = Harness::new();
    let mut state = build(init());
    let mut record = FailureRecord::default();
    let mut context = CorrelationContext::default();
    run_sync_handler(&mut state, &mut record, &mut context, &registry, &harness);
    let recorded = logger.recorded();
    assert_eq!(recorded.len(), 1);
    let messages = recorded[0]
        .1
        .messages
        .clone()
        .expect("messages are present");
    assert_eq!(messages, placeholder_messages());
}

#[test]
fn a_self_redacting_logger_keeps_its_messages() {
    let logger = Arc::new(
        FakeLogger::new("private")
            .turning_off_messages()
            .redacting_messages_itself(),
    );
    let registry = registry(&[Arc::clone(&logger)]);
    let harness = Harness::new();
    let mut state = build(init());
    let mut record = FailureRecord::default();
    let mut context = CorrelationContext::default();
    run_sync_handler(&mut state, &mut record, &mut context, &registry, &harness);
    let recorded = logger.recorded();
    assert_eq!(
        recorded[0].1.messages,
        Some(Json::List(vec![Json::Object(params(&[
            ("role", text("user")),
            ("content", text("the secret prompt")),
        ]))]))
    );
}

#[test]
fn excluded_fields_are_removed_from_the_event() {
    let logger = Arc::new(FakeLogger::new("private"));
    let registry = registry(&[Arc::clone(&logger)]);
    let harness = Harness::new();
    let mut deps: FailureDeps<'_> = harness.deps();
    let excluded = vec!["messages".to_owned()];
    deps.excluded_fields = &excluded;
    let mut state = build(init());
    let mut record = FailureRecord::default();
    let mut context = CorrelationContext::default();
    let mut failure_call = call(&mut state, &mut record, &mut context);
    failure_handler(
        &mut failure_call,
        &report("provider exploded"),
        &callbacks(&registry),
        &deps,
    );
    let recorded = logger.recorded();
    let payload = payload_to_map(recorded[0].1.as_ref()).expect("the payload maps");
    assert!(!payload.contains_key("messages"));
}

#[test]
fn a_missing_payload_skips_the_callbacks_but_keeps_the_claim() {
    let logger = Arc::new(FakeLogger::new("first"));
    let registry = registry(&[Arc::clone(&logger)]);
    let harness = Harness::without_payload();
    let mut state = build(init());
    let mut record = FailureRecord::default();
    let mut context = CorrelationContext::default();
    run_sync_handler(&mut state, &mut record, &mut context, &registry, &harness);
    assert!(logger.calls().is_empty());
    assert!(claim(&state, "has_logged_sync_failure"));
}

#[test]
fn a_preparation_error_skips_the_callbacks_and_the_claim() {
    let logger = Arc::new(FakeLogger::new("first"));
    let registry = registry(&[Arc::clone(&logger)]);
    let harness = Harness::new();
    let mut logging_init = init();
    logging_init.litellm_params = params(&[("metadata", text("not-a-map"))]);
    let mut state = build(logging_init);
    let mut record = FailureRecord::default();
    let mut context = CorrelationContext::default();
    let mut failure_call = call(&mut state, &mut record, &mut context);
    let mut facts = super::support::exception("provider exploded");
    let mut headers = indexmap::IndexMap::new();
    headers.insert("x-request-id".to_owned(), "request-1".to_owned());
    facts.headers = Some(headers);
    let failure = FailureReport {
        exception: Arc::new(facts),
        traceback: String::new(),
        start_time: None,
        end_time: None,
    };
    failure_handler(
        &mut failure_call,
        &failure,
        &callbacks(&registry),
        &harness.deps(),
    );
    assert!(logger.calls().is_empty());
    assert!(!claim(&state, "has_logged_sync_failure"));
    assert!(record.payload.is_none());
}

#[test]
fn the_context_is_restored() {
    let logger = Arc::new(FakeLogger::new("first"));
    let registry = registry(&[Arc::clone(&logger)]);
    let harness = Harness::new();
    let mut context = CorrelationContext {
        trace_id: "outer-trace".to_owned(),
        session_id: "outer-session".to_owned(),
    };
    let settings = CorrelationSettings {
        supports_correlation_logging: true,
        request_correlation_in_logs: true,
        secret_redaction: SecretRedaction::Enabled,
    };
    let mut logging_init = init();
    logging_init.litellm_trace_id = Some("inner-trace".to_owned());
    let mut state = build_in(logging_init, &mut context, settings);
    assert_eq!(context.trace_id, "inner-trace");
    let mut record = FailureRecord::default();
    run_sync_handler(&mut state, &mut record, &mut context, &registry, &harness);
    assert_eq!(context.trace_id, "outer-trace");
    assert_eq!(context.session_id, "outer-session");
}

#[test]
fn the_event_carries_the_stream_flag() {
    let logger = Arc::new(FakeLogger::new("first"));
    let registry = registry(&[Arc::clone(&logger)]);
    let harness = Harness::new();
    let mut logging_init = init();
    logging_init.stream = Some(true);
    let mut state = build(logging_init);
    let mut record = FailureRecord::default();
    let mut context = CorrelationContext::default();
    run_sync_handler(&mut state, &mut record, &mut context, &registry, &harness);
    assert!(logger.recorded()[0].2);
}

fn placeholder_messages() -> Json {
    let mut message = indexmap::IndexMap::new();
    message.insert("content".to_owned(), text(REDACTED_TEXT));
    message.insert("role".to_owned(), text("assistant"));
    message.insert("tool_calls".to_owned(), Json::Null);
    message.insert("function_call".to_owned(), Json::Null);
    message.insert("provider_specific_fields".to_owned(), Json::Null);
    Json::List(vec![Json::Object(message)])
}

#[rstest]
#[case::sync_request_calls_the_sync_callback(sync_request_calls_the_sync_callback)]
#[case::async_sdk_requests_skip_the_sync_callback(async_sdk_requests_skip_the_sync_callback)]
#[case::passthrough_requests_skip_the_sync_callback(passthrough_requests_skip_the_sync_callback)]
#[case::no_log_requests_skip_the_callback_but_take_the_claim(
    no_log_requests_skip_the_callback_but_take_the_claim
)]
#[case::global_disable_no_log_wins(global_disable_no_log_wins)]
#[case::the_first_failure_is_logged_once(the_first_failure_is_logged_once)]
#[case::stream_failures_are_logged_each_time(stream_failures_are_logged_each_time)]
#[case::a_failing_callback_does_not_stop_the_next(a_failing_callback_does_not_stop_the_next)]
#[case::a_turned_off_logger_gets_placeholder_messages(
    a_turned_off_logger_gets_placeholder_messages
)]
#[case::a_self_redacting_logger_keeps_its_messages(a_self_redacting_logger_keeps_its_messages)]
#[case::excluded_fields_are_removed_from_the_event(excluded_fields_are_removed_from_the_event)]
#[case::a_missing_payload_skips_the_callbacks_but_keeps_the_claim(
    a_missing_payload_skips_the_callbacks_but_keeps_the_claim
)]
#[case::a_preparation_error_skips_the_callbacks_and_the_claim(
    a_preparation_error_skips_the_callbacks_and_the_claim
)]
#[case::the_context_is_restored(the_context_is_restored)]
#[case::the_event_carries_the_stream_flag(the_event_carries_the_stream_flag)]
fn logging_failure_behavior_tests_behavior(#[case] behavior: fn()) {
    behavior();
}
