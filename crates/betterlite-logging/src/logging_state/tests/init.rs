use betterlite_models::decisions::Json;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use std::time::Duration;

use super::support::{
    build, build_clocked, build_in, encoded, init, object, params, text, try_build,
};
use crate::logging_state::correlation::{CorrelationContext, CorrelationSettings, SecretRedaction};
use crate::logging_state::types::{Messages, RequestLoggingFlags};
use crate::logging_state::{LoggingInit, LoggingState};

enum Scenario {
    Text,
    StringList,
    ObjectList,
    MixedList,
    Absent,
    GeneratedTraceId,
    EmptyTraceId,
    Session(Json),
    BrokenSession,
    Details,
    Dependencies,
    DebugOutput,
    CopyTiming,
    Correlation(CorrelationSettings, &'static str),
    OmittedSettings(&'static str),
}

fn input(state: &LoggingState) -> String {
    encoded(&state.model_call_details.get("input").cloned())
}

fn normalize_outcome(messages: Option<Messages>) -> String {
    let state = build(LoggingInit { messages, ..init() });
    format!("{}/{}", encoded(&state.messages), input(&state))
}

fn trace_outcome(litellm_trace_id: Option<String>) -> String {
    let state = build(LoggingInit {
        litellm_trace_id,
        ..init()
    });
    format!(
        "{}/{}",
        state.litellm_trace_id,
        encoded(&state.model_call_details.get("litellm_trace_id").cloned())
    )
}

fn session_outcome(session: &Json) -> String {
    let state = build(LoggingInit {
        kwargs: params(&[("litellm_session_id", session.clone())]),
        ..init()
    });
    state.litellm_session_id
}

fn broken_session_outcome() -> String {
    let mut fixture = init();
    fixture.kwargs = params(&[(
        "litellm_session_id",
        Json::Number(serde_json::Number::from(7_i32)),
    )]);
    try_build(fixture, Duration::default)
        .expect_err("a numeric session id is refused")
        .to_string()
}

fn details_outcome() -> String {
    let state = build(LoggingInit {
        litellm_params: params(&[("api_base", text("http://proxy"))]),
        applied_guardrails: Some(vec!["guard".to_owned()]),
        ..init()
    });
    encoded(&state.model_call_details)
}

fn trusted_json(state: &LoggingState) -> Json {
    Json::List(
        state
            .trusted_callback_vars
            .iter()
            .map(|(name, value)| {
                Json::List(vec![
                    Json::Text(name.clone()),
                    Json::Text(value.expose_secret().to_owned()),
                ])
            })
            .collect(),
    )
}

fn dependencies_outcome() -> String {
    let state = build(LoggingInit {
        litellm_params: params(&[("api_base", text("http://proxy"))]),
        callback_dynamic_params: params(&[("langfuse_host", text("h"))]),
        trusted_callback_vars: vec![("dd_api_key".to_owned(), SecretString::from("k"))],
        standard_built_in_tools_params: params(&[("web_search_options", Json::Null)]),
        request_logging: RequestLoggingFlags {
            log_raw_request_response: true,
            raw_request_only: true,
        },
        ..init()
    });
    let summary = object(&[
        ("litellm_params", Json::Object(state.litellm_params.clone())),
        (
            "standard_callback_dynamic_params",
            Json::Object(state.callback_dynamic_params.clone()),
        ),
        ("trusted_callback_vars", trusted_json(&state)),
        (
            "standard_built_in_tools_params",
            Json::Object(state.standard_built_in_tools_params.clone()),
        ),
        ("custom_pricing", Json::Bool(state.cost.custom_pricing)),
    ]);
    format!(
        "{}/{}/{}/{}",
        encoded(&summary),
        encoded(&state.request_logging),
        encoded(&state.completion_start_time),
        encoded(&state.response_timing_metrics)
    )
}

fn debug_outcome() -> String {
    let secrets = ["sk-param", "lf-param", "dd-trusted"];
    let mut fixture = init();
    fixture
        .litellm_params
        .insert("api_key".to_owned(), text(secrets[0]));
    fixture
        .callback_dynamic_params
        .insert("langfuse_secret_key".to_owned(), text(secrets[1]));
    fixture
        .trusted_callback_vars
        .push(("dd_api_key".to_owned(), SecretString::from(secrets[2])));
    let printed = format!("{fixture:?} {:?}", build(fixture.clone()));
    secrets
        .iter()
        .any(|secret| printed.contains(secret))
        .to_string()
}

fn timing_outcome() -> String {
    let step = Duration::from_millis(250);
    let mut now = Duration::ZERO;
    let clock = move || {
        let reading = now;
        now = now.saturating_add(step);
        reading
    };
    let state = build_clocked(
        LoggingInit {
            messages: Some(Messages::Text("hi".to_owned())),
            ..init()
        },
        clock,
    );
    format!(
        "{}/{}",
        state.message_copy_duration_ms, state.callback_duration_ms
    )
}

fn omitted_settings_outcome(settings: &str) -> String {
    serde_json::from_str::<CorrelationSettings>(settings).map_or_else(
        |error| error.to_string(),
        |parsed| correlation_outcome(parsed, "inner\r-trace"),
    )
}

fn correlation_outcome(settings: CorrelationSettings, trace_id: &str) -> String {
    let mut context = CorrelationContext {
        trace_id: "outer-trace".to_owned(),
        session_id: "outer-session".to_owned(),
    };
    let state = build_in(
        LoggingInit {
            litellm_trace_id: Some(trace_id.to_owned()),
            kwargs: params(&[("litellm_session_id", text("inner-session"))]),
            ..init()
        },
        &mut context,
        settings,
    );
    format!(
        "{}/{}/{}",
        encoded(&context),
        encoded(&state.correlation.pre_call),
        encoded(&state.correlation.own)
    )
}

fn outcome(scenario: &Scenario) -> String {
    match scenario {
        Scenario::Text => normalize_outcome(Some(Messages::Text("hi".to_owned()))),
        Scenario::StringList => {
            normalize_outcome(Some(Messages::Values(vec![text("a"), text("b")])))
        }
        Scenario::ObjectList => normalize_outcome(Some(Messages::Values(vec![object(&[
            ("role", text("system")),
            ("content", text("s")),
        ])]))),
        Scenario::MixedList => normalize_outcome(Some(Messages::Values(vec![
            text("a"),
            object(&[("role", text("system"))]),
        ]))),
        Scenario::Absent => normalize_outcome(None),
        Scenario::GeneratedTraceId => trace_outcome(None),
        Scenario::EmptyTraceId => trace_outcome(Some(String::new())),
        Scenario::Session(session) => session_outcome(session),
        Scenario::BrokenSession => broken_session_outcome(),
        Scenario::Details => details_outcome(),
        Scenario::Dependencies => dependencies_outcome(),
        Scenario::DebugOutput => debug_outcome(),
        Scenario::CopyTiming => timing_outcome(),
        Scenario::Correlation(settings, trace_id) => correlation_outcome(*settings, trace_id),
        Scenario::OmittedSettings(settings) => omitted_settings_outcome(settings),
    }
}

#[rstest]
#[case::text_becomes_a_user_message(Scenario::Text, r#"[{"role":"user","content":"hi"}]/"hi""#)]
#[case::string_list_wraps_every_element(
    Scenario::StringList,
    r#"[{"role":"user","content":"a"},{"role":"user","content":"b"}]/["a","b"]"#
)]
#[case::object_list_is_kept(
    Scenario::ObjectList,
    r#"[{"role":"system","content":"s"}]/[{"role":"system","content":"s"}]"#
)]
#[case::mixed_list_follows_its_first_element(Scenario::MixedList, r#"[{"role":"user","content":"a"},{"role":"user","content":{"role":"system"}}]/["a",{"role":"system"}]"#)]
#[case::absent_messages_stay_absent(Scenario::Absent, "null/null")]
#[case::missing_trace_id_is_generated(
    Scenario::GeneratedTraceId,
    r#"generated-trace/"generated-trace""#
)]
#[case::an_explicit_trace_id_is_used_as_given(Scenario::EmptyTraceId, r#"/"""#)]
#[case::session_id_comes_from_kwargs(Scenario::Session(text("session-1")), "session-1")]
#[case::null_session_id_is_empty(Scenario::Session(Json::Null), "")]
#[case::a_numeric_session_id_names_the_field(
    Scenario::BrokenSession,
    "the litellm_session_id value must be a string"
)]
#[case::initial_details_keep_the_original_input(Scenario::Details, r#"{"litellm_trace_id":"trace-1","litellm_call_id":"call-1","input":null,"litellm_params":{"api_base":"http://proxy"},"applied_guardrails":["guard"],"model":"gpt-4o"}"#)]
#[case::injected_dependencies_are_stored(Scenario::Dependencies, r#"{"litellm_params":{"api_base":"http://proxy"},"standard_callback_dynamic_params":{"langfuse_host":"h"},"trusted_callback_vars":[["dd_api_key","k"]],"standard_built_in_tools_params":{"web_search_options":null},"custom_pricing":false}/{"log_raw_request_response":true,"raw_request_only":true}/null/{}"#)]
#[case::debug_output_hides_credentials(Scenario::DebugOutput, "false")]
#[case::disabled_correlation_leaves_the_context_alone(
    Scenario::Correlation(CorrelationSettings {
        supports_correlation_logging: true,
        request_correlation_in_logs: false,
        secret_redaction: SecretRedaction::Enabled,
    }, "inner\r-trace"), r#"{"trace_id":"outer-trace","session_id":"outer-session"}/{"trace_id":"outer-trace","session_id":"outer-session"}/{"trace_id":"outer-trace","session_id":"outer-session"}"#)]
