#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_models::decisions::Json as ReplyJson;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use rstest::rstest;

use crate::cohere_streams::CohereUsage;
use crate::error::ProviderError;
use crate::openai_base_config::Headers;

use super::error::{CohereError, CohereV1ConfigError, get_error_class};
use super::reply::{CohereV1Reply, ReplyError, ReplyInput, decode_cohere_chat_v1};
use super::{
    CohereV1ChatConfig, RequestInput, SUPPORTED_OPENAI_PARAMS, map_openai_params, request_options,
};

fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

fn reply_json(text: &str) -> ReplyJson {
    serde_json::from_str(text).unwrap()
}

fn messages(text: &str) -> Vec<Message> {
    serde_json::from_str(text).unwrap()
}

fn configured(settings: &str) -> CohereV1ChatConfig {
    CohereV1ChatConfig::with_settings(object(settings)).unwrap()
}

fn request_result(
    settings: &str,
    optional: &str,
    messages_text: &str,
) -> Result<Object, CohereV1ConfigError> {
    let config = configured(settings);
    let messages = messages(messages_text);
    request_options(&RequestInput {
        config: &config,
        messages: &messages,
        optional_params: &object(optional),
    })
}

fn request_value(settings: &str, optional: &str, messages_text: &str) -> Object {
    request_result(settings, optional, messages_text).unwrap()
}

fn reply_input(body: &[u8], status: u16, created: u64) -> ReplyInput<'_> {
    ReplyInput {
        body,
        status,
        model: "command-r",
        created,
        id: String::from("chatcmpl-template"),
        finish_reason: Some(String::from("stop")),
    }
}

fn decode(body: &str, status: u16) -> Result<CohereV1Reply, ReplyError> {
    decode_cohere_chat_v1(reply_input(body.as_bytes(), status, 1234))
}

fn reply(body: &str) -> Result<CohereV1Reply, ReplyError> {
    decode(body, 200)
}

#[test]
fn supported_params_match_the_v1_advertised_list() {
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

#[rstest]
#[case::max_tokens_first(
    r#"{"max_tokens": 10, "max_completion_tokens": 20}"#,
    r#"{"max_tokens": 20}"#
)]
#[case::max_completion_tokens_first(
    r#"{"max_completion_tokens": 20, "max_tokens": 10}"#,
    r#"{"max_tokens": 10}"#
)]
#[case::max_tokens_alone(r#"{"max_tokens": 10}"#, r#"{"max_tokens": 10}"#)]
#[case::max_completion_tokens_alone(r#"{"max_completion_tokens": 20}"#, r#"{"max_tokens": 20}"#)]
fn both_token_limits_write_max_tokens_in_input_order(
    #[case] requested: &str,
    #[case] expected: &str,
) {
    let mapped = map_openai_params(&object(requested), Object::new());
    assert_eq!(mapped, object(expected));
}

