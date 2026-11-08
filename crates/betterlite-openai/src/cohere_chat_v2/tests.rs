#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;

use super::reply::citations::{Annotation, UrlCitation, annotations};
use super::reply::tool_calls::{ReplyFunction, ReplyToolCall};
use super::reply::{AssistantReply, CohereV2Reply, ReplyError, ReplyInput, decode_cohere_chat_v2};
use super::{CohereV2ChatConfig, SUPPORTED_OPENAI_PARAMS, complete_url, map_openai_params};
use crate::chat::ChatMessage;
use crate::chat_content::RefuseDownloads;
use crate::cohere_streams::{CohereUsage, cohere_error};

fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

fn json_list(text: &str) -> Vec<Json> {
    serde_json::from_str(text).unwrap()
}

fn reply_input<'body>(
    body: &'body [u8],
    status: u16,
    created: u64,
    content: Option<&str>,
) -> ReplyInput<'body> {
    ReplyInput {
        body,
        status,
        model: "command-r",
        created,
        id: String::from("chatcmpl-template"),
        finish_reason: Some(String::from("stop")),
        message: AssistantReply {
            content: content.map(str::to_owned),
            tool_calls: None,
            annotations: None,
        },
    }
}

fn decode(body: &str) -> Result<CohereV2Reply, ReplyError> {
    decode_cohere_chat_v2(reply_input(body.as_bytes(), 200, 1234, None))
}

fn decode_preserving(body: &str, content: &str) -> Result<CohereV2Reply, ReplyError> {
    decode_cohere_chat_v2(reply_input(body.as_bytes(), 200, 1234, Some(content)))
}

fn url_annotation(start_index: i64, end_index: i64, title: &str, url: &str) -> Annotation {
    Annotation {
        kind: String::from("url_citation"),
        url_citation: UrlCitation {
            start_index,
            end_index,
            title: title.to_owned(),
            url: url.to_owned(),
        },
    }
}

#[test]
fn supported_params_match_the_v2_advertised_list() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        [
            "stream",
            "temperature",
            "max_tokens",
            "max_completion_tokens",
            "top_p",
            "frequency_penalty",
            "presence_penalty",
            "stop",
            "n",
            "tools",
            "tool_choice",
            "seed",
            "extra_headers",
        ]
    );
}

#[rstest]
#[case::max_tokens_first(
    r#"{"max_tokens": 10, "max_completion_tokens": 20}"#,
    r#"{"max_tokens": 20}"#
)]
#[case::max_completion_tokens_first(
    r#"{"max_completion_tokens": 20, "max_tokens": 10}"#,
    r#"{"max_tokens": 20}"#
)]
fn max_completion_tokens_wins_in_either_order(#[case] requested: &str, #[case] expected: &str) {
    let mapped = map_openai_params(&object(requested), Object::new());
    assert_eq!(mapped, object(expected));
}

#[test]
fn map_openai_params_renames_and_leaves_unmapped_names_alone() {
    let requested = object(
        r#"{"stream": true, "temperature": 0.1, "n": 2, "top_p": 0.5, "frequency_penalty": 0.2,
            "presence_penalty": 0.3, "stop": ["x"], "tools": [], "seed": 5, "tool_choice": "auto",
            "extra_headers": {"x-test": "1"}, "unsupported": 1}"#,
    );
    let mapped = map_openai_params(&requested, object(r#"{"kept": 1}"#));
    assert_eq!(
        mapped,
        object(
            r#"{"kept": 1, "stream": true, "temperature": 0.1, "num_generations": 2, "p": 0.5,
                "frequency_penalty": 0.2, "presence_penalty": 0.3, "stop_sequences": ["x"],
                "tools": [], "seed": 5}"#
        )
    );
}

