use std::sync::Arc;

use betterlite_models::decisions::Json;
use rstest::rstest;

use betterlite_config::sections::logging_privacy::Settings;

use crate::dynamic_callbacks::{DynamicCallback, DynamicCallbackLists};
use crate::exporter_options::redaction::REDACTED_TEXT;
use crate::logging_failure::callbacks::FailureCallbacks;
use crate::logging_failure::handlers::failure_handler;
use crate::logging_failure::{FailureDeps, FailureRecord, FailureReport};
use crate::logging_state::correlation::{CorrelationContext, CorrelationSettings, SecretRedaction};
use crate::logging_state::types::CallbackPolicy;
use crate::logging_state::{LoggingInit, LoggingState};

use super::support::{
    FakeLogger, Harness, build, build_in, call, callbacks, init, object, params, registry, report,
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
fn a_proxy_logger_still_runs_on_a_no_log_request() {
    let ordinary = Arc::new(FakeLogger::new("first"));
    let proxy = Arc::new(FakeLogger::new("_PROXY_MaxParallelRequestsHandler_v3"));
    let registry = registry(&[Arc::clone(&ordinary), Arc::clone(&proxy)]);
    let harness = Harness::new();
    let mut logging_init = init();
    logging_init.litellm_params = params(&[("no-log", Json::Bool(true))]);
    let mut state = build(logging_init);
    let mut record = FailureRecord::default();
    let mut context = CorrelationContext::default();
    run_sync_handler(&mut state, &mut record, &mut context, &registry, &harness);
    assert!(ordinary.calls().is_empty());
    assert_eq!(proxy.calls(), ["sync"]);
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

fn secret_messages() -> Json {
    Json::List(vec![Json::Object(params(&[
        ("role", text("user")),
        ("content", text("the secret prompt")),
    ]))])
}

fn delivered_messages(logging_init: LoggingInit, privacy: Settings) -> Option<Json> {
    let logger = Arc::new(FakeLogger::new("plain"));
    let registry = registry(&[Arc::clone(&logger)]);
    let mut harness = Harness::new();
    harness.privacy = privacy;
    let mut state = build(logging_init);
    let mut record = FailureRecord::default();
    let mut context = CorrelationContext::default();
    run_sync_handler(&mut state, &mut record, &mut context, &registry, &harness);
    logger.recorded()[0].1.messages.clone()
}

fn with_metadata(slot: &str, headers: Json) -> LoggingInit {
    let mut logging_init = init();
    logging_init.litellm_params = params(&[(slot, object(&[("headers", headers)]))]);
    logging_init
}

fn with_dynamic_flag(flag: bool) -> LoggingInit {
    let mut logging_init = init();
    logging_init.callback_dynamic_params =
        params(&[("turn_off_message_logging", Json::Bool(flag))]);
    logging_init
}

fn global_redaction() -> Settings {
    Settings {
        turn_off_message_logging: true,
        redact_user_api_key_info: false,
    }
}

#[test]
fn without_a_redaction_setting_the_messages_are_kept() {
    let messages = delivered_messages(init(), Settings::default());
    assert_eq!(messages, Some(secret_messages()));
}

#[test]
fn a_request_dynamic_param_redacts_the_messages_for_a_default_logger() {
    let messages = delivered_messages(with_dynamic_flag(true), Settings::default());
    assert_eq!(messages, Some(placeholder_messages()));
}

#[test]
fn a_false_dynamic_param_beats_the_global_setting() {
    let messages = delivered_messages(with_dynamic_flag(false), global_redaction());
    assert_eq!(messages, Some(secret_messages()));
}

#[test]
fn the_global_setting_redacts_the_messages() {
    let messages = delivered_messages(init(), global_redaction());
    assert_eq!(messages, Some(placeholder_messages()));
}

#[test]
fn the_enable_header_redacts_the_messages() {
    let headers = object(&[("x-litellm-enable-message-redaction", Json::Bool(true))]);
    let messages = delivered_messages(with_metadata("metadata", headers), Settings::default());
    assert_eq!(messages, Some(placeholder_messages()));
}

#[test]
fn the_enable_header_in_litellm_metadata_redacts_the_messages() {
    let headers = object(&[("litellm-enable-message-redaction", Json::Bool(true))]);
    let messages = delivered_messages(
        with_metadata("litellm_metadata", headers),
        Settings::default(),
    );
    assert_eq!(messages, Some(placeholder_messages()));
}

#[test]
fn the_disable_header_beats_the_global_setting() {
    let headers = object(&[("litellm-disable-message-redaction", Json::Bool(true))]);
    let messages = delivered_messages(with_metadata("metadata", headers), global_redaction());
    assert_eq!(messages, Some(secret_messages()));
}

#[test]
fn an_unreadable_redaction_header_redacts_the_messages() {
    let headers = object(&[(
        "x-litellm-enable-message-redaction",
        text("os.environ/REDACT"),
    )]);
    let messages = delivered_messages(with_metadata("metadata", headers), Settings::default());
    assert_eq!(messages, Some(placeholder_messages()));
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
    assert_eq!(recorded[0].1.messages, None);
    assert_eq!(recorded[0].1.model, "gpt-4o");
}

#[test]
fn an_excluded_field_the_typed_payload_requires_is_kept() {
    let logger = Arc::new(FakeLogger::new("private"));
    let registry = registry(&[Arc::clone(&logger)]);
    let harness = Harness::new();
    let mut deps: FailureDeps<'_> = harness.deps();
    let excluded = vec!["model".to_owned(), "messages".to_owned()];
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
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].1.model, "gpt-4o");
    assert_eq!(recorded[0].1.messages, None);
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
fn dynamic_failure_loggers_run_once_beside_the_registry() {
    let registered = Arc::new(FakeLogger::new("registered"));
    let dynamic_only = Arc::new(FakeLogger::new("dynamic"));
    let registry = registry(&[Arc::clone(&registered)]);
    let harness = Harness::new();
    let lists = DynamicCallbackLists {
        input: None,
        success: None,
        async_success: None,
        async_failure: None,
        failure: Some(vec![
            DynamicCallback::Logger(Arc::<FakeLogger>::clone(&dynamic_only)),
            DynamicCallback::Logger(Arc::<FakeLogger>::clone(&dynamic_only)),
            DynamicCallback::Logger(Arc::<FakeLogger>::clone(&registered)),
        ]),
    };
    let failure_callbacks = FailureCallbacks {
        registry: &registry,
        dynamic: Some(&lists),
    };
    let mut state = build(init());
    let mut record = FailureRecord::default();
    let mut context = CorrelationContext::default();
    failure_handler(
        &mut call(&mut state, &mut record, &mut context),
        &report("provider exploded"),
        &failure_callbacks,
        &harness.deps(),
    );
    assert_eq!(dynamic_only.calls(), ["sync"]);
    assert_eq!(registered.calls(), ["sync"]);
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
#[case::a_proxy_logger_still_runs_on_a_no_log_request(
    a_proxy_logger_still_runs_on_a_no_log_request
)]
#[case::an_excluded_field_the_typed_payload_requires_is_kept(
    an_excluded_field_the_typed_payload_requires_is_kept
)]
#[case::global_disable_no_log_wins(global_disable_no_log_wins)]
#[case::the_first_failure_is_logged_once(the_first_failure_is_logged_once)]
#[case::stream_failures_are_logged_each_time(stream_failures_are_logged_each_time)]
#[case::a_failing_callback_does_not_stop_the_next(a_failing_callback_does_not_stop_the_next)]
#[case::a_turned_off_logger_gets_placeholder_messages(
    a_turned_off_logger_gets_placeholder_messages
)]
#[case::without_a_redaction_setting_the_messages_are_kept(
    without_a_redaction_setting_the_messages_are_kept
)]
#[case::a_request_dynamic_param_redacts_the_messages_for_a_default_logger(
    a_request_dynamic_param_redacts_the_messages_for_a_default_logger
)]
#[case::a_false_dynamic_param_beats_the_global_setting(
    a_false_dynamic_param_beats_the_global_setting
)]
#[case::the_global_setting_redacts_the_messages(the_global_setting_redacts_the_messages)]
#[case::the_enable_header_redacts_the_messages(the_enable_header_redacts_the_messages)]
#[case::the_enable_header_in_litellm_metadata_redacts_the_messages(
    the_enable_header_in_litellm_metadata_redacts_the_messages
)]
#[case::the_disable_header_beats_the_global_setting(the_disable_header_beats_the_global_setting)]
#[case::an_unreadable_redaction_header_redacts_the_messages(
    an_unreadable_redaction_header_redacts_the_messages
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
#[case::dynamic_failure_loggers_run_once_beside_the_registry(
    dynamic_failure_loggers_run_once_beside_the_registry
)]
#[case::the_event_carries_the_stream_flag(the_event_carries_the_stream_flag)]
fn logging_failure_behavior_tests_behavior(#[case] behavior: fn()) {
    behavior();
}
