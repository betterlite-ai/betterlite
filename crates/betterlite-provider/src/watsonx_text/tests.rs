#![cfg(test)]

use std::fs;
use std::sync::Mutex;

use betterlite_openai::watsonx_auth::environment::WatsonxEnvironment;
use betterlite_openai::watsonx_auth::types::WatsonxParams;
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::{Hub, builtin_entries};
use chrono::{DateTime, Utc};
use rstest::rstest;
use secrecy::SecretString;

use super::WatsonxTextConfig;
use super::error::WatsonxTextError;
use super::reply::{decode_text_generation_reply, transform_response};
use super::request::{
    EU_REGIONS, SPECIAL_AUTH_PARAMS, SUPPORTED_OPENAI_PARAMS, US_REGIONS, build_body, complete_url,
    is_watsonx_text_param, map_openai_params, map_special_auth_params,
};
use super::stream::decode_stream_line;

fn params(wire: &str) -> WatsonxParams {
    serde_json::from_str(wire).unwrap()
}

fn messages(wire: &str) -> Vec<Message> {
    serde_json::from_str(wire).unwrap()
}

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

fn environment(values: &[(&str, &str)]) -> WatsonxEnvironment {
    let find = |name: &str| {
        values
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| *value)
    };
    let secret = |name: &str| find(name).map(|value| SecretString::from(value.to_owned()));
    let text = |name: &str| find(name).map(str::to_owned);
    WatsonxEnvironment {
        watsonx_iam_url: text("WATSONX_IAM_URL"),
        wx_api_key: secret("WX_API_KEY"),
        watsonx_api_key: secret("WATSONX_API_KEY"),
        watsonx_apikey: secret("WATSONX_APIKEY"),
        watsonx_zenapikey: secret("WATSONX_ZENAPIKEY"),
        watsonx_token: secret("WATSONX_TOKEN"),
        watsonx_api_base: text("WATSONX_API_BASE"),
        watsonx_url: text("WATSONX_URL"),
        wx_url: text("WX_URL"),
        wml_url: text("WML_URL"),
        watsonx_project_id: text("WATSONX_PROJECT_ID"),
        wx_project_id: text("WX_PROJECT_ID"),
        project_id: text("PROJECT_ID"),
        watsonx_region: text("WATSONX_REGION"),
        wx_region: text("WX_REGION"),
        region: text("REGION"),
        watsonx_deployment_space_id: text("WATSONX_DEPLOYMENT_SPACE_ID"),
        watsonx_space_id: text("WATSONX_SPACE_ID"),
        wx_space_id: text("WX_SPACE_ID"),
        space_id: text("SPACE_ID"),
    }
}

fn project_environment() -> WatsonxEnvironment {
    environment(&[("WATSONX_PROJECT_ID", "project")])
}

fn empty_environment() -> WatsonxEnvironment {
    environment(&[])
}

fn factory() -> PromptFactory {
    PromptFactory {
        cache: Mutex::new(builtin_entries()),
        hub: Hub::Unreachable,
    }
}

#[test]
fn supported_params_match_litellm() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        [
            "temperature",
            "max_tokens",
            "top_p",
            "frequency_penalty",
            "stop",
            "seed",
            "stream"
        ]
    );
    assert_eq!(
        SPECIAL_AUTH_PARAMS,
        [
            ("project", "watsonx_project"),
            ("region_name", "watsonx_region_name"),
            ("token", "watsonx_token")
        ]
    );
    assert_eq!(EU_REGIONS, ["eu-de", "eu-gb"]);
    assert_eq!(US_REGIONS, ["us-south"]);
}

