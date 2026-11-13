#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::{Hub, builtin_entries};
use chrono::{DateTime, Utc};
use rstest::rstest;
use secrecy::SecretString;

use super::config::{
    CallFailure, PredibaseConfig, call_error, get_complete_url, get_supported_openai_params,
    map_openai_params, validate_environment,
};
use super::error::PredibaseSetupError;
use super::request::{CustomPrompt, RequestContext};

mod reply;

fn params(wire: &str) -> Object {
    serde_json::from_str(wire).unwrap()
}

fn integer(value: &Json) -> Option<i64> {
    match value {
        Json::Number(number) => number.as_i64(),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn messages(wire: &str) -> Vec<Message> {
    serde_json::from_str(wire).unwrap()
}

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

fn factory() -> PromptFactory {
    PromptFactory {
        cache: Mutex::new(builtin_entries()),
        hub: Hub::Unreachable,
    }
}

fn config() -> PredibaseConfig {
    PredibaseConfig {
        details: true,
        max_new_tokens: 4096,
        return_full_text: false,
    }
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

#[test]
fn supported_openai_params_are_litellm_order() {
    assert_eq!(
        get_supported_openai_params(),
        [
            "stream",
            "temperature",
            "max_completion_tokens",
            "max_tokens",
            "top_p",
            "stop",
            "n",
            "response_format"
        ]
    );
}

#[rstest]
#[case::temperature_zero(r#"{"temperature":0}"#, r#"{"temperature":0.01}"#)]
#[case::temperature_zero_float(r#"{"temperature":0.0}"#, r#"{"temperature":0.01}"#)]
#[case::temperature_kept(r#"{"temperature":0.5}"#, r#"{"temperature":0.5}"#)]
#[case::top_p(r#"{"top_p":0.9}"#, r#"{"top_p":0.9}"#)]
#[case::stream(r#"{"stream":true}"#, r#"{"stream":true}"#)]
#[case::stop(r#"{"stop":["x"]}"#, r#"{"stop":["x"]}"#)]
#[case::response_format(
    r#"{"response_format":{"type":"json_object"}}"#,
    r#"{"response_format":{"type":"json_object"}}"#
)]
#[case::max_tokens(r#"{"max_tokens":5}"#, r#"{"max_new_tokens":5}"#)]
#[case::max_tokens_zero(r#"{"max_tokens":0}"#, r#"{"max_new_tokens":1}"#)]
#[case::max_completion_tokens_zero(r#"{"max_completion_tokens":0}"#, r#"{"max_new_tokens":1}"#)]
#[case::n(r#"{"n":3}"#, r#"{"best_of":3,"do_sample":true}"#)]
#[case::echo(r#"{"echo":true}"#, r#"{"decoder_input_details":true}"#)]
#[case::echo_false_still_sets_details(r#"{"echo":false}"#, r#"{"decoder_input_details":true}"#)]
#[case::unmapped_is_ignored(r#"{"best_of":2,"other":1}"#, "{}")]
fn openai_params_map_into_the_options(#[case] wire: &str, #[case] expected: &str) {
    let mut options = Object::new();
    map_openai_params(&params(wire), &mut options);
    assert_eq!(options, params(expected));
}

#[test]
fn openai_params_keep_the_existing_options() {
    let mut options = params(r#"{"temperature":0.5}"#);
    map_openai_params(&params(r#"{"top_p":0.9}"#), &mut options);
    assert_eq!(options, params(r#"{"temperature":0.5,"top_p":0.9}"#));
}

#[rstest]
#[case::max_tokens_last(r#"{"max_completion_tokens":7,"max_tokens":5}"#, 5)]
#[case::max_completion_tokens_last(r#"{"max_tokens":5,"max_completion_tokens":7}"#, 7)]
fn the_later_max_limit_wins(#[case] wire: &str, #[case] expected: i64) {
    let mut options = Object::new();
    map_openai_params(&params(wire), &mut options);
    let found = options.get("max_new_tokens").and_then(integer);
    assert_eq!(found, Some(expected));
}

#[test]
fn get_config_is_the_non_null_class_defaults() {
    let config = PredibaseConfig {
        details: true,
        max_new_tokens: 123,
        return_full_text: false,
    };
    assert_eq!(
        config.get_config(),
        params(r#"{"details":true,"max_new_tokens":123,"return_full_text":false}"#)
    );
}

#[tokio::test]
async fn request_body_carries_the_config_defaults() {
    let config = config();
    let prompts = factory();
    let custom = BTreeMap::new();
    let context = RequestContext {
        config: &config,
        prompts: &prompts,
        custom_prompts: &custom,
        now: now(),
    };
    let body = context
        .transform_request(
            "meta-llama/llama-2-7b-chat",
            &messages(r#"[{"role":"user","content":"hi"}]"#),
            &params("{}"),
        )
        .await
        .unwrap();
    assert_eq!(body.inputs.as_deref(), Some("<s>[INST] hi [/INST]\n"));
    assert_eq!(
        body.parameters,
        params(r#"{"details":true,"max_new_tokens":4096,"return_full_text":false}"#)
    );
}

#[tokio::test]
async fn request_body_keeps_the_callers_false_and_zero() {
    let config = config();
    let prompts = factory();
    let custom = BTreeMap::new();
    let context = RequestContext {
        config: &config,
        prompts: &prompts,
        custom_prompts: &custom,
        now: now(),
    };
    let body = context
        .transform_request(
            "meta-llama/llama-2-7b-chat",
            &messages(r#"[{"role":"user","content":"hi"}]"#),
            &params(r#"{"details":false,"max_new_tokens":0,"return_full_text":false}"#),
        )
        .await
        .unwrap();
    assert_eq!(
        body.parameters,
        params(r#"{"details":false,"max_new_tokens":0,"return_full_text":false}"#)
    );
}

#[tokio::test]
async fn request_body_pops_stream_but_keeps_the_mapped_options() {
    let config = config();
    let prompts = factory();
    let custom = BTreeMap::new();
    let context = RequestContext {
        config: &config,
        prompts: &prompts,
        custom_prompts: &custom,
        now: now(),
    };
    let mut options = Object::new();
    map_openai_params(&params(r#"{"stream":true,"n":2}"#), &mut options);
    let body = context
        .transform_request(
            "meta-llama/llama-2-7b-chat",
            &messages(r#"[{"role":"user","content":"hi"}]"#),
            &options,
        )
        .await
        .unwrap();
    assert_eq!(
        body.parameters,
        params(
            r#"{"best_of":2,"do_sample":true,"details":true,"max_new_tokens":4096,"return_full_text":false}"#
        )
    );
}

#[tokio::test]
async fn request_body_without_a_prompt_factory_match_has_a_null_input() {
    let config = config();
    let prompts = factory();
    let custom = BTreeMap::new();
    let context = RequestContext {
        config: &config,
        prompts: &prompts,
        custom_prompts: &custom,
        now: now(),
    };
    let body = context
        .transform_request(
            "tiiuae/falcon-7b",
            &messages(r#"[{"role":"user","content":"hi"}]"#),
            &params("{}"),
        )
        .await
        .unwrap();
    assert_eq!(body.inputs, None);
    let wire = serde_json::to_string(&body).unwrap();
    assert!(wire.contains(r#""inputs":null"#));
}

fn custom_prompt_dict(wire: &str) -> BTreeMap<String, CustomPrompt> {
    serde_json::from_str(wire).unwrap()
}

#[tokio::test]
async fn a_custom_prompt_dictionary_entry_builds_the_prompt() {
    let config = config();
    let prompts = factory();
    let custom = custom_prompt_dict(
        r#"{"m":{"roles":{"user":{"pre_message":"U:","post_message":"!"}},"initial_prompt_value":"I","final_prompt_value":"F"}}"#,
    );
    let context = RequestContext {
        config: &config,
        prompts: &prompts,
        custom_prompts: &custom,
        now: now(),
    };
    let body = context
        .transform_request(
            "m",
            &messages(r#"[{"role":"user","content":"hi"}]"#),
            &params("{}"),
        )
        .await
        .unwrap();
    assert_eq!(body.inputs.as_deref(), Some("IU:hi!F"));
}

#[tokio::test]
async fn a_custom_prompt_for_another_model_uses_the_prompt_factory() {
    let config = config();
    let prompts = factory();
    let custom = custom_prompt_dict(
        r#"{"other":{"roles":{},"initial_prompt_value":"I","final_prompt_value":"F"}}"#,
    );
    let context = RequestContext {
        config: &config,
        prompts: &prompts,
        custom_prompts: &custom,
        now: now(),
    };
    let body = context
        .transform_request(
            "meta-llama/llama-2-7b-chat",
            &messages(r#"[{"role":"user","content":"hi"}]"#),
            &params("{}"),
        )
        .await
        .unwrap();
    assert_eq!(body.inputs.as_deref(), Some("<s>[INST] hi [/INST]\n"));
}

#[rstest]
#[case::missing_roles(
    r#"{"m":{"initial_prompt_value":"I","final_prompt_value":"F"}}"#,
    "roles"
)]
#[case::missing_initial(
    r#"{"m":{"roles":{},"final_prompt_value":"F"}}"#,
    "initial_prompt_value"
)]
#[case::missing_final(
    r#"{"m":{"roles":{},"initial_prompt_value":"I"}}"#,
    "final_prompt_value"
)]
#[tokio::test]
async fn a_custom_prompt_entry_missing_a_field_fails(#[case] wire: &str, #[case] field: &str) {
    let config = config();
    let prompts = factory();
    let custom = custom_prompt_dict(wire);
    let context = RequestContext {
        config: &config,
        prompts: &prompts,
        custom_prompts: &custom,
        now: now(),
    };
    let error = context
        .transform_request(
            "m",
            &messages(r#"[{"role":"user","content":"hi"}]"#),
            &params("{}"),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        PredibaseSetupError::CustomPromptField { model, field: found } if model == "m" && found == field
    ));
}

#[rstest]
#[case::default_base(
    None,
    None,
    "https://serving.app.predibase.com/t/deployments/v2/llms/m/generate"
)]
#[case::empty_base(
    Some(""),
    None,
    "https://serving.app.predibase.com/t/deployments/v2/llms/m/generate"
)]
#[case::explicit_base(
    Some("https://host"),
    None,
    "https://host/t/deployments/v2/llms/m/generate"
)]
#[case::trailing_slash_is_kept(
    Some("https://host/"),
    None,
    "https://host//t/deployments/v2/llms/m/generate"
)]
#[case::model_with_slash(
    Some("https://host"),
    Some("org/model"),
    "https://host/t/deployments/v2/llms/org/model/generate"
)]
fn complete_urls_follow_the_base_and_model(
    #[case] api_base: Option<&str>,
    #[case] model: Option<&str>,
    #[case] expected: &str,
) {
    let url = get_complete_url(
        Some("t"),
        api_base,
        model.unwrap_or("m"),
        &Object::new(),
        None,
    )
    .unwrap();
    assert_eq!(url, expected);
}

#[rstest]
#[case::stream_argument_true(Some(true), r#"{"stream":false}"#, true)]
#[case::stream_argument_false(Some(false), r#"{"stream":true}"#, false)]
#[case::options_true(None, r#"{"stream":true}"#, true)]
#[case::options_false(None, r#"{"stream":false}"#, false)]
#[case::options_integer_one(None, r#"{"stream":1}"#, false)]
#[case::options_string_true(None, r#"{"stream":"true"}"#, false)]
#[case::options_absent(None, "{}", false)]
fn the_stream_suffix_follows_the_argument_then_the_options(
    #[case] stream: Option<bool>,
    #[case] options: &str,
    #[case] streams: bool,
) {
    let url = get_complete_url(
        Some("t"),
        Some("https://host"),
        "m",
        &params(options),
        stream,
    )
    .unwrap();
    let suffix = if streams {
        "generate_stream"
    } else {
        "generate"
    };
    assert_eq!(
        url,
        format!("https://host/t/deployments/v2/llms/m/{suffix}")
    );
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn complete_urls_require_a_tenant(#[case] tenant: Option<&str>) {
    let error =
        get_complete_url(tenant, Some("https://host"), "m", &Object::new(), None).unwrap_err();
    assert!(matches!(error, PredibaseSetupError::MissingTenantId));
    assert_eq!(
        error.to_string(),
        "Missing Predibase Tenant ID - Required for making the request. Set dynamically (e.g. `completion(..tenant_id=<MY-ID>)`) or in env - `PREDIBASE_TENANT_ID`."
    );
}

#[test]
fn validate_environment_adds_the_default_headers() {
    let headers = BTreeMap::new();
    let validated = validate_environment(Some(&secret("sk-x")), &headers).unwrap();
    assert_eq!(
        validated,
        BTreeMap::from([
            (String::from("Authorization"), String::from("Bearer sk-x")),
            (
                String::from("content-type"),
                String::from("application/json")
            ),
        ])
    );
}

#[test]
fn validate_environment_with_an_empty_key_sends_an_empty_bearer() {
    let validated = validate_environment(Some(&secret("")), &BTreeMap::new()).unwrap();
    assert_eq!(
        validated.get("Authorization").map(String::as_str),
        Some("Bearer ")
    );
}

#[test]
fn a_missing_key_fails_with_litellms_text() {
    let error = validate_environment(None, &BTreeMap::new()).unwrap_err();
    assert!(matches!(error, PredibaseSetupError::MissingApiKey));
    assert_eq!(
        error.to_string(),
        "Missing Predibase API Key - A call is being made to predibase but no key is set either in the environment variables or via params"
    );
}

#[test]
fn caller_headers_replace_the_default_of_the_same_spelling_only() {
    let headers = BTreeMap::from([
        (String::from("Authorization"), String::from("Bearer other")),
        (String::from("Content-Type"), String::from("text/plain")),
        (String::from("X"), String::from("1")),
    ]);
    let validated = validate_environment(Some(&secret("sk-x")), &headers).unwrap();
    assert_eq!(
        validated,
        BTreeMap::from([
            (String::from("Authorization"), String::from("Bearer other")),
            (String::from("Content-Type"), String::from("text/plain")),
            (String::from("X"), String::from("1")),
            (
                String::from("content-type"),
                String::from("application/json")
            ),
        ])
    );
}

#[rstest]
#[case::streaming_ok(CallFailure::Response { status: 200, body: "boom" }, true, None)]
#[case::streaming_created(CallFailure::Response { status: 201, body: "boom" }, true, Some((201, "boom")))]
#[case::streaming_error(CallFailure::Response { status: 502, body: "boom" }, true, Some((502, "boom")))]
#[case::streaming_timeout(CallFailure::Rethrown, true, None)]
#[case::streaming_transport_failure(CallFailure::Transport { message: "boom" }, true, None)]
#[case::non_streaming_ok(CallFailure::Response { status: 200, body: "boom" }, false, None)]
#[case::non_streaming_created(CallFailure::Response { status: 201, body: "boom" }, false, None)]
#[case::non_streaming_no_content(CallFailure::Response { status: 204, body: "boom" }, false, None)]
#[case::non_streaming_redirect(
    CallFailure::Response { status: 301, body: "boom" },
    false,
    Some((301, "HTTPStatusError - received status_code=301, error_message=boom"))
)]
#[case::non_streaming_error(
    CallFailure::Response { status: 502, body: "boom" },
    false,
    Some((502, "HTTPStatusError - received status_code=502, error_message=boom"))
)]
#[case::non_streaming_timeout(CallFailure::Rethrown, false, None)]
#[case::non_streaming_transport_failure(
    CallFailure::Transport { message: "boom" },
    false,
    Some((500, "boom"))
)]
fn call_errors_follow_the_streaming_and_plain_shapes(
    #[case] failure: CallFailure<'static>,
    #[case] streaming: bool,
    #[case] expected: Option<(u16, &str)>,
) {
    let error = call_error(failure, streaming);
    assert_eq!(
        error.map(|found| (found.status_code, found.message)),
        expected.map(|(code, message)| (code, String::from(message)))
    );
}
