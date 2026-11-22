#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_prompts::arguments::json::Object;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use super::error::PerplexityResponsesError;
use super::{
    PROVIDER, SUPPORTED_OPENAI_PARAMS, SUPPORTS_NATIVE_WEBSOCKET, get_complete_url,
    transform_request, transform_response, validate_environment, with_message_types,
};
use crate::openai_base_config::Headers;
use crate::response_events::{RawResponse, ReplyBody};
use crate::response_input::ResponsesInput;

const INIT: &str = r#"{"model":"sonar","call_type":"responses","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const COMPLETED: &str = r#"{"id":"resp_1","object":"response","created_at":1700000000,"status":"completed","model":"sonar","output":[]}"#;

struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

fn input(text: &str) -> ResponsesInput {
    serde_json::from_str(text).unwrap()
}

fn body_text(body: &Object) -> String {
    serde_json::to_string(body).unwrap()
}

fn with_state<T, A>(action: A) -> T
where
    A: FnOnce(&mut LoggingState, &PrePostEnv<'_>) -> T,
{
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).unwrap(),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("trace"),
        Duration::default,
    )
    .unwrap();
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor: &redactor,
        log: &Silent,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:05:00+00:00").unwrap(),
    };
    action(&mut state, &env)
}

fn decode(text: &str, status: u16) -> Result<ReplyBody, PerplexityResponsesError> {
    with_state(|state, env| {
        let headers = IndexMap::new();
        let reply = RawResponse {
            status,
            headers: &headers,
            text,
        };
        transform_response(&reply, 1_700_000_000, state, env).map(|decoded| decoded.response)
    })
}

fn failure(text: &str, status: u16) -> (u16, String) {
    let Err(PerplexityResponsesError::Failed { error }) = decode(text, status) else {
        panic!("a failed reply raises");
    };
    (error.status_code, error.message)
}

#[test]
fn the_config_names_perplexity_and_no_websocket() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        [
            "max_output_tokens",
            "stream",
            "temperature",
            "top_p",
            "tools",
            "reasoning",
            "instructions",
            "models",
        ]
    );
    assert_eq!(PROVIDER, ProviderId::Perplexity);
    const { assert!(!SUPPORTS_NATIVE_WEBSOCKET) };
}

#[rstest]
#[case::explicit_key(Some("sk-explicit"), &[("PERPLEXITYAI_API_KEY", "sk-env")], Some("Bearer sk-explicit"))]
#[case::empty_key_falls_through(Some(""), &[("PERPLEXITYAI_API_KEY", "sk-env")], Some("Bearer sk-env"))]
#[case::second_variable(None, &[("PERPLEXITY_API_KEY", "sk-second")], Some("Bearer sk-second"))]
#[case::no_key(None, &[], None)]
fn the_bearer_is_added_over_the_caller_headers(
    #[case] api_key: Option<&str>,
    #[case] pairs: &[(&str, &str)],
    #[case] authorization: Option<&str>,
) {
    let key = api_key.map(|text| SecretString::from(text.to_owned()));
    let caller = Headers::from([(String::from("X-Trace"), String::from("t1"))]);
    let headers = validate_environment(caller, key.as_ref(), &environment(pairs));
    assert_eq!(headers.get("X-Trace").map(String::as_str), Some("t1"));
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        authorization
    );
    assert!(!headers.contains_key("Content-Type"));
}

#[rstest]
#[case::explicit_base(Some("https://mock.example"), &[("PERPLEXITY_API_BASE", "https://env.example")], "https://mock.example/v1/responses")]
#[case::base_with_v1(Some("https://mock.example/v1"), &[], "https://mock.example/v1/v1/responses")]
#[case::trailing_slashes(Some("https://mock.example///"), &[], "https://mock.example/v1/responses")]
#[case::empty_base_reads_the_variable(Some(""), &[("PERPLEXITY_API_BASE", "https://env.example/")], "https://env.example/v1/responses")]
#[case::default_base(None, &[], "https://api.perplexity.ai/v1/responses")]
fn the_url_appends_v1_responses_to_the_trimmed_base(
    #[case] api_base: Option<&str>,
    #[case] pairs: &[(&str, &str)],
    #[case] url: &str,
) {
    assert_eq!(get_complete_url(api_base, &environment(pairs)), url);
}