#[test]
fn config_defaults_to_nothing_and_keeps_explicit_values() {
    assert!(CohereV1ChatConfig::default().get_config().is_empty());
    let config = configured(
        r#"{"preamble": "hello", "temperature": 0, "tools": [], "seed": 0, "max_tokens": null}"#,
    );
    assert_eq!(
        config.get_config(),
        object(r#"{"preamble": "hello", "temperature": 0, "tools": [], "seed": 0}"#)
    );
    let empty_strings =
        configured(r#"{"preamble": "", "conversation_id": null, "max_completion_tokens": null}"#);
    assert_eq!(empty_strings.get_config(), object(r#"{"preamble": ""}"#));
}

#[test]
fn config_rejects_an_undeclared_setting() {
    let error =
        CohereV1ChatConfig::with_settings(object(r#"{"preamble": "hello", "temperaturez": 1}"#))
            .unwrap_err();
    assert!(matches!(
        error,
        CohereV1ConfigError::UnknownSetting { key } if key == "temperaturez"
    ));
}

#[test]
fn request_fills_only_missing_options_from_the_config_defaults() {
    let options = request_value(
        r#"{"preamble": "be brief", "temperature": 0.5, "max_tokens": 99}"#,
        r#"{"temperature": 0, "max_tokens": null, "seed": 1}"#,
        r#"[{"role":"user","content":"hi"}]"#,
    );
    assert_eq!(
        options,
        object(
            r#"{"temperature": 0, "max_tokens": null, "seed": 1, "preamble": "be brief",
                "message": "hi"}"#
        )
    );
}

#[test]
fn request_selects_the_final_message_and_never_inserts_the_history() {
    let options = request_value(
        "{}",
        "{}",
        r#"[{"role":"user","content":"first"},{"role":"user","content":"second"}]"#,
    );
    assert_eq!(
        options,
        object(r#"{"message": "second", "force_single_step": true}"#)
    );
}

#[test]
fn request_leaves_force_single_step_out_when_the_history_ends_with_chatbot() {
    let options = request_value(
        "{}",
        "{}",
        r#"[
            {"role":"user","content":"first"},
            {"role":"assistant","content":"answer"},
            {"role":"user","content":"second"}
        ]"#,
    );
    assert_eq!(options, object(r#"{"message": "second"}"#));
}

#[test]
fn a_final_tool_result_becomes_a_singleton_tool_results_list() {
    let options = request_value(
        "{}",
        "{}",
        r#"[
            {"role":"user","content":"weather?"},
            {"role":"assistant","content":null,"tool_calls":[
                {"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{\"city\":\"Paris\"}"}}
            ]},
            {"role":"tool","tool_call_id":"call_1","content":"{\"temp\": 21}"}
        ]"#,
    );
    assert_eq!(
        options,
        object(
            r#"{"tool_results": [{"call": {"name": "get_weather", "parameters": {"city": "Paris"}},
                "outputs": [{"temp": 21}]}], "force_single_step": true}"#
        )
    );
}

#[test]
fn tools_keep_only_name_description_type_and_required() {
    let options = request_value(
        "{}",
        r#"{"tools": [{"type":"function","function":{"name":"get_weather","description":"Weather",
            "parameters":{"type":"object","properties":{
                "location":{"type":"string","description":"City","enum":["a","b"]},
                "nested":{"type":"object","properties":{"inner":{"type":"string"}}},
                "free":{"description":"x"},
                "unit":{"type":"string"}
            },"required":["location"]}}}]}"#,
        r#"[{"role":"user","content":"hi"}]"#,
    );
    assert_eq!(
        options,
        object(
            r#"{"message": "hi", "tools": [{"name": "get_weather", "description": "Weather",
                "parameter_definitions": {
                    "location": {"description": "City", "type": "string", "required": true},
                    "nested": {"description": "", "type": "object", "required": false},
                    "free": {"description": "x", "type": "", "required": false},
                    "unit": {"description": "", "type": "string", "required": false}
                }}]}"#
        )
    );
}

#[test]
fn a_tool_without_a_description_is_rejected() {
    let error = request_result(
        "{}",
        r#"{"tools": [{"function": {"name": "noop", "parameters": {"properties": {}}}}]}"#,
        r#"[{"role":"user","content":"hi"}]"#,
    )
    .unwrap_err();
    assert!(matches!(
        &error,
        CohereV1ConfigError::ToolShape { index: 0, .. }
    ));
    assert!(error.to_string().contains("description"));
}

