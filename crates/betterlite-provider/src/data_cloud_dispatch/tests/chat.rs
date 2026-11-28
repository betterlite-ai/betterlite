use std::collections::BTreeMap;

use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use secrecy::{ExposeSecret, SecretString};

use crate::data_cloud_dispatch::call::CompletionCall;
use crate::data_cloud_dispatch::chat::dispatch;
use crate::data_cloud_dispatch::context::DispatchGlobals;
use crate::data_cloud_dispatch::error::DispatchError;
use crate::data_cloud_dispatch::handler::HandlerError;

use super::globals::OwnedGlobals;
use super::support::{Fixture, Recorder, environment, logging, redactor};

#[test]
fn databricks_completion_prefers_the_context_then_the_globals() {
    let mut fixture = Fixture::new();
    fixture.environment = environment(&[
        ("DATABRICKS_API_BASE", "https://env.example.com"),
        ("DATABRICKS_API_KEY", "env-key"),
    ]);
    fixture.api_base = Some(String::from("https://context.example.com"));
    fixture.api_key = Some(SecretString::from("context-key"));
    fixture.headers = Some(BTreeMap::from([(
        String::from("X-Context"),
        String::from("one"),
    )]));
    let mut globals = OwnedGlobals::empty();
    globals.api_base = Some(String::from("https://global.example.com"));
    globals.api_key = Some(SecretString::from("global-key"));
    globals.databricks_key = Some(SecretString::from("databricks-key"));
    let handler = Recorder::succeeding();
    let (outcome, calls, _state) = run_arm(
        ChatConfigTag::Databricks,
        &fixture,
        globals.borrowed(),
        &handler,
    );
    assert_eq!(outcome.ok(), Some(String::from("reply")));
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(
        call.api_base.as_deref(),
        Some("https://context.example.com")
    );
    assert_eq!(
        call.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("context-key")
    );
    assert_eq!(
        call.headers
            .as_ref()
            .and_then(|headers| headers.get("X-Context")),
        Some(&String::from("one"))
    );
    assert_eq!(call.custom_llm_provider, "databricks");
}

#[test]
fn databricks_completion_reads_the_environment_when_no_setting_is_given() {
    let mut fixture = Fixture::new();
    fixture.environment = environment(&[
        ("DATABRICKS_API_BASE", "https://env.example.com"),
        ("DATABRICKS_API_KEY", "env-key"),
    ]);
    let handler = Recorder::succeeding();
    let (outcome, calls, _state) = run_arm(
        ChatConfigTag::Databricks,
        &fixture,
        DispatchGlobals::default(),
        &handler,
    );
    assert_eq!(outcome.ok(), Some(String::from("reply")));
    assert_eq!(
        calls[0].api_base.as_deref(),
        Some("https://env.example.com")
    );
    assert_eq!(
        calls[0].api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("env-key")
    );
}

#[test]
fn databricks_completion_prefers_the_global_key_over_the_databricks_key() {
    let mut fixture = Fixture::new();
    fixture.environment = environment(&[("DATABRICKS_API_KEY", "env-key")]);
    let mut globals = OwnedGlobals::empty();
    globals.api_key = Some(SecretString::from("global-key"));
    globals.databricks_key = Some(SecretString::from("databricks-key"));
    let handler = Recorder::succeeding();
    let (outcome, calls, _state) = run_arm(
        ChatConfigTag::Databricks,
        &fixture,
        globals.borrowed(),
        &handler,
    );
    assert_eq!(outcome.ok(), Some(String::from("reply")));
    assert_eq!(
        calls[0].api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("global-key")
    );
}

#[test]
fn databricks_completion_logs_a_failure_and_returns_the_same_error() {
    let mut fixture = Fixture::new();
    fixture.headers = Some(BTreeMap::from([(
        "X-Request-Tag".to_owned(),
        "one".to_owned(),
    )]));
    let handler = Recorder::failing(500);
    let (outcome, _calls, state) = run_arm(
        ChatConfigTag::Databricks,
        &fixture,
        DispatchGlobals::default(),
        &handler,
    );
    match outcome {
        Err(DispatchError::Handler(HandlerError::Upstream { status_code })) => {
            assert_eq!(status_code, 500);
        }
        other => panic!("expected the handler failure, got {other:?}"),
    }
    let details = &state.model_call_details;
    let error_text = Json::Text(String::from("the provider answered with status 500"));
    assert_eq!(details.get("original_response"), Some(&error_text));
    assert_eq!(
        details.get("log_event_type"),
        Some(&Json::Text(String::from("post_api_call")))
    );
    let Some(Json::List(input)) = details.get("input") else {
        panic!("the post_call log holds the message list");
    };
    assert_eq!(input.len(), 1);
    let Some(Json::Object(args)) = details.get("additional_args") else {
        panic!("the post_call log holds the additional arguments");
    };
    let Some(Json::Object(headers)) = args.get("headers") else {
        panic!("the post_call log holds the headers");
    };
    assert_eq!(
        headers.get("X-Request-Tag"),
        Some(&Json::Text(String::from("one")))
    );
}