#[rstest]
#[case::max_tokens(r#"{"max_tokens":5}"#, r#"{"max_new_tokens":5}"#)]
#[case::stream(r#"{"stream":true}"#, r#"{"stream":true}"#)]
#[case::temperature(r#"{"temperature":0.2}"#, r#"{"temperature":0.2}"#)]
#[case::top_p(r#"{"top_p":0.9}"#, r#"{"top_p":0.9}"#)]
#[case::frequency_penalty(r#"{"frequency_penalty":0.5}"#, r#"{"repetition_penalty":0.5}"#)]
#[case::seed(r#"{"seed":42}"#, r#"{"random_seed":42}"#)]
#[case::stop(r#"{"stop":["x"]}"#, r#"{"stop_sequences":["x"]}"#)]
#[case::decoding_method(
    r#"{"decoding_method":"greedy"}"#,
    r#"{"extra_body":{"decoding_method":"greedy"}}"#
)]
#[case::min_tokens(r#"{"min_tokens":2}"#, r#"{"extra_body":{"min_new_tokens":2}}"#)]
#[case::top_k(r#"{"top_k":3}"#, r#"{"extra_body":{"top_k":3}}"#)]
#[case::truncate_input_tokens(
    r#"{"truncate_input_tokens":8}"#,
    r#"{"extra_body":{"truncate_input_tokens":8}}"#
)]
#[case::length_penalty(
    r#"{"length_penalty":{"decay_factor":2.5}}"#,
    r#"{"extra_body":{"length_penalty":{"decay_factor":2.5}}}"#
)]
#[case::time_limit(r#"{"time_limit":1000}"#, r#"{"extra_body":{"time_limit":1000}}"#)]
#[case::return_options(
    r#"{"return_options":{"input_text":true}}"#,
    r#"{"extra_body":{"return_options":{"input_text":true}}}"#
)]
#[case::unmapped_is_dropped(r#"{"moderations":{"hap":true}}"#, "{}")]
fn openai_params_map_into_the_options(#[case] wire: &str, #[case] expected: &str) {
    let mut options = WatsonxParams::new();
    map_openai_params(&params(wire), &mut options);
    assert_eq!(options, params(expected));
}

