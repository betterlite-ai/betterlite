#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_models::decisions::{Extra, Json};
use betterlite_prompts::message_content::{Content, Message};
use betterlite_tokenizer::count::error::CountError;
use rstest::rstest;
use uuid::Uuid;

use super::{
    AgentEngineTarget, AgentQueryPayload, HAS_CUSTOM_STREAM_WRAPPER, SHOULD_FAKE_STREAM,
    STREAM_PARAM_IN_BODY, SUPPORTED_OPENAI_PARAMS, TOKEN_MODEL, TokenCounter, complete_url,
    decode_vertex_agent_buffer, map_openai_params, transform_request, transform_response,
};
use crate::vertex_agent_engine::error::{AgentEngineError, get_error_class};

const GENERATED_USER: &str = "litellm-user-123e4567";
const MODEL: &str = "agent_engine/7";

fn params(source: &str) -> Extra {
    serde_json::from_str(source).unwrap()
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn message(role: &str, content: &str) -> Message {
    Message {
        role: role.to_owned(),
        content: Some(Content::Text(content.to_owned())),
        name: None,
        provider_specific_fields: Extra::new(),
    }
}

fn target<'call>(
    model: &'call str,
    project: Option<&'call str>,
    location: Option<&'call str>,
) -> AgentEngineTarget<'call> {
    AgentEngineTarget {
        model,
        project,
        location,
    }
}

fn generated() -> Uuid {
    Uuid::parse_str("123e4567-e89b-12d3-a456-426614174000").unwrap()
}

fn payload(source: &str) -> Result<AgentQueryPayload, AgentEngineError> {
    transform_request(&[message("user", "hi")], &params(source), generated())
}

fn body_of(lines: &[&str]) -> Vec<u8> {
    lines.join("\n").into_bytes()
}

struct FixedCounter;

impl TokenCounter for FixedCounter {
    async fn count_messages(&self, model: &str, _messages: &[Message]) -> Result<u64, CountError> {
        if model == TOKEN_MODEL {
            return Ok(3);
        }
        Err(CountError::Overflow)
    }

    async fn count_response_tokens(&self, model: &str, text: &str) -> Result<u64, CountError> {
        if model != TOKEN_MODEL || text != "hi" {
            return Err(CountError::Overflow);
        }
        Ok(5)
    }
}

struct FailingCounter;

impl TokenCounter for FailingCounter {
    async fn count_messages(&self, _model: &str, _messages: &[Message]) -> Result<u64, CountError> {
        Err(CountError::Overflow)
    }

    async fn count_response_tokens(&self, _model: &str, _text: &str) -> Result<u64, CountError> {
        Err(CountError::Overflow)
    }
}

#[test]
fn the_supported_openai_params_are_user_only() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["user"]);
}

#[rstest]
#[case::user(r#"{"user":"ada"}"#, r#"{"user_id":"ada"}"#)]
#[case::other(r#"{"temperature":0.5}"#, r#"{"temperature":0.5}"#)]
fn map_openai_params_copies_user_to_user_id(#[case] non_default: &str, #[case] expected: &str) {
    let mapped = map_openai_params(&params(non_default), Extra::new());
    assert_eq!(mapped, params(expected));
}

#[rstest]
#[case::configured(Some("europe-west4"))]
#[case::absent(None)]
fn complete_url_builds_the_regional_stream_query(#[case] location: Option<&str>) {
    let region = location.unwrap_or("us-central1");
    let url = complete_url(&target(MODEL, Some("proj"), location)).unwrap();
    assert_eq!(
        url,
        format!(
            "https://{region}-aiplatform.googleapis.com/v1beta1/projects/proj/locations/{region}/reasoningEngines/7:streamQuery"
        )
    );
}

#[test]
fn a_full_resource_path_keeps_its_region_and_takes_the_configured_host() {
    let url = complete_url(&target(
        "agent_engine/projects/q/locations/europe-west4/reasoningEngines/9",
        Some("ignored"),
        Some("us-west1"),
    ))
    .unwrap();
    assert_eq!(
        url,
        "https://us-west1-aiplatform.googleapis.com/v1beta1/projects/q/locations/europe-west4/reasoningEngines/9:streamQuery"
    );
}