#[test]
fn handler_errors_report_their_status() {
    assert_eq!(
        HandlerError::Upstream { status_code: 502 }.status_code(),
        Some(502)
    );
    assert_eq!(HandlerError::EmptyReply.status_code(), None);
}

#[test]
fn databricks_completion_without_a_stream_logs_nothing_on_success() {
    let fixture = Fixture::new();
    let handler = Recorder::succeeding();
    let (outcome, _calls, state) = run_arm(
        ChatConfigTag::Databricks,
        &fixture,
        DispatchGlobals::default(),
        &handler,
    );
    assert_eq!(outcome.ok(), Some(String::from("reply")));
    assert!(state.model_call_details.get("original_response").is_none());
}

#[test]
fn databricks_completion_logs_the_opened_stream_once() {
    let mut fixture = Fixture::new();
    fixture.stream = Some(true);
    let handler = Recorder::succeeding();
    let (outcome, _calls, state) = run_arm(
        ChatConfigTag::Databricks,
        &fixture,
        DispatchGlobals::default(),
        &handler,
    );
    assert_eq!(outcome.ok(), Some(String::from("reply")));
    assert_eq!(
        state.model_call_details.get("original_response"),
        Some(&Json::Null)
    );
    assert_eq!(
        state.model_call_details.get("log_event_type"),
        Some(&Json::Text(String::from("post_api_call")))
    );
}

#[test]
fn snowflake_completion_passes_the_context_values_and_logs_only_a_failure() {
    let mut fixture = Fixture::new();
    fixture.provider = String::from("snowflake");
    fixture.model = String::from("snowflake/claude-3-5-sonnet");
    fixture.api_base = Some(String::from("https://acct.snowflakecomputing.com/api/v2"));
    fixture.api_key = Some(SecretString::from("pat/abc"));
    fixture.stream = Some(true);
    let handler = Recorder::failing(400);
    let (outcome, calls, state) = run_arm(
        ChatConfigTag::Snowflake,
        &fixture,
        DispatchGlobals::default(),
        &handler,
    );
    assert!(matches!(
        outcome,
        Err(DispatchError::Handler(HandlerError::Upstream {
            status_code: 400
        }))
    ));
    assert_eq!(calls[0].custom_llm_provider, "snowflake");
    assert_eq!(
        calls[0].api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("pat/abc")
    );
    assert!(state.model_call_details.get("original_response").is_some());

    let succeeding = Recorder::succeeding();
    let (answered, _calls, second_state) = run_arm(
        ChatConfigTag::Snowflake,
        &fixture,
        DispatchGlobals::default(),
        &succeeding,
    );
    assert_eq!(answered.ok(), Some(String::from("reply")));
    assert!(
        second_state
            .model_call_details
            .get("original_response")
            .is_none()
    );
}

#[test]
fn dispatch_leaves_unknown_tags_to_the_later_arms() {
    let fixture = Fixture::new();
    let handler = Recorder::succeeding();
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = LogContext {
        state: &mut state,
        env: &env,
    };
    let outcome = dispatch(
        ChatConfigTag::OpenAiGpt,
        &fixture.context(DispatchGlobals::default()),
        &handler,
        &mut log,
    );
    assert!(outcome.is_none());
    assert!(handler.calls().is_empty());
}

#[test]
fn oci_completion_passes_the_context_values_without_a_fallback() {
    let mut fixture = Fixture::new();
    fixture.provider = String::from("oci");
    fixture.model = String::from("oci/cohere.command-r-plus");
    let mut globals = OwnedGlobals::empty();
    globals.api_base = Some(String::from("https://global.example.com"));
    globals.api_key = Some(SecretString::from("global-key"));
    globals.headers = Some(BTreeMap::from([(
        String::from("X-Global"),
        String::from("1"),
    )]));
    let handler = Recorder::succeeding();
    let (outcome, calls, state) = run_arm(
        ChatConfigTag::OciChat,
        &fixture,
        globals.borrowed(),
        &handler,
    );
    assert_eq!(outcome.ok(), Some(String::from("reply")));
    let call = &calls[0];
    assert_eq!(call.api_base, None);
    assert!(call.api_key.is_none());
    assert!(call.headers.is_none());
    assert_eq!(call.custom_llm_provider, "oci");
    assert!(state.model_call_details.get("original_response").is_none());
}