#[test]
fn extra_body_is_written_only_when_it_has_an_entry() {
    let mut options = WatsonxParams::new();
    map_openai_params(&params(r#"{"temperature":0.2}"#), &mut options);
    assert!(!options.contains_key("extra_body"));
}

#[rstest]
#[case::decoding_method("decoding_method", true)]
#[case::max_new_tokens("max_new_tokens", true)]
#[case::min_new_tokens("min_new_tokens", true)]
#[case::length_penalty("length_penalty", true)]
#[case::stop_sequences("stop_sequences", true)]
#[case::top_k("top_k", true)]
#[case::repetition_penalty("repetition_penalty", true)]
#[case::truncate_input_tokens("truncate_input_tokens", true)]
#[case::include_stop_sequences("include_stop_sequences", true)]
#[case::return_options("return_options", true)]
#[case::random_seed("random_seed", true)]
#[case::moderations("moderations", true)]
#[case::min_tokens("min_tokens", true)]
#[case::temperature("temperature", false)]
#[case::max_tokens("max_tokens", false)]
#[case::empty("", false)]
fn text_params_are_recognised(#[case] name: &str, #[case] expected: bool) {
    assert_eq!(is_watsonx_text_param(name), expected);
}

#[test]
fn special_auth_params_are_renamed() {
    let mut options = WatsonxParams::new();
    map_special_auth_params(
        &params(r#"{"project":"p","region_name":"r","token":"t","other":1}"#),
        &mut options,
    );
    assert_eq!(
        options,
        params(r#"{"watsonx_project":"p","watsonx_region_name":"r","watsonx_token":"t"}"#)
    );
}

#[test]
fn payload_has_the_source_layout() {
    let body = build_body(
        "granite",
        Some("prompt"),
        params(r#"{"stream":true,"temperature":0.2,"watsonx_token":"t"}"#),
        &project_environment(),
    )
    .unwrap();
    assert_eq!(body.input.as_deref(), Some("prompt"));
    assert_eq!(body.moderations, WatsonxParams::new());
    assert_eq!(
        body.parameters,
        params(r#"{"stream":true,"temperature":0.2,"watsonx_token":"t"}"#)
    );
    assert_eq!(
        body.identity,
        params(r#"{"model_id":"granite","project_id":"project"}"#)
    );
}

#[test]
fn payload_overlays_extra_body_before_the_api_params() {
    let body = build_body(
        "granite",
        Some("prompt"),
        params(
            r#"{"temperature":0.2,"extra_body":{"top_k":3,"temperature":0.9,"project_id":"from-extra"},"moderations":{"hap":true}}"#,
        ),
        &project_environment(),
    )
    .unwrap();
    assert_eq!(body.parameters, params(r#"{"temperature":0.9,"top_k":3}"#));
    assert_eq!(body.moderations, params(r#"{"hap":true}"#));
    assert_eq!(
        body.identity,
        params(r#"{"model_id":"granite","project_id":"from-extra"}"#)
    );
    assert!(!body.parameters.contains_key("project_id"));
}

#[test]
fn deployment_payload_has_no_identity() {
    let body = build_body(
        "deployment/deployment-id",
        Some("prompt"),
        WatsonxParams::new(),
        &empty_environment(),
    )
    .unwrap();
    assert_eq!(body.identity, WatsonxParams::new());
}

#[test]
fn payload_without_project_or_space_fails_401() {
    let error = build_body(
        "granite",
        Some("prompt"),
        WatsonxParams::new(),
        &empty_environment(),
    )
    .unwrap_err();
    assert_eq!(error.status(), Some(401));
}

#[rstest]
#[case::plain(None, "granite", false, "/ml/v1/text/generation")]
#[case::plain_stream(None, "granite", true, "/ml/v1/text/generation_stream")]
#[case::deployment(
    None,
    "deployment/deployment-id",
    false,
    "/ml/v1/deployments/deployment-id/text/generation"
)]
#[case::deployment_stream(
    None,
    "deployment/deployment-id",
    true,
    "/ml/v1/deployments/deployment-id/text/generation_stream"
)]
#[case::explicit_base(Some("https://host/"), "granite", false, "/ml/v1/text/generation")]
fn urls_follow_the_model_shape(
    #[case] api_base: Option<&str>,
    #[case] model: &str,
    #[case] stream: bool,
    #[case] endpoint: &str,
) {
    let environment = environment(&[("WATSONX_API_BASE", "https://env-host")]);
    let mut options = WatsonxParams::new();
    let url = complete_url(&environment, api_base, model, &mut options, stream).unwrap();
    let base = api_base.unwrap_or("https://env-host").trim_end_matches('/');
    assert_eq!(url, format!("{base}{endpoint}?version=2024-03-13"));
}

#[test]
fn api_version_is_popped_from_the_options() {
    let environment = environment(&[("WATSONX_API_BASE", "https://env-host")]);
    let mut options = params(r#"{"api_version":"2024-01-01"}"#);
    let url = complete_url(&environment, None, "granite", &mut options, false).unwrap();
    assert_eq!(
        url,
        "https://env-host/ml/v1/text/generation?version=2024-01-01"
    );
    assert!(!options.contains_key("api_version"));
}

#[test]
fn reply_gives_text_finish_reason_usage_and_created() {
    let response = transform_response(
        br#"{"model_id":"granite","created_at":"2024-01-01T00:00:00Z","results":[{"generated_text":"hi","input_token_count":3,"generated_token_count":2,"stop_reason":"length"}]}"#,
        now(),
    )
    .unwrap();
    assert_eq!(response.text, "hi");
    assert_eq!(response.finish_reason, "length");
    assert_eq!(response.created, 1_704_067_200);
    assert_eq!(response.usage.prompt_tokens, 3);
    assert_eq!(response.usage.completion_tokens, 2);
    assert_eq!(response.usage.total_tokens, 5);
}

#[test]
fn reply_without_results_fails_500() {
    let error = transform_response(br#"{"model_id":"granite"}"#, now()).unwrap_err();
    assert!(matches!(error, WatsonxTextError::InvalidResponse { .. }));
    assert_eq!(error.status(), Some(500));
}

#[test]
fn reply_with_an_empty_results_list_has_no_result() {
    let error = transform_response(br#"{"results":[]}"#, now()).unwrap_err();
    assert!(matches!(error, WatsonxTextError::NoResults));
    assert_eq!(error.status(), Some(500));
}

#[rstest]
#[case::missing_generated_text(
    r#"{"results":[{"input_token_count":1,"generated_token_count":2,"stop_reason":"stop"}]}"#
)]
#[case::missing_input_tokens(
    r#"{"results":[{"generated_text":"x","generated_token_count":2,"stop_reason":"stop"}]}"#
)]
#[case::missing_generated_tokens(
    r#"{"results":[{"generated_text":"x","input_token_count":1,"stop_reason":"stop"}]}"#
)]
#[case::missing_stop_reason(
    r#"{"results":[{"generated_text":"x","input_token_count":1,"generated_token_count":2}]}"#
)]
fn reply_missing_a_result_field_is_an_error(#[case] body: &str) {
    let error = transform_response(body.as_bytes(), now()).unwrap_err();
    assert!(matches!(error, WatsonxTextError::InvalidResponse { .. }));
}

#[test]
fn reply_without_created_at_uses_the_clock() {
    let response = transform_response(
        br#"{"results":[{"generated_text":"hi","input_token_count":0,"generated_token_count":0,"stop_reason":"stop"}]}"#,
        now(),
    )
    .unwrap();
    assert_eq!(response.created, 1_700_000_000);
}

#[test]
fn reply_reads_a_naive_created_at_as_utc() {
    let response = transform_response(
        br#"{"created_at":"2024-01-01T00:00:00","results":[{"generated_text":"hi","input_token_count":0,"generated_token_count":0,"stop_reason":"stop"}]}"#,
        now(),
    )
    .unwrap();
    assert_eq!(response.created, 1_704_067_200);
}

#[test]
fn reply_maps_an_unknown_stop_reason_to_stop() {
    let response = transform_response(
        br#"{"results":[{"generated_text":"hi","input_token_count":0,"generated_token_count":0,"stop_reason":"not_finished"}]}"#,
        now(),
    )
    .unwrap();
    assert_eq!(response.finish_reason, "stop");
}

#[rstest]
#[case::not_finished(
    r#"data: {"results":[{"generated_text":"a","stop_reason":"not_finished","input_token_count":2,"generated_token_count":1}]}"#,
    "a",
    Some("not_finished"),
    false,
    Some((2, 1, 3))
)]
#[case::stop(
    r#"data: {"results":[{"generated_text":"b","stop_reason":"stop","input_token_count":1,"generated_token_count":2}]}"#,
    "b",
    Some("stop"),
    true,
    Some((1, 2, 3))
)]
#[case::missing_stop_reason(
    r#"data: {"results":[{"generated_text":"c"}]}"#,
    "c",
    None,
    true,
    Some((0, 0, 0))
)]
#[case::missing_text(
    r#"data: {"results":[{"stop_reason":"stop","input_token_count":4,"generated_token_count":5}]}"#,
    "",
    Some("stop"),
    true,
    Some((4, 5, 9))
)]
#[case::tight_prefix(
    r#"data:{"results":[{"generated_text":"d","stop_reason":"stop"}]}"#,
    "d",
    Some("stop"),
    true,
    Some((0, 0, 0))
)]
fn stream_chunks_carry_text_finish_reason_and_usage(
    #[case] line: &str,
    #[case] text: &str,
    #[case] finish_reason: Option<&str>,
    #[case] is_finished: bool,
    #[case] usage: Option<(i64, i64, i64)>,
) {
    let chunk = decode_stream_line(line);
    assert_eq!(chunk.text, text);
    assert_eq!(chunk.finish_reason.as_deref(), finish_reason);
    assert_eq!(chunk.is_finished, is_finished);
    assert_eq!(
        chunk.usage.map(|counted| (
            counted.prompt_tokens,
            counted.completion_tokens,
            counted.total_tokens
        )),
        usage
    );
}