#[test]
fn config_defaults_to_nothing_and_keeps_explicit_values() {
    assert!(CohereV2ChatConfig::default().get_config().is_empty());
    let config = CohereV2ChatConfig::with_settings(object(
        r#"{"preamble": "hello", "temperature": 0, "tools": [], "seed": 0, "max_tokens": null}"#,
    ))
    .unwrap();
    assert_eq!(
        config.get_config(),
        object(r#"{"preamble": "hello", "temperature": 0, "tools": [], "seed": 0}"#)
    );
}

#[test]
fn config_rejects_an_undeclared_setting() {
    let error =
        CohereV2ChatConfig::with_settings(object(r#"{"preamble": "hello", "temperaturez": 1}"#))
            .unwrap_err();
    assert!(matches!(
        error,
        super::CohereV2ConfigError::UnknownSetting { key } if key == "temperaturez"
    ));
}

#[test]
fn complete_url_accepts_an_empty_base_and_rejects_none() {
    assert_eq!(complete_url(Some("")).unwrap(), "");
    assert_eq!(
        complete_url(Some("https://api.cohere.com/v2/chat")).unwrap(),
        "https://api.cohere.com/v2/chat"
    );
    assert!(matches!(
        complete_url(None),
        Err(super::CohereV2ConfigError::MissingApiBase)
    ));
}

fn body_for(messages: &[ChatMessage], optional_params: &Object) -> Object {
    let environment = BTreeMap::new();
    super::request_body(&super::RequestInput {
        model: "command-r",
        messages,
        optional_params,
        api_base: None,
        environment: &environment,
        downloads: &RefuseDownloads,
    })
    .unwrap()
}

#[test]
fn request_body_delegates_to_the_openai_transformation() {
    let messages: Vec<ChatMessage> = serde_json::from_str(
        r#"[{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}}]}]"#,
    )
    .unwrap();
    let optional_params = object(
        r#"{"temperature": 0.2, "tool_choice": "auto",
            "tools": [{"type": "function", "function": {"name": "f"}, "cache_control": {"type": "ephemeral"}}]}"#,
    );
    let body = body_for(&messages, &optional_params);
    let written = serde_json::to_string(&body).unwrap();
    assert!(written.contains(r#""temperature":0.2"#));
    assert!(written.contains(r#""tool_choice":"auto""#));
    assert!(!written.contains("cache_control"));
    let without_tools = object(r#"{"tool_choice": "auto", "temperature": 0.2}"#);
    let stripped = body_for(&messages, &without_tools);
    assert!(!stripped.contains_key("tool_choice"));
    assert_eq!(
        stripped.get("model"),
        Some(&Json::Text(String::from("command-r")))
    );
}

#[test]
fn content_concatenates_every_non_null_part_and_defaults_missing_text() {
    let reply = decode_preserving(
        r#"{"message":{"content":[{"text":"hello "},null,{"text":"world"},{}]},"usage":{"tokens":{}}}"#,
        "existing",
    )
    .unwrap();
    assert_eq!(reply.message.content, Some(String::from("hello world")));
}

#[rstest]
#[case::absent_content(r#"{"message":{},"usage":{"tokens":{}}}"#)]
#[case::null_content(r#"{"message":{"content":null},"usage":{"tokens":{}}}"#)]
fn absent_or_null_content_keeps_the_existing_assistant_content(#[case] body: &str) {
    let reply = decode_preserving(body, "existing").unwrap();
    assert_eq!(reply.message.content, Some(String::from("existing")));
}

#[test]
fn an_empty_content_list_replaces_the_existing_content_with_empty_text() {
    let reply = decode(r#"{"message":{"content":[]},"usage":{"tokens":{}}}"#).unwrap();
    assert_eq!(reply.message.content, Some(String::new()));
}

#[test]
fn citations_expand_one_annotation_per_document_source() {
    let reply = decode(
        r#"{"message":{"content":[],"citations":[{"start":1,"end":4,"sources":[
            {"type":"document","document":{"title":"First"},"id":"one"},
            {"type":"document","document":{"title":"Second"},"url":"https://example.test"},
            {"type":"other","document":{"title":"Third"},"id":"three"},
            {"type":"document","id":"four"}
        ]}]},"usage":{"tokens":{}}}"#,
    )
    .unwrap();
    assert_eq!(
        reply.message.annotations,
        Some(vec![
            url_annotation(1, 4, "First", "source:one"),
            url_annotation(1, 4, "Second", "https://example.test"),
        ])
    );
}

#[rstest]
#[case::missing_start_and_end(r#"{"sources":[{"type":"document","document":{}}]}"#)]
#[case::null_start_and_end(
    r#"{"start":null,"end":null,"sources":[{"type":"document","document":{}}]}"#
)]
#[case::explicit_zero(r#"{"start":0,"end":0,"sources":[{"type":"document","document":{}}]}"#)]
fn citations_default_missing_indices_to_zero(#[case] citation: &str) {
    let found = annotations(&json_list(&format!("[{citation}]"))).unwrap();
    assert_eq!(found, vec![url_annotation(0, 0, "", "source:unknown")]);
}

#[test]
fn citations_default_a_missing_id_and_ignore_falsy_urls() {
    let found = annotations(&json_list(
        r#"[{"sources":[{"type":"document","document":{},"url":"","id":null}]},
            {"sources":[{"type":"document","document":{},"url":0,"id":"kept"}]}]"#,
    ))
    .unwrap();
    assert_eq!(found[0].url_citation.url, "source:unknown");
    assert_eq!(found[1].url_citation.url, "source:kept");
}