#[test]
fn objects_without_a_type_become_messages_and_the_rest_stays() {
    let sent = input(
        r#"[{"role":"user","content":"hi"},{"type":"input_text","text":"x"},"plain",3,{"type":null}]"#,
    );
    assert_eq!(
        with_message_types(&sent),
        input(
            r#"[{"role":"user","content":"hi","type":"message"},{"type":"input_text","text":"x"},"plain",3,{"type":null}]"#
        )
    );
    assert_eq!(
        with_message_types(&input(r#""hello""#)),
        input(r#""hello""#)
    );
}

#[test]
fn a_preset_model_sends_the_preset_name_and_no_model() {
    let body = transform_request(
        "preset/pro-search",
        &input(r#"[{"role":"user","content":"hi"}]"#),
        &object(r#"{"max_output_tokens": 50}"#),
        None,
    )
    .unwrap();
    assert_eq!(
        body_text(&body),
        r#"{"preset":"pro-search","input":[{"role":"user","content":"hi","type":"message"}],"max_output_tokens":50}"#
    );
}

#[test]
fn a_preset_body_lets_an_optional_input_replace_the_repaired_one() {
    let body = transform_request(
        "preset/fast-search",
        &input(r#""question""#),
        &object(r#"{"input": "replaced", "preset": "other"}"#),
        None,
    )
    .unwrap();
    assert_eq!(body_text(&body), r#"{"preset":"other","input":"replaced"}"#);
}

#[test]
fn a_preset_body_validates_reasoning_items() {
    let body = transform_request(
        "preset/pro-search",
        &input(r#"[{"type":"reasoning","id":"r1","summary":[],"status":null}]"#),
        &Object::new(),
        None,
    )
    .unwrap();
    assert_eq!(
        body_text(&body),
        r#"{"preset":"pro-search","input":[{"id":"r1","summary":[],"type":"reasoning"}]}"#
    );
}

#[test]
fn an_ordinary_model_goes_through_the_openai_transform_with_repaired_input() {
    let body = transform_request(
        "sonar",
        &input(r#"[{"role":"user","content":"hi"}]"#),
        &object(r#"{"temperature": 0.2}"#),
        None,
    )
    .unwrap();
    assert_eq!(
        body,
        object(
            r#"{"temperature":0.2,"model":"sonar","input":[{"role":"user","content":"hi","type":"message"}]}"#
        )
    );
    assert!(!body.contains_key("preset"));
}

#[test]
fn an_ordinary_model_refuses_an_optional_model_key() {
    let refused = transform_request(
        "sonar",
        &input(r#""hi""#),
        &object(r#"{"model":"x"}"#),
        None,
    );
    assert!(matches!(refused, Err(PerplexityResponsesError::Input(_))));
}

#[test]
fn a_failed_status_on_200_raises_the_provider_message() {
    let text = r#"{"status":"failed","error":{"message":"quota exceeded","code":"q"}}"#;
    assert_eq!(failure(text, 200), (200, String::from("quota exceeded")));
}

#[rstest]
#[case::no_error(r#"{"status":"failed"}"#)]
#[case::error_not_object(r#"{"status":"failed","error":"boom"}"#)]
#[case::error_without_message(r#"{"status":"failed","error":{"code":"x"}}"#)]
#[case::null_error(r#"{"status":"failed","error":null}"#)]
fn a_failed_status_without_a_message_raises_the_unknown_text(#[case] text: &str) {
    assert_eq!(
        failure(text, 202),
        (202, String::from("Unknown Perplexity error"))
    );
}

#[test]
fn a_completed_reply_goes_to_the_openai_decode() {
    let ReplyBody::Typed(response) = decode(COMPLETED, 200).unwrap() else {
        panic!("a typed reply");
    };
    assert_eq!(response.id, "resp_1");
}

#[rstest]
#[case::not_json("upstream down")]
#[case::json_list(r#"[{"status":"failed"}]"#)]
fn a_reply_that_is_no_object_goes_on_to_the_openai_error(#[case] text: &str) {
    assert!(matches!(
        decode(text, 200),
        Err(PerplexityResponsesError::Reply(_))
    ));
}