#[rstest]
#[case::empty_results(r#"data: {"results":[]}"#)]
#[case::absent_results(r#"data: {"model_id":"granite"}"#)]
fn stream_chunk_without_results_is_unfinished_stop(#[case] line: &str) {
    let chunk = decode_stream_line(line);
    assert_eq!(chunk.text, "");
    assert!(!chunk.is_finished);
    assert_eq!(chunk.finish_reason.as_deref(), Some("stop"));
    assert_eq!(chunk.usage, None);
}

#[test]
fn done_line_finishes_the_stream() {
    let chunk = decode_stream_line("data: [DONE]");
    assert!(chunk.is_finished);
    assert_eq!(chunk.finish_reason.as_deref(), Some("stop"));
    assert_eq!(chunk.usage, None);
}

#[rstest]
#[case::truncated(r#"data: {"results":"#)]
#[case::not_an_object("data: []")]
#[case::empty_object("data: {}")]
#[case::blank("")]
fn malformed_line_gives_the_empty_chunk(#[case] line: &str) {
    let chunk = decode_stream_line(line);
    assert_eq!(chunk.text, "");
    assert!(!chunk.is_finished);
    assert_eq!(chunk.finish_reason.as_deref(), Some(""));
    assert_eq!(chunk.usage, None);
}

#[tokio::test]
async fn request_converts_the_messages_with_the_model_provider() {
    let config = WatsonxTextConfig {
        environment: &project_environment(),
        prompts: &factory(),
        now: now(),
    };
    let body = config
        .transform_request(
            "ibm-mistralai/granite-chat-13b",
            &messages(r#"[{"role":"user","content":"hi"}]"#),
            params(r#"{"max_tokens":5}"#),
        )
        .await
        .unwrap();
    assert_eq!(body.input.as_deref(), Some("<s>[INST] hi [/INST]\n"));
    assert_eq!(body.parameters, params(r#"{"max_tokens":5}"#));
}

#[test]
fn replays_the_watsonx_text_reply_corpus() {
    let seen = fs::read_dir("fuzz/corpus/watsonx_text_reply")
        .unwrap()
        .map(|entry| fs::read(entry.unwrap().path()).unwrap())
        .inspect(|bytes| {
            let _replayed = decode_text_generation_reply(bytes);
        })
        .count();
    assert!(seen > 0_usize);
}

#[test]
fn reply_decoder_rejects_non_utf8_without_panicking() {
    let error = decode_text_generation_reply(&[0xff_u8, 0xfe]).unwrap_err();
    assert_eq!(error.status(), Some(500));
}