#[test]
fn databricks_completion_falls_back_to_the_global_headers() {
    let mut fixture = Fixture::new();
    fixture.environment = environment(&[("DATABRICKS_API_KEY", "env-key")]);
    let mut globals = OwnedGlobals::empty();
    globals.headers = Some(BTreeMap::from([(
        String::from("X-Global"),
        String::from("one"),
    )]));
    let handler = Recorder::succeeding();
    let (outcome, calls, _state) = run_arm(
        ChatConfigTag::Databricks,
        &fixture,
        globals.borrowed(),
        &handler,
    );
    assert_eq!(outcome.ok(), Some(String::from("reply")));
    assert_eq!(
        calls[0]
            .headers
            .as_ref()
            .and_then(|headers| headers.get("X-Global")),
        Some(&String::from("one"))
    );
}

#[test]
fn databricks_completion_names_the_literal_provider() {
    let mut fixture = Fixture::new();
    fixture.provider = String::from("azure");
    let handler = Recorder::succeeding();
    let (outcome, calls, _state) = run_arm(
        ChatConfigTag::Databricks,
        &fixture,
        DispatchGlobals::default(),
        &handler,
    );
    assert_eq!(outcome.ok(), Some(String::from("reply")));
    assert_eq!(calls[0].custom_llm_provider, "databricks");
}

#[test]
fn databricks_completion_clears_the_provider_config() {
    let mut fixture = Fixture::new();
    fixture.provider_config = Some(ChatConfigTag::A2a);
    let handler = Recorder::succeeding();
    let (outcome, calls, _state) = run_arm(
        ChatConfigTag::Databricks,
        &fixture,
        DispatchGlobals::default(),
        &handler,
    );
    assert_eq!(outcome.ok(), Some(String::from("reply")));
    assert_eq!(calls[0].provider_config, None);
}

#[test]
fn databricks_completion_treats_an_empty_context_setting_as_unset() {
    let mut fixture = Fixture::new();
    fixture.environment = environment(&[("DATABRICKS_API_BASE", "https://env.example.com")]);
    fixture.api_base = Some(String::new());
    fixture.api_key = Some(SecretString::from(String::new()));
    let mut globals = OwnedGlobals::empty();
    globals.api_base = Some(String::from("https://global.example.com"));
    globals.api_key = Some(SecretString::from("global-key"));
    let handler = Recorder::succeeding();
    let (outcome, calls, _state) = run_arm(
        ChatConfigTag::Databricks,
        &fixture,
        globals.borrowed(),
        &handler,
    );
    assert_eq!(outcome.ok(), Some(String::from("reply")));
    let call = &calls[0];
    assert_eq!(call.api_base.as_deref(), Some("https://global.example.com"));
    assert_eq!(
        call.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("global-key")
    );
}

#[test]
fn databricks_completion_skips_an_empty_global_key() {
    let mut fixture = Fixture::new();
    fixture.environment = environment(&[("DATABRICKS_API_KEY", "env-key")]);
    let mut globals = OwnedGlobals::empty();
    globals.api_key = Some(SecretString::from(String::new()));
    globals.databricks_key = Some(SecretString::from("databricks-key"));
    let handler = Recorder::succeeding();
    let (outcome, calls, _state) = run_arm(
        ChatConfigTag::Databricks,
        &fixture,
        globals.borrowed(),
        &handler,
    );
    assert_eq!(outcome.ok(), Some(String::from("reply")));
    assert_eq!(
        calls[0].api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("databricks-key")
    );
}

#[test]
fn databricks_completion_skips_an_empty_databricks_key() {
    let mut fixture = Fixture::new();
    fixture.environment = environment(&[("DATABRICKS_API_KEY", "env-key")]);
    let mut globals = OwnedGlobals::empty();
    globals.databricks_key = Some(SecretString::from(String::new()));
    let handler = Recorder::succeeding();
    let (outcome, calls, _state) = run_arm(
        ChatConfigTag::Databricks,
        &fixture,
        globals.borrowed(),
        &handler,
    );
    assert_eq!(outcome.ok(), Some(String::from("reply")));
    assert_eq!(
        calls[0].api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("env-key")
    );
}

fn run_arm(
    tag: ChatConfigTag,
    fixture: &Fixture,
    globals: DispatchGlobals<'_>,
    handler: &Recorder,
) -> (
    Result<String, DispatchError>,
    Vec<CompletionCall>,
    LoggingState,
) {
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = LogContext {
        state: &mut state,
        env: &env,
    };
    let outcome = dispatch(tag, &fixture.context(globals), handler, &mut log)
        .expect("the dispatch table answers");
    (outcome, handler.calls(), state)
}