#[case::unsupported_correlation_leaves_the_context_alone(
    Scenario::Correlation(CorrelationSettings {
        supports_correlation_logging: false,
        request_correlation_in_logs: true,
        secret_redaction: SecretRedaction::Enabled,
    }, "inner\r-trace"), r#"{"trace_id":"outer-trace","session_id":"outer-session"}/{"trace_id":"outer-trace","session_id":"outer-session"}/{"trace_id":"outer-trace","session_id":"outer-session"}"#)]
#[case::enabled_correlation_stamps_sanitized_ids(
    Scenario::Correlation(CorrelationSettings {
        supports_correlation_logging: true,
        request_correlation_in_logs: true,
        secret_redaction: SecretRedaction::Enabled,
    }, "inner\r-trace"), r#"{"trace_id":"inner-trace","session_id":"inner-session"}/{"trace_id":"outer-trace","session_id":"outer-session"}/{"trace_id":"inner-trace","session_id":"inner-session"}"#)]
#[case::an_omitted_support_flag_defaults_to_true(
    Scenario::OmittedSettings(r#"{"request_correlation_in_logs":true}"#),
    r#"{"trace_id":"inner-trace","session_id":"inner-session"}/{"trace_id":"outer-trace","session_id":"outer-session"}/{"trace_id":"inner-trace","session_id":"inner-session"}"#
)]
#[case::an_explicit_false_support_flag_is_kept(
    Scenario::OmittedSettings(r#"{"supports_correlation_logging":false,"request_correlation_in_logs":true}"#),
    r#"{"trace_id":"outer-trace","session_id":"outer-session"}/{"trace_id":"outer-trace","session_id":"outer-session"}/{"trace_id":"outer-trace","session_id":"outer-session"}"#
)]
#[case::a_secret_shaped_trace_id_is_redacted_when_redaction_is_on(
    Scenario::Correlation(CorrelationSettings {
        supports_correlation_logging: true,
        request_correlation_in_logs: true,
        secret_redaction: SecretRedaction::Enabled,
    }, "sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r"), r#"{"trace_id":"REDACTED","session_id":"inner-session"}/{"trace_id":"outer-trace","session_id":"outer-session"}/{"trace_id":"REDACTED","session_id":"inner-session"}"#)]
#[case::a_secret_shaped_trace_id_is_kept_but_still_stripped_when_redaction_is_off(
    Scenario::Correlation(CorrelationSettings {
        supports_correlation_logging: true,
        request_correlation_in_logs: true,
        secret_redaction: SecretRedaction::Disabled,
    }, "sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r"), r#"{"trace_id":"sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","session_id":"inner-session"}/{"trace_id":"outer-trace","session_id":"outer-session"}/{"trace_id":"sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","session_id":"inner-session"}"#)]
#[case::message_normalization_is_timed_by_the_injected_clock(Scenario::CopyTiming, "250/0")]
fn logging_state_behavior_tests_behavior(#[case] scenario: Scenario, #[case] expected: &str) {
    assert_eq!(outcome(&scenario), expected);
}
