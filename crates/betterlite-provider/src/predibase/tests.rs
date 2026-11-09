#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_openai::error::ProviderError;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::{Hub, builtin_entries};
use chrono::{DateTime, Utc};
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;

use super::config::{
    PredibaseConfig, call_error, get_complete_url, get_supported_openai_params, map_openai_params,
    validate_environment,
};
use super::decode::{DecodedReply, decode_predibase_reply, output_parser};
use super::error::{PredibaseError, PredibaseSetupError};
use super::reply::{
    PredibaseResponse, PredibaseUsage, ReplyInput, TokenCounter, transform_response,
};
use super::request::{CustomPrompt, RequestBody, RequestContext};

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
#[case::stream_argument(
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
#[case::streaming_ok(Some(200), true, None)]
#[case::streaming_error(Some(502), true, Some((502, "boom")))]
#[case::streaming_transport_failure(None, true, None)]
#[case::non_streaming_ok(Some(200), false, None)]
#[case::non_streaming_error(Some(502), false, Some((502, "HTTPStatusError - received status_code=502, error_message=boom")))]
#[case::non_streaming_transport_failure(None, false, Some((500, "boom")))]
fn call_errors_follow_the_streaming_and_plain_shapes(
    #[case] status: Option<u16>,
    #[case] streaming: bool,
    #[case] expected: Option<(u16, &str)>,
) {
    let error = call_error(status, "boom", streaming);
    assert_eq!(
        error.map(|found| (found.status_code, found.message)),
        expected.map(|(code, message)| (code, String::from(message)))
    );
}

fn reply_fixture(body: &str) -> Fixture {
    Fixture {
        request: RequestBody {
            inputs: None,
            parameters: Object::new(),
        },
        headers: BTreeMap::new(),
        options: Object::new(),
        messages: Vec::new(),
        body: body.as_bytes().to_vec(),
        status: 200,
        model: String::from("model"),
        created: 1_700_000_000,
    }
}

struct Fixture {
    request: RequestBody,
    headers: BTreeMap<String, String>,
    options: Object,
    messages: Vec<Message>,
    body: Vec<u8>,
    status: u16,
    model: String,
    created: i64,
}

impl Fixture {
    fn input(&self) -> ReplyInput<'_> {
        ReplyInput {
            body: &self.body,
            status: self.status,
            headers: &self.headers,
            request_body: &self.request,
            options: &self.options,
            messages: &self.messages,
            model: &self.model,
            created: self.created,
        }
    }
}

struct Counting {
    fail_messages: bool,
    fail_text: bool,
}

impl Counting {
    const fn working() -> Self {
        Self {
            fail_messages: false,
            fail_text: false,
        }
    }
}

#[async_trait]
impl TokenCounter for Counting {
    async fn count_messages(&self, messages: &[Message]) -> Option<i64> {
        (!self.fail_messages).then(|| i64::try_from(messages.len()).unwrap_or(i64::MAX))
    }

    async fn count_text(&self, text: &str) -> Option<i64> {
        (!self.fail_text).then(|| i64::try_from(text.chars().count()).unwrap_or(i64::MAX))
    }
}

#[tokio::test]
async fn a_reply_gives_content_finish_reason_created_and_model() {
    let fixture = reply_fixture(
        r#"{"generated_text":"<|assistant|>hi","details":{"finish_reason":"length","tokens":[{"logprob":-0.5},{"logprob":null},{"logprob":-0.25}]}}"#,
    );
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(response.model, "model");
    assert_eq!(response.created, 1_700_000_000);
    assert_eq!(response.choices.len(), 1_usize);
    let choice = &response.choices[0];
    assert_eq!(choice.index, 0);
    assert_eq!(choice.content.as_deref(), Some("hi"));
    assert_eq!(choice.finish_reason, "length");
    assert_eq!(
        choice.private_logprob,
        Some(Number::from_f64(-0.75).unwrap())
    );
    assert_eq!(choice.logprobs, None);
}