#[test]
fn only_one_agent_engine_prefix_is_removed() {
    let url = complete_url(&target("agent_engine/agent_engine/7", Some("proj"), None)).unwrap();
    assert!(url.ends_with("/reasoningEngines/agent_engine/7:streamQuery"));
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn a_model_without_a_resource_path_needs_the_project(#[case] project: Option<&str>) {
    let error = complete_url(&target(MODEL, project, None)).unwrap_err();
    assert!(matches!(error, AgentEngineError::ProjectRequired));
    assert_eq!(error.status_code(), Some(400));
}

#[test]
fn an_injected_location_never_chooses_the_host() {
    let error = complete_url(&target(MODEL, Some("proj"), Some("attacker.example/"))).unwrap_err();
    assert!(matches!(error, AgentEngineError::Location(_)));
    assert_eq!(error.status_code(), Some(400));
}

#[test]
fn the_last_message_is_the_query_prompt() {
    let mut search = message("user", "second");
    search
        .provider_specific_fields
        .insert("search_results".to_owned(), json("[]"));
    let messages = [message("user", "first"), search];
    let built = transform_request(&messages, &params("{}"), generated()).unwrap();
    assert_eq!(built.input.message, "second");
}

#[test]
fn a_search_result_contributes_its_text_to_the_prompt() {
    let mut search = message("user", "q");
    search.provider_specific_fields.insert(
        "search_results".to_owned(),
        json(r#"[{"title":"t","content":[{"text":"c"}]}]"#),
    );
    let built = transform_request(&[search], &params("{}"), generated()).unwrap();
    assert_eq!(built.input.message, "qtc");
}

#[test]
fn empty_messages_are_rejected() {
    let error = transform_request(&[], &params("{}"), generated()).unwrap_err();
    assert!(matches!(error, AgentEngineError::EmptyMessages));
    assert_eq!(error.status_code(), Some(400));
}

#[test]
fn user_id_wins_over_user() {
    let built = payload(r#"{"user_id":"first","user":"second"}"#).unwrap();
    assert_eq!(built.input.user_id, "first");
}

#[test]
fn an_empty_user_id_falls_through_to_user() {
    let built = payload(r#"{"user_id":"","user":"second"}"#).unwrap();
    assert_eq!(built.input.user_id, "second");
}

#[test]
fn a_generated_user_id_keeps_eight_uuid_characters() {
    let built = payload("{}").unwrap();
    assert_eq!(built.input.user_id, GENERATED_USER);
}

#[rstest]
#[case::kept(r#"{"session_id":"session"}"#, Some("session"))]
#[case::empty(r#"{"session_id":""}"#, None)]
#[case::null(r#"{"session_id":null}"#, None)]
#[case::absent("{}", None)]
fn only_a_truthy_session_id_is_sent(#[case] optional: &str, #[case] expected: Option<&str>) {
    let built = payload(optional).unwrap();
    assert_eq!(built.input.session_id.as_deref(), expected);
}

#[rstest]
#[case::user_id(r#"{"user_id":5}"#)]
#[case::user(r#"{"user":5}"#)]
#[case::session(r#"{"session_id":5}"#)]
fn a_non_string_parameter_is_rejected(#[case] optional: &str) {
    let error = payload(optional).unwrap_err();
    assert!(matches!(error, AgentEngineError::FieldType { .. }));
    assert_eq!(error.status_code(), Some(400));
}

#[test]
fn the_payload_is_class_method_then_input() {
    let built = payload(r#"{"user":"ada","session_id":"s"}"#).unwrap();
    assert_eq!(
        serde_json::to_string(&built).unwrap(),
        r#"{"class_method":"stream_query","input":{"message":"hi","user_id":"ada","session_id":"s"}}"#
    );
}

#[test]
fn the_last_non_empty_snapshot_is_the_answer() {
    let body = body_of(&[
        r#"{"content":{"parts":[{"text":"first"}]}}"#,
        r#"{"content":{"parts":[{"text":"second"}]}}"#,
        r#"{"content":{"parts":[{}]},"actions":{"state_delta":{}}}"#,
    ]);
    let answer = decode_vertex_agent_buffer(200, &body, MODEL).unwrap();
    assert_eq!(answer.content, "second");
}

#[test]
fn an_empty_first_text_part_blocks_the_state_delta_fallback() {
    let body = body_of(&[
        r#"{"content":{"parts":[{"text":""}]},"actions":{"state_delta":{"k":"delta"}}}"#,
    ]);
    let answer = decode_vertex_agent_buffer(200, &body, MODEL).unwrap();
    assert_eq!(answer.content, "");
}

#[test]
fn the_state_delta_fallback_takes_the_first_non_empty_string() {
    let body = body_of(&[
        r#"{"content":{"parts":[{}]},"actions":{"state_delta":{"a":5,"b":"","c":"first","d":"second"}}}"#,
    ]);
    let answer = decode_vertex_agent_buffer(200, &body, MODEL).unwrap();
    assert_eq!(answer.content, "first");
}

#[test]
fn malformed_and_data_prefixed_lines_are_ignored() {
    let ignored = body_of(&[
        r#"data: {"content":{"parts":[{"text":"sse"}]}}"#,
        "not json",
        "[1,2]",
    ]);
    let answer = decode_vertex_agent_buffer(200, &ignored, MODEL).unwrap();
    assert_eq!(answer.content, "");
    let mixed = body_of(&[
        r#"data: {"content":{"parts":[{"text":"sse"}]}}"#,
        r#"{"content":{"parts":[{"text":"real"}]}}"#,
    ]);
    let answer = decode_vertex_agent_buffer(200, &mixed, MODEL).unwrap();
    assert_eq!(answer.content, "real");
}

#[test]
fn the_choice_is_one_assistant_stop_at_index_zero() {
    let answer = decode_vertex_agent_buffer(200, b"", MODEL).unwrap();
    assert_eq!(answer.model, MODEL);
    assert_eq!(answer.index, 0);
    assert_eq!(answer.finish_reason, "stop");
    assert_eq!(answer.role, "assistant");
    assert_eq!(answer.usage, None);
}

#[test]
fn a_non_utf8_body_fails_with_the_raw_status() {
    let error = decode_vertex_agent_buffer(503, &[0xff_u8], MODEL).unwrap_err();
    assert!(matches!(error, AgentEngineError::NotUtf8(503, _)));
    assert_eq!(error.status_code(), Some(503));
}

#[tokio::test]
async fn the_usage_counts_with_gpt_3_5_turbo_whatever_the_agent_model() {
    let messages = [message("user", "hi")];
    let answer = transform_response(200, b"", MODEL, &messages, &FixedCounter)
        .await
        .unwrap();
    let usage = answer.usage.unwrap();
    assert_eq!(usage.prompt_tokens, 3);
    assert_eq!(usage.completion_tokens, 5);
    assert_eq!(usage.total_tokens, 8);
}

#[tokio::test]
async fn a_counter_failure_leaves_the_usage_unset() {
    let messages = [message("user", "hi")];
    let answer = transform_response(200, b"", MODEL, &messages, &FailingCounter)
        .await
        .unwrap();
    assert_eq!(answer.usage, None);
}

#[test]
fn the_capability_flags_match_agent_engine() {
    assert!(HAS_CUSTOM_STREAM_WRAPPER);
    assert!(!STREAM_PARAM_IN_BODY);
    assert!(!SHOULD_FAKE_STREAM);
}

#[test]
fn get_error_class_keeps_the_raw_status() {
    let error = get_error_class("boom", 429);
    assert_eq!(error.status_code(), Some(429));
    assert_eq!(error.to_string(), "boom");
}

#[rstest]
fn replays_the_vertex_agent_buffer_corpus(
    #[files("fuzz/corpus/vertex_agent_buffer/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    drop(decode_vertex_agent_buffer(200, &bytes, MODEL));
}