#[test]
fn citations_without_sources_or_documents_yield_nothing() {
    let found = annotations(&json_list(
        r#"[{"sources":[]},{"type":"document","document":{"title":"kept"}},
            {"sources":[{"type":"other","document":{}},{"type":"document"}]}]"#,
    ))
    .unwrap();
    assert!(found.is_empty());
}

fn expected_calls() -> Vec<ReplyToolCall> {
    vec![
        ReplyToolCall {
            index: 0,
            id: String::from("call_a"),
            kind: String::from("function"),
            function: ReplyFunction {
                name: Some(String::from("lookup")),
                arguments: String::from("{}"),
            },
        },
        ReplyToolCall {
            index: 1,
            id: String::from("call_b"),
            kind: String::from("function"),
            function: ReplyFunction {
                name: Some(String::from("weather")),
                arguments: String::from(r#"{"city":"Paris"}"#),
            },
        },
    ]
}

#[test]
fn tool_calls_replace_content_and_take_their_enumerate_index() {
    let reply = decode_preserving(
        r#"{"id":"provider-id","finish_reason":"COMPLETE","message":{"content":[{"text":"dropped"}],"tool_calls":[
            {"id":"call_a","type":"function","index":9,"function":{"name":"lookup","parameters":{}}},
            {"id":"call_b","type":"function","function":{"name":"weather","parameters":{"city":"Paris"}}}
        ],"citations":[{"start":2,"end":3,"sources":[{"type":"document","document":{"title":"Doc"},"id":"s"}]}]},"usage":{"tokens":{"input_tokens":1,"output_tokens":2}}}"#,
        "existing",
    )
    .unwrap();
    assert_eq!(reply.id, "chatcmpl-template");
    assert_eq!(reply.finish_reason.as_deref(), Some("stop"));
    assert_eq!(reply.created, 1234);
    assert_eq!(reply.model, "command-r");
    assert_eq!(reply.message.content, None);
    assert_eq!(reply.message.tool_calls, Some(expected_calls()));
    assert_eq!(
        reply.message.annotations,
        Some(vec![url_annotation(2, 3, "Doc", "source:s")])
    );
    assert_eq!(
        reply.usage,
        CohereUsage {
            prompt_tokens: 1,
            completion_tokens: 2,
            total_tokens: 3,
        }
    );
}

#[test]
fn annotations_attach_to_the_existing_message_only_when_present() {
    let with_citations = decode_preserving(
        r#"{"message":{"tool_calls":[],"citations":[{"start":0,"end":1,"sources":[{"type":"document","document":{}}]}]},"usage":{"tokens":{}}}"#,
        "kept",
    )
    .unwrap();
    assert_eq!(with_citations.message.content.as_deref(), Some("kept"));
    assert_eq!(
        with_citations.message.annotations,
        Some(vec![url_annotation(0, 1, "", "source:unknown")])
    );
    let without = decode_preserving(
        r#"{"message":{"citations":[]},"usage":{"tokens":{}}}"#,
        "kept",
    )
    .unwrap();
    assert_eq!(without.message.annotations, None);
}