#[test]
fn an_explicit_null_tools_value_becomes_an_empty_list() {
    let options = request_value(
        "{}",
        r#"{"tools": null}"#,
        r#"[{"role":"user","content":"hi"}]"#,
    );
    assert_eq!(options, object(r#"{"tools": [], "message": "hi"}"#));
}

#[test]
fn a_reply_with_tools_but_no_text_fails_with_the_upstream_status() {
    let error = decode(
        r#"{"tool_calls":[{"name":"lookup","generation_id":"g","parameters":{}}]}"#,
        429,
    )
    .unwrap_err();
    assert!(matches!(error, ReplyError::Text(429)));
    assert_eq!(error.status_code(), Some(429));
}

#[rstest]
#[case::invalid_json("{not json")]
#[case::array_root("[1, 2]")]
#[case::missing_text(r#"{"tool_calls": []}"#)]
fn parse_and_text_failures_keep_the_upstream_status(#[case] body: &str) {
    let error = decode(body, 502).unwrap_err();
    assert_eq!(error.status_code(), Some(502));
    assert!(matches!(
        error,
        ReplyError::Json(502, _) | ReplyError::Text(502)
    ));
}

#[test]
fn top_level_citations_are_forwarded_even_when_null() {
    let present = reply(r#"{"text":"hi","citations":[{"start":1,"end":2}]}"#).unwrap();
    assert_eq!(
        present.citations,
        Some(reply_json(r#"[{"start":1,"end":2}]"#))
    );
    let null = reply(r#"{"text":"hi","citations":null}"#).unwrap();
    assert_eq!(null.citations, Some(ReplyJson::Null));
    let absent = reply(r#"{"text":"hi"}"#).unwrap();
    assert_eq!(absent.citations, None);
}

#[test]
fn tool_calls_replace_the_content_and_carry_python_arguments() {
    let decoded = reply(
        r#"{"text":"dropped","tool_calls":[
            {"name":"lookup","generation_id":"gen-1","parameters":{
                "city": "Paris", "days": [1, 2], "note": "\u00e9", "ratio": 1.5, "ok": true,
                "none": null}},
            {"parameters":{}},
            {"name":null,"generation_id":null,"parameters":null}
        ]}"#,
    )
    .unwrap();
    assert_eq!(decoded.content, None);
    let calls = decoded.tool_calls.unwrap();
    assert_eq!(calls[0].id, "call_gen-1");
    assert_eq!(calls[0].kind, "function");
    assert_eq!(calls[0].function.name.as_deref(), Some("lookup"));
    assert_eq!(
        calls[0].function.arguments,
        r#"{"city": "Paris", "days": [1, 2], "note": "\u00e9", "ratio": 1.5, "ok": true, "none": null}"#
    );
    assert_eq!(calls[1].id, "call_");
    assert_eq!(calls[1].function.name.as_deref(), Some(""));
    assert_eq!(calls[1].function.arguments, "{}");
    assert_eq!(calls[2].id, "call_");
    assert_eq!(calls[2].function.name, None);
    assert_eq!(calls[2].function.arguments, "null");
}

#[test]
fn an_empty_tool_calls_list_keeps_the_text_content() {
    let kept = reply(r#"{"text":"kept","tool_calls":[]}"#).unwrap();
    assert_eq!(kept.content.as_deref(), Some("kept"));
    assert_eq!(kept.tool_calls, None);
    let null_tools = reply(r#"{"text":"kept","tool_calls":null}"#).unwrap();
    assert_eq!(null_tools.content.as_deref(), Some("kept"));
    assert_eq!(null_tools.tool_calls, None);
}

#[test]
fn usage_reads_billed_units_and_defaults_missing_counts_to_zero() {
    let billed = reply(
        r#"{"text":"hi","meta":{"billed_units":{"input_tokens":7,"output_tokens":9},
            "tokens":{"input_tokens":100,"output_tokens":200}}}"#,
    )
    .unwrap();
    assert_eq!(
        billed.usage,
        CohereUsage {
            prompt_tokens: 7,
            completion_tokens: 9,
            total_tokens: 16,
        }
    );
    let missing = reply(r#"{"text":"hi"}"#).unwrap();
    assert_eq!(
        missing.usage,
        CohereUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
        }
    );
    let empty_meta = reply(r#"{"text":"hi","meta":{}}"#).unwrap();
    assert_eq!(empty_meta.usage.total_tokens, 0);
    let null_counts = reply(
        r#"{"text":"hi","meta":{"billed_units":{"input_tokens":null,"output_tokens":null}}}"#,
    )
    .unwrap();
    assert_eq!(null_counts.usage.total_tokens, 0);
}

#[test]
fn whole_float_token_counts_are_read_as_integers() {
    let billed =
        reply(r#"{"text":"hi","meta":{"billed_units":{"input_tokens":9.0,"output_tokens":4.0}}}"#)
            .unwrap();
    assert_eq!(
        billed.usage,
        CohereUsage {
            prompt_tokens: 9,
            completion_tokens: 4,
            total_tokens: 13,
        }
    );
}

#[test]
fn the_reply_takes_the_request_model_and_clock_and_keeps_the_other_fields() {
    let reply = decode_cohere_chat_v1(reply_input(
        br#"{"text":"hi","model":"provider-model","finish_reason":"COMPLETE"}"#,
        200,
        1_700_000_000,
    ))
    .unwrap();
    assert_eq!(reply.model, "command-r");
    assert_eq!(reply.created, 1_700_000_000);
    assert_eq!(reply.finish_reason.as_deref(), Some("stop"));
    assert_eq!(reply.id, "chatcmpl-template");
    assert_eq!(reply.content.as_deref(), Some("hi"));
}

#[rstest]
#[case::text_not_a_string(r#"{"text":5}"#)]
#[case::tool_calls_not_a_list(r#"{"text":"hi","tool_calls":{}}"#)]
#[case::tool_not_an_object(r#"{"text":"hi","tool_calls":[5]}"#)]
#[case::tool_name_not_a_string(r#"{"text":"hi","tool_calls":[{"name":5}]}"#)]
#[case::generation_id_not_a_string(r#"{"text":"hi","tool_calls":[{"generation_id":5}]}"#)]
#[case::meta_not_an_object(r#"{"text":"hi","meta":[]}"#)]
#[case::billed_units_not_an_object(r#"{"text":"hi","meta":{"billed_units":[]}}"#)]
#[case::token_not_an_integer(r#"{"text":"hi","meta":{"billed_units":{"input_tokens":"7"}}}"#)]
#[case::token_with_a_fraction(r#"{"text":"hi","meta":{"billed_units":{"input_tokens":1.5}}}"#)]
fn later_field_failures_carry_no_status(#[case] body: &str) {
    let error = reply(body).unwrap_err();
    assert_eq!(error.status_code(), None, "status for {body}");
}

#[test]
fn token_total_overflow_is_its_own_failure() {
    let body = format!(
        r#"{{"text":"hi","meta":{{"billed_units":{{"input_tokens":{},"output_tokens":1}}}}}}"#,
        i64::MAX
    );
    let error = reply(&body).unwrap_err();
    assert!(matches!(error, ReplyError::UsageOverflow));
    assert_eq!(error.status_code(), None);
}

#[test]
fn the_v1_error_uses_the_v1_chat_request_and_drops_headers() {
    let provider = ProviderError::from(CohereError {
        status_code: 418,
        message: String::from("teapot"),
    });
    assert_eq!(provider.status_code, 418);
    assert_eq!(provider.request.method, "POST");
    assert_eq!(provider.request.url, "https://api.cohere.ai/v1/chat");
    assert_eq!(provider.response.status_code, 418);
    assert_eq!(provider.response.request, provider.request);
    assert!(provider.headers.is_none());
    assert!(provider.body.is_none());
    assert!(!provider.status_code_is_synthesized);

    let mut headers = Headers::new();
    headers.insert(String::from("x-request-id"), String::from("abc"));
    let dropped = ProviderError::from(get_error_class("bad", 429, &headers));
    assert!(dropped.headers.is_none());
    assert_eq!(dropped.message, "bad");
    assert_eq!(dropped.status_code, 429);
}

#[test]
fn a_reply_error_keeps_the_upstream_status_in_the_v1_error() {
    let provider = ProviderError::from(decode("{not json", 503).unwrap_err());
    assert_eq!(provider.status_code, 503);
    assert_eq!(provider.request.url, "https://api.cohere.ai/v1/chat");
    assert!(
        provider
            .message
            .starts_with("the Cohere response is not valid JSON")
    );
}

#[rstest]
fn replays_the_cohere_chat_v1_corpus(#[files("fuzz/corpus/cohere_chat_v1/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let _decoded: Result<CohereV1Reply, ReplyError> =
        decode_cohere_chat_v1(reply_input(&bytes, 200, 0));
}