#[tokio::test]
async fn an_empty_generated_text_leaves_the_content_null() {
    let fixture = reply_fixture(r#"{"generated_text":""}"#);
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(response.choices[0].content, None);
    assert_eq!(response.choices[0].finish_reason, "stop");
}

#[tokio::test]
async fn the_finish_reason_maps_through_litellms_table() {
    let fixture = reply_fixture(
        r#"{"generated_text":"x","details":{"finish_reason":"content_filtered","tokens":[]}}"#,
    );
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(response.choices[0].finish_reason, "content_filter");
    assert_eq!(
        response.choices[0].private_logprob,
        Some(Number::from(0_i64))
    );
}

#[tokio::test]
async fn an_unmapped_finish_reason_becomes_stop() {
    let fixture =
        reply_fixture(r#"{"generated_text":"x","details":{"finish_reason":"weird","tokens":[]}}"#);
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(response.choices[0].finish_reason, "stop");
}

#[tokio::test]
async fn a_null_finish_reason_becomes_stop() {
    let fixture =
        reply_fixture(r#"{"generated_text":"x","details":{"finish_reason":null,"tokens":[]}}"#);
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(response.choices[0].finish_reason, "stop");
}

#[tokio::test]
async fn details_without_tokens_leave_the_finish_reason_alone() {
    let fixture = reply_fixture(r#"{"generated_text":"x","details":{"finish_reason":"length"}}"#);
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(response.choices[0].finish_reason, "stop");
    assert_eq!(response.choices[0].private_logprob, None);
}

#[tokio::test]
async fn tokens_without_a_finish_reason_fail_with_a_server_error() {
    let fixture = reply_fixture(r#"{"generated_text":"x","details":{"tokens":[]}}"#);
    let error = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap_err();
    assert_eq!(error.status_code, 500);
    assert!(error.message.contains("finish_reason"));
}

#[tokio::test]
async fn best_of_sequences_become_extra_choices() {
    let mut fixture = reply_fixture(
        r#"{"generated_text":"a","details":{"finish_reason":"stop","tokens":[],"best_of_sequences":[{"generated_text":"<|user|>b","finish_reason":"length","tokens":[{"logprob":-0.5},{"logprob":null}]},{"generated_text":"","finish_reason":"stop","tokens":[{"logprob":-0.25}]},{"generated_text":"c","finish_reason":"weird","tokens":[]}]}}"#,
    );
    fixture.options = params(r#"{"best_of":2}"#);
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(response.choices.len(), 4_usize);
    assert_eq!(response.choices[1].index, 1);
    assert_eq!(response.choices[1].content.as_deref(), Some("b"));
    assert_eq!(response.choices[1].finish_reason, "length");
    assert_eq!(
        response.choices[1].logprobs,
        Some(Number::from_f64(-0.5).unwrap())
    );
    assert_eq!(response.choices[1].private_logprob, None);
    assert_eq!(response.choices[2].index, 2);
    assert_eq!(response.choices[2].content, None);
    assert_eq!(response.choices[2].logprobs, None);
    assert_eq!(response.choices[3].index, 3);
    assert_eq!(response.choices[3].content.as_deref(), Some("c"));
    assert_eq!(response.choices[3].finish_reason, "stop");
    assert_eq!(response.choices[3].logprobs, Some(Number::from(0_i64)));
}

#[tokio::test]
async fn best_of_reads_the_request_parameters_when_the_options_have_none() {
    let mut fixture = reply_fixture(
        r#"{"generated_text":"a","details":{"finish_reason":"stop","tokens":[],"best_of_sequences":[{"generated_text":"b","finish_reason":"stop","tokens":[]}]}}"#,
    );
    fixture.request.parameters = params(r#"{"best_of":2}"#);
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(response.choices.len(), 2_usize);
}

#[tokio::test]
async fn a_null_best_of_falls_back_to_the_request_parameters() {
    let mut fixture = reply_fixture(
        r#"{"generated_text":"a","details":{"finish_reason":"stop","tokens":[],"best_of_sequences":[{"generated_text":"b","finish_reason":"stop","tokens":[]}]}}"#,
    );
    fixture.options = params(r#"{"best_of":null}"#);
    fixture.request.parameters = params(r#"{"best_of":2}"#);
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(response.choices.len(), 2_usize);
}

#[tokio::test]
async fn the_options_best_of_wins_over_the_request_parameters() {
    let mut fixture = reply_fixture(
        r#"{"generated_text":"a","details":{"finish_reason":"stop","tokens":[],"best_of_sequences":[{"generated_text":"b","finish_reason":"stop","tokens":[]}]}}"#,
    );
    fixture.options = params(r#"{"best_of":1}"#);
    fixture.request.parameters = params(r#"{"best_of":2}"#);
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(response.choices.len(), 1_usize);
}

#[rstest]
#[case::string(r#"{"best_of":"2"}"#)]
#[case::float(r#"{"best_of":2.5}"#)]
#[case::boolean(r#"{"best_of":true}"#)]
#[case::null(r#"{"best_of":null}"#)]
#[tokio::test]
async fn a_best_of_that_is_not_an_integer_adds_no_choices(#[case] options: &str) {
    let mut fixture = reply_fixture(
        r#"{"generated_text":"a","details":{"finish_reason":"stop","tokens":[],"best_of_sequences":[{"generated_text":"b","finish_reason":"stop","tokens":[]}]}}"#,
    );
    fixture.options = params(options);
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(response.choices.len(), 1_usize);
}

#[tokio::test]
async fn a_reply_with_sequences_alone_adds_no_choices() {
    let fixture = reply_fixture(
        r#"{"generated_text":"a","details":{"finish_reason":"stop","tokens":[],"best_of_sequences":[{"generated_text":"b","finish_reason":"stop","tokens":[]}]}}"#,
    );
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(response.choices.len(), 1_usize);
}

#[tokio::test]
async fn usage_counts_the_messages_and_the_parsed_text() {
    let mut fixture = reply_fixture(r#"{"generated_text":"<|assistant|>hi"}"#);
    fixture.messages = messages(r#"[{"role":"user","content":"hello"}]"#);
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(
        response.usage,
        PredibaseUsage {
            prompt_tokens: 1,
            completion_tokens: 2,
            total_tokens: 3
        }
    );
}

#[tokio::test]
async fn usage_is_zero_when_the_text_counter_fails() {
    let mut fixture = reply_fixture(r#"{"generated_text":"hi"}"#);
    fixture.messages = messages(r#"[{"role":"user","content":"hello"}]"#);
    let failing_text = Counting {
        fail_messages: false,
        fail_text: true,
    };
    let response = transform_response(&fixture.input(), &failing_text)
        .await
        .unwrap();
    assert_eq!(
        response.usage,
        PredibaseUsage {
            prompt_tokens: 1,
            completion_tokens: 0,
            total_tokens: 1
        }
    );
}

#[tokio::test]
async fn usage_is_zero_when_the_message_counter_fails() {
    let mut fixture = reply_fixture(r#"{"generated_text":"hi"}"#);
    fixture.messages = messages(r#"[{"role":"user","content":"hello"}]"#);
    let failing_messages = Counting {
        fail_messages: true,
        fail_text: false,
    };
    let response = transform_response(&fixture.input(), &failing_messages)
        .await
        .unwrap();
    assert_eq!(
        response.usage,
        PredibaseUsage {
            prompt_tokens: 0,
            completion_tokens: 2,
            total_tokens: 2
        }
    );
}

#[tokio::test]
async fn usage_is_zero_for_empty_content() {
    let empty = reply_fixture(r#"{"generated_text":""}"#);
    let response = transform_response(&empty.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(
        response.usage,
        PredibaseUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0
        }
    );
}

#[tokio::test]
async fn x_headers_are_copied_with_the_provider_prefix() {
    let mut fixture = reply_fixture(r#"{"generated_text":"a"}"#);
    fixture.headers = BTreeMap::from([
        (String::from("X-Upper"), String::from("u")),
        (String::from("foo"), String::from("f")),
        (String::from("x-request-id"), String::from("r")),
    ]);
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(
        response.additional_headers,
        BTreeMap::from([
            (String::from("llm_provider-x-request-id"), String::from("r")),
            (String::from("llm_provider-x-upper"), String::from("u")),
        ])
    );
}

#[tokio::test]
async fn an_error_key_raises_the_response_status() {
    let mut fixture = reply_fixture(r#"{"error":"boom"}"#);
    fixture.status = 200;
    let error = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap_err();
    assert_eq!(error.status_code, 200);
    assert_eq!(error.message, "boom");
    fixture.status = 502;
    let failed = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap_err();
    assert_eq!(failed.status_code, 502);
}

#[tokio::test]
async fn an_error_object_keeps_its_json_text() {
    let fixture = reply_fixture(r#"{"error":{"message":"boom"}}"#);
    let error = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap_err();
    assert_eq!(error.status_code, 200);
    assert!(error.message.contains("boom"));
}

#[tokio::test]
async fn a_null_error_is_still_an_error() {
    let fixture = reply_fixture(r#"{"error":null}"#);
    let error = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap_err();
    assert_eq!(error.status_code, 200);
}

#[rstest]
#[case::not_json("not json", 422)]
#[case::array("[1,2]", 422)]
#[case::string("\"hi\"", 422)]
#[case::missing_generated_text(r#"{"foo":1}"#, 422)]
#[case::empty_object("{}", 422)]
#[case::null_generated_text(r#"{"generated_text":null}"#, 500)]
#[case::number_generated_text(r#"{"generated_text":5}"#, 500)]
#[case::array_generated_text(r#"{"generated_text":[]}"#, 500)]
#[case::wrong_details(r#"{"generated_text":"x","details":[]}"#, 500)]
#[case::wrong_tokens(r#"{"generated_text":"x","details":{"tokens":5}}"#, 500)]
#[case::token_without_logprob(
    r#"{"generated_text":"x","details":{"finish_reason":"stop","tokens":[{}]}}"#,
    500
)]
#[case::string_logprob(
    r#"{"generated_text":"x","details":{"finish_reason":"stop","tokens":[{"logprob":"x"}]}}"#,
    500
)]
#[case::sequence_without_tokens(r#"{"generated_text":"x","details":{"finish_reason":"stop","best_of_sequences":[{"generated_text":"b","finish_reason":"stop"}]}}"#, 500)]
#[tokio::test]
async fn bad_replies_give_a_predibase_error(#[case] body: &str, #[case] status: u16) {
    let fixture = reply_fixture(body);
    let error = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap_err();
    assert_eq!(error.status_code, status);
}

#[test]
fn the_reply_decoder_is_typed() {
    let decoded = decode_predibase_reply(
        br#"{"generated_text":"hi","details":{"finish_reason":"stop","tokens":[{"logprob":1}]}}"#,
    )
    .unwrap();
    let DecodedReply::Text(text) = decoded else {
        panic!("a text reply");
    };
    assert_eq!(text.generated_text, "hi");
    let details = text.details.unwrap();
    assert_eq!(details.finish_reason, Some(Some(String::from("stop"))));
    assert_eq!(details.tokens.map(|tokens| tokens.len()), Some(1_usize));
}

#[test]
fn a_body_that_is_not_utf8_names_its_size() {
    let error = decode_predibase_reply(&[0xff_u8, 0xfe]).unwrap_err();
    assert_eq!(error.status_code, 422);
    assert!(error.message.contains("2 bytes that are not UTF-8"));
}

#[test]
fn a_malformed_body_keeps_its_text() {
    let body = "not json";
    let error = decode_predibase_reply(body.as_bytes()).unwrap_err();
    assert_eq!(error.status_code, 422);
    assert_eq!(error.message, body);
}

#[test]
fn a_missing_generated_text_names_the_body() {
    let body = r#"{"foo":1}"#;
    let error = decode_predibase_reply(body.as_bytes()).unwrap_err();
    assert_eq!(error.status_code, 422);
    assert!(error.message.contains(body));
}

#[rstest]
#[case::plain("hello", "hello")]
#[case::both_ends("<|assistant|>hi<|assistant|>", "hi")]
#[case::leading_space_kept("  <|assistant|>hi", "  hi")]
#[case::bos_eos("<s>hello</s>", "hello")]
#[case::order_matters("<s><|user|>hi</s>", "<|user|>hi")]
#[case::end_only("abc<s>", "abc")]
#[case::trailing_space_kept("abc </s>", "abc ")]
#[case::system_then_user("<|system|> <|user|>x", " x")]
#[case::empty("", "")]
#[case::blank("   ", "   ")]
#[case::one_of_each("<s><s>x</s></s>", "<s>x</s>")]
#[case::end_of_another_token("<|assistant|>x<s>", "x")]
fn output_parser_strips_template_tokens_in_order(#[case] given: &str, #[case] expected: &str) {
    assert_eq!(output_parser(given), expected);
}

#[test]
fn an_empty_error_key_value_is_reported_as_null() {
    let decoded = decode_predibase_reply(br#"{"error":null,"generated_text":"x"}"#).unwrap();
    assert_eq!(decoded, DecodedReply::Error(String::from("null")));
}

#[test]
fn an_error_key_wins_over_the_generated_text() {
    let decoded = decode_predibase_reply(br#"{"error":"boom","generated_text":"x"}"#).unwrap();
    assert_eq!(decoded, DecodedReply::Error(String::from("boom")));
}

#[test]
fn replays_the_predibase_reply_corpus() {
    let seen = fs::read_dir("fuzz/corpus/predibase_reply")
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .inspect(|path| {
            let bytes = fs::read(path).unwrap();
            let _replayed = decode_predibase_reply(&bytes);
        })
        .count();
    assert!(seen > 0_usize);
}

#[test]
fn a_predibase_error_converts_to_the_gateway_error() {
    let error = PredibaseError {
        status_code: 418,
        message: String::from("boom"),
        headers: BTreeMap::from([(String::from("x"), String::from("1"))]),
    };
    let converted = ProviderError::from(error);
    assert_eq!(converted.status_code, 418);
    assert_eq!(converted.message, "boom");
    assert_eq!(
        converted.headers,
        Some(vec![(String::from("x"), String::from("1"))])
    );
}

#[test]
fn a_predibase_response_is_debuggable() {
    let response = PredibaseResponse {
        model: String::from("m"),
        created: 1,
        choices: Vec::new(),
        usage: PredibaseUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
        },
        additional_headers: BTreeMap::new(),
    };
    assert!(!format!("{response:?}").is_empty());
}