#[test]
fn usage_reads_tokens_and_ignores_billed_units() {
    let reply = decode(
        r#"{"message":{"content":[]},"usage":{"tokens":{"input_tokens":7,"output_tokens":9},"billed_units":{"input_tokens":100,"output_tokens":100}}}"#,
    )
    .unwrap();
    assert_eq!(
        reply.usage,
        CohereUsage {
            prompt_tokens: 7,
            completion_tokens: 9,
            total_tokens: 16,
        }
    );
    let empty = decode(r#"{"message":{"content":[]},"usage":{"tokens":{}}}"#).unwrap();
    assert_eq!(empty.usage.total_tokens, 0);
}

#[test]
fn json_failures_keep_the_upstream_status_and_mapping_failures_are_422() {
    let input = reply_input;
    let invalid = decode_cohere_chat_v2(input(b"{not json", 502, 0, None)).unwrap_err();
    assert_eq!(invalid.status_code(), Some(502));
    let array = decode_cohere_chat_v2(input(b"[1,2]", 200, 0, None)).unwrap_err();
    assert_eq!(array.status_code(), Some(422));
    assert!(matches!(array, ReplyError::NotAMapping));
}

#[rstest]
#[case::missing_message(r#"{"usage":{"tokens":{}}}"#)]
#[case::message_not_object(r#"{"message":"text","usage":{"tokens":{}}}"#)]
#[case::content_not_list(r#"{"message":{"content":"text"},"usage":{"tokens":{}}}"#)]
#[case::content_part_not_object(r#"{"message":{"content":["text"]},"usage":{"tokens":{}}}"#)]
#[case::content_text_not_string(r#"{"message":{"content":[{"text":5}]},"usage":{"tokens":{}}}"#)]
#[case::citations_not_list(r#"{"message":{"citations":"text"},"usage":{"tokens":{}}}"#)]
#[case::citation_not_object(r#"{"message":{"citations":[5]},"usage":{"tokens":{}}}"#)]
#[case::source_not_object(r#"{"message":{"citations":[{"sources":["x"]}]},"usage":{"tokens":{}}}"#)]
#[case::document_not_object(
    r#"{"message":{"citations":[{"sources":[{"type":"document","document":null}]}]},"usage":{"tokens":{}}}"#
)]
#[case::start_not_integer(
    r#"{"message":{"citations":[{"start":"1","sources":[]}]},"usage":{"tokens":{}}}"#
)]
#[case::tool_calls_not_list(r#"{"message":{"tool_calls":{}},"usage":{"tokens":{}}}"#)]
#[case::tool_call_without_id(
    r#"{"message":{"tool_calls":[{"type":"function","function":{}}]},"usage":{"tokens":{}}}"#
)]
#[case::tool_call_without_function(
    r#"{"message":{"tool_calls":[{"id":"a"}]},"usage":{"tokens":{}}}"#
)]
#[case::missing_usage(r#"{"message":{"content":[]}}"#)]
#[case::usage_not_object(r#"{"message":{"content":[]},"usage":null}"#)]
#[case::tokens_not_object(r#"{"message":{"content":[]},"usage":{"tokens":[]}}"#)]
#[case::token_not_integer(r#"{"message":{"content":[]},"usage":{"tokens":{"input_tokens":"7"}}}"#)]
fn later_field_access_failures_carry_no_status(#[case] body: &str) {
    let error = decode(body).unwrap_err();
    assert_eq!(error.status_code(), None, "status for {body}");
}

#[test]
fn tool_arguments_without_parameters_are_empty_and_missing_type_is_function() {
    let reply = decode(
        r#"{"message":{"tool_calls":[{"id":"call_a","function":{"name":"noop"}}]},"usage":{"tokens":{}}}"#,
    )
    .unwrap();
    let call = &reply.message.tool_calls.unwrap()[0];
    assert_eq!(call.kind, "function");
    assert_eq!(call.function.arguments, "");
    assert_eq!(call.function.name.as_deref(), Some("noop"));
    assert_eq!(call.index, 0);
}

#[test]
fn token_total_overflow_is_its_own_failure() {
    let body = format!(
        r#"{{"message":{{"content":[]}},"usage":{{"tokens":{{"input_tokens":{},"output_tokens":1}}}}}}"#,
        i64::MAX
    );
    let error = decode(&body).unwrap_err();
    assert!(matches!(error, ReplyError::UsageOverflow));
    assert_eq!(error.status_code(), None);
}

#[rstest]
fn replays_the_cohere_chat_v2_corpus(#[files("fuzz/corpus/cohere_chat_v2/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let _decoded: Result<CohereV2Reply, ReplyError> =
        decode_cohere_chat_v2(reply_input(&bytes, 200, 0, None));
}

#[test]
fn v2_error_class_keeps_status_and_message_without_headers() {
    let error = cohere_error(418, String::from("teapot"));
    assert_eq!(error.status_code, 418);
    assert_eq!(error.headers, None);
}
