#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_openai::chat_choices::stream::chunk_parser;
use betterlite_openai::chat_choices::wire::StreamReply;
use betterlite_openai::openai_base_config::{CatalogCapabilities, Object};
use rstest::rstest;
use serde_json::{from_str, json, to_value};
use uuid::Uuid;

use super::ai21::Ai21Config;
use super::gpt_oss::GptOssConfig;
use super::identity::PartnerClass;
use super::llama::Llama3Config;
use super::llama::reply::decode_reply;
use super::llama::stream::LlamaStreamState;
use super::settings::PartnerSettings;

const AI21_MODEL: &str = "jamba-1.5-large@001";
const LLAMA_MODEL: &str = "meta/llama-3.1-405b-instruct-maas";
// a Model Garden publisher id with one segment is LiteLLM's self-deployed endpoint shape
const DEPLOYED_MODEL: &str = "openai/gpt-5";
const GPT_OSS_MODEL: &str = "openai/gpt-oss-120b-maas";
const GPT_OSS_FUNDED_MODEL: &str = "groq/openai/gpt-oss-120b";

fn catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

fn object(source: &str) -> Object {
    from_str(source).expect("the fixture is an object")
}

fn chunk(source: &str) -> StreamReply {
    let wire = from_str(source).expect("the fixture is a chunk");
    chunk_parser(&wire, Uuid::nil, || 7).expect("the fixture parses as a chunk")
}

#[test]
fn ai21_validates_the_base_list_but_maps_reasoning_effort() {
    let supported = Ai21Config::supported_params(AI21_MODEL, &catalog());
    assert_eq!(supported.len(), 31);
    assert!(!supported.contains(&"reasoning_effort"));
    assert!(supported.contains(&"max_retries"));
    assert!(supported.contains(&"response_format"));

    let requested = object(r#"{"reasoning_effort":"low","user":"max"}"#);
    let mut existing = Object::new();
    Ai21Config::map_params(AI21_MODEL, &requested, &mut existing, &catalog());
    assert_eq!(existing, object(r#"{"reasoning_effort":"low"}"#));
}

#[test]
fn ai21_overwrites_max_tokens_with_the_completion_alias() {
    let requested = object(r#"{"max_tokens":8,"max_completion_tokens":16,"temperature":0.5}"#);
    let mut existing = Object::new();
    Ai21Config::map_params(AI21_MODEL, &requested, &mut existing, &catalog());
    assert_eq!(existing, object(r#"{"max_tokens":16,"temperature":0.5}"#));

    let mut existing = object(r#"{"max_tokens":1}"#);
    Ai21Config::map_params(
        AI21_MODEL,
        &object(r#"{"max_tokens":8}"#),
        &mut existing,
        &catalog(),
    );
    assert_eq!(existing, object(r#"{"max_tokens":8}"#));
}

#[test]
fn gpt_oss_appends_reasoning_effort_to_the_base_list() {
    let supported = GptOssConfig::supported_params(GPT_OSS_MODEL, &catalog(), None);
    assert_eq!(supported.len(), 32);
    assert!(supported.contains(&"reasoning_effort"));
    assert!(supported.contains(&"tool_choice"));
}

#[test]
fn gpt_oss_removes_the_tool_params_only_when_function_calling_is_false() {
    let disabled = GptOssConfig::supported_params(GPT_OSS_MODEL, &catalog(), Some(false));
    assert_eq!(disabled.len(), 29);
    assert!(!disabled.contains(&"tool_choice"));
    assert!(!disabled.contains(&"function_call"));
    assert!(!disabled.contains(&"functions"));
    // the removal list spells its first entry "tool", so the inherited "tools" survives
    assert!(disabled.contains(&"tools"));
}

#[rstest]
#[case::unspecified(None)]
#[case::supported(Some(true))]
fn gpt_oss_keeps_the_tool_params_unless_function_calling_is_false(#[case] support: Option<bool>) {
    let supported = GptOssConfig::supported_params(GPT_OSS_MODEL, &catalog(), support);
    assert_eq!(supported.len(), 32);
    assert!(supported.contains(&"tool_choice"));
}

#[test]
fn gpt_oss_maps_through_its_narrowed_list() {
    let requested =
        object(r#"{"tools":[{"type":"function"}],"tool_choice":"auto","reasoning_effort":"high"}"#);
    let mut existing = Object::new();
    GptOssConfig::map_params(
        GPT_OSS_MODEL,
        &requested,
        &mut existing,
        &catalog(),
        Some(false),
    );
    assert_eq!(
        existing,
        object(r#"{"tools":[{"type":"function"}],"reasoning_effort":"high"}"#)
    );

    let mut funded = Object::new();
    GptOssConfig::map_params(
        GPT_OSS_FUNDED_MODEL,
        &requested,
        &mut funded,
        &catalog(),
        Some(true),
    );
    assert_eq!(
        funded,
        object(r#"{"tools":[{"type":"function"}],"tool_choice":"auto","reasoning_effort":"high"}"#)
    );
}

#[test]
fn llama_excludes_only_max_retries_for_a_publisher_model() {
    let supported = Llama3Config::supported_params(LLAMA_MODEL, &catalog());
    assert_eq!(supported.len(), 30);
    assert!(!supported.contains(&"max_retries"));
    assert!(supported.contains(&"store"));
    assert!(supported.contains(&"audio"));
}

#[test]
fn llama_excludes_the_ten_deployment_params_for_a_self_deployed_endpoint() {
    let supported = Llama3Config::supported_params(DEPLOYED_MODEL, &catalog());
    assert_eq!(supported.len(), 21);
    assert!(supported.contains(&"max_tokens"));
    assert!(supported.contains(&"tools"));
}

#[rstest]
#[case::audio("audio")]
#[case::max_retries("max_retries")]
#[case::modalities("modalities")]
#[case::prediction("prediction")]
#[case::prompt_cache_key("prompt_cache_key")]
#[case::prompt_cache_retention("prompt_cache_retention")]
#[case::safety_identifier("safety_identifier")]
#[case::service_tier("service_tier")]
#[case::store("store")]
#[case::web_search_options("web_search_options")]
fn llama_deployment_params_are_unsupported(#[case] param: &str) {
    assert!(
        !Llama3Config::supported_params(DEPLOYED_MODEL, &catalog()).contains(&param),
        "{param}"
    );
}

#[test]
fn llama_aliases_tokens_before_filtering_its_list() {
    let requested = object(r#"{"max_tokens":4,"max_completion_tokens":12,"max_retries":3}"#);
    let mut existing = Object::new();
    Llama3Config::map_params(LLAMA_MODEL, &requested, &mut existing, &catalog());
    assert_eq!(existing, object(r#"{"max_tokens":12}"#));

    let mut deployed = Object::new();
    Llama3Config::map_params(DEPLOYED_MODEL, &requested, &mut deployed, &catalog());
    assert_eq!(deployed, object(r#"{"max_tokens":12}"#));
}

#[test]
fn partner_class_tokens_default_to_none_and_keep_a_configured_value() {
    assert_eq!(Ai21Config::default().settings.max_tokens, None);
    assert_eq!(Llama3Config::default().settings.max_tokens, None);
    let configured = Llama3Config {
        settings: PartnerSettings {
            max_tokens: Some(512),
        },
    };
    assert_eq!(configured.settings.max_tokens, Some(512));
}

#[test]
fn mistral_is_the_only_partner_that_changes_its_provider() {
    assert_eq!(
        PartnerClass::Mistral.custom_llm_provider(),
        Some("vertex_ai")
    );
}

#[rstest]
#[case::ai21(PartnerClass::Ai21)]
#[case::gpt_oss(PartnerClass::GptOss)]
#[case::llama(PartnerClass::Llama3)]
fn other_partners_keep_the_inherited_provider(#[case] partner: PartnerClass) {
    assert_eq!(partner.custom_llm_provider(), None);
}

#[test]
fn llama_reply_defaults_missing_fields_and_keeps_an_upstream_identity() {
    let defaulted = decode_reply(
        &object(r#"{"choices":[{"index":0,"finish_reason":"stop","message":{"content":"hi"}}]}"#),
        LLAMA_MODEL,
        false,
        Uuid::nil,
    )
    .expect("the reply decodes");
    assert_eq!(defaulted.id, "");
    assert_eq!(defaulted.created, 0);
    assert_eq!(defaulted.model, LLAMA_MODEL);
    assert_eq!(defaulted.usage, None);

    let named = decode_reply(
        &object(
            r#"{"id":"reply-1","created":11,"model":"upstream","usage":{"prompt_tokens":3},"choices":[{"index":0,"finish_reason":"stop","message":{"content":"hi"}}]}"#,
        ),
        LLAMA_MODEL,
        false,
        Uuid::nil,
    )
    .expect("the reply decodes");
    assert_eq!(named.id, "reply-1");
    assert_eq!(named.created, 11);
    assert_eq!(named.model, "upstream");
    assert_eq!(named.usage, Some(object(r#"{"prompt_tokens":3}"#)));
}

#[test]
fn llama_reply_hands_the_choices_to_the_shared_transform() {
    let reply = decode_reply(
        &object(
            r#"{"choices":[{"index":0,"finish_reason":"stop","message":{"content":"hi","tool_calls":[{"id":"call-1","type":"function","function":{"name":"f","arguments":"{}"}}]}}]}"#,
        ),
        LLAMA_MODEL,
        false,
        Uuid::nil,
    )
    .expect("the reply decodes");

    assert_eq!(reply.choices.len(), 1);
    assert_eq!(reply.choices[0].index, 0);
    // a tool call wins over the received finish reason
    assert_eq!(reply.choices[0].finish_reason, "tool_calls");
    assert_eq!(reply.choices[0].message.role, "assistant");
    assert_eq!(reply.choices[0].message.content, Some(String::from("hi")));
}

#[test]
fn llama_stream_adds_the_missing_role_to_the_first_chunk_only() {
    let mut state = LlamaStreamState::default();
    let first = state.push(chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[{"index":0,"delta":{"content":"Hel"}}]}"#,
    ));
    assert_eq!(first.len(), 1);
    assert_eq!(
        to_value(&first[0]).expect("the chunk serializes"),
        json!({
            "id": "chunk-1",
            "object": "chat.completion.chunk",
            "created": 7,
            "model": "m",
            "choices": [{
                "index": 0,
                "finish_reason": null,
                "delta": {
                    "role": "assistant",
                    "content": "Hel",
                    "function_call": null,
                    "tool_calls": null,
                    "audio": null,
                },
                "logprobs": null,
            }],
        })
    );

    let later = state.push(chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[{"index":0,"delta":{"content":"lo"}}]}"#,
    ));
    assert_eq!(later.len(), 1);
    assert_eq!(later[0].choices[0].delta.role, None);
    assert_eq!(later[0].choices[0].delta.content.as_deref(), Some("lo"));
}

#[test]
fn llama_stream_splits_a_terminal_first_chunk_and_queues_its_terminal() {
    let mut state = LlamaStreamState::default();
    let chunks = state.push(chunk(
        r#"{"id":"chunk-9","created":7,"model":"m","choices":[{"index":1,"delta":{"role":null,"content":null},"finish_reason":"stop"}]}"#,
    ));
    assert_eq!(chunks.len(), 2);
    assert_eq!(
        to_value(&chunks[0]).expect("the chunk serializes"),
        json!({
            "id": "chunk-9",
            "object": "chat.completion.chunk",
            "created": 7,
            "model": "m",
            "choices": [{
                "index": 1,
                "finish_reason": null,
                "delta": {
                    "role": "assistant",
                    "content": "",
                    "function_call": null,
                    "tool_calls": null,
                    "audio": null,
                    "provider_specific_fields": {},
                },
                "logprobs": null,
            }],
        })
    );
    assert_eq!(
        to_value(&chunks[1]).expect("the chunk serializes"),
        json!({
            "id": "chunk-9",
            "object": "chat.completion.chunk",
            "created": 7,
            "model": "m",
            "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "delta": {
                    "role": null,
                    "content": null,
                    "function_call": null,
                    "tool_calls": null,
                    "audio": null,
                },
                "logprobs": null,
            }],
        })
    );

    let later = state.push(chunk(
        r#"{"id":"chunk-9","created":7,"model":"m","choices":[{"index":0,"delta":{"content":"x"}}]}"#,
    ));
    assert_eq!(later.len(), 1);
    assert_eq!(later[0].choices[0].delta.role, None);
}

#[rstest]
#[case::empty_string(r#"{"content":""}"#, Some(""))]
#[case::empty_delta("{}", None)]
fn llama_stream_gives_an_empty_initial_delta_provider_fields(
    #[case] delta: &str,
    #[case] content: Option<&str>,
) {
    let source = format!(
        r#"{{"id":"chunk-1","created":7,"model":"m","choices":[{{"index":0,"delta":{delta}}}]}}"#
    );
    let mut state = LlamaStreamState::default();
    let chunks = state.push(chunk(&source));
    assert_eq!(chunks.len(), 1);
    assert_eq!(
        chunks[0].choices[0].delta.role.as_deref(),
        Some("assistant")
    );
    assert_eq!(chunks[0].choices[0].delta.content.as_deref(), content);
    assert_eq!(
        chunks[0].choices[0]
            .delta
            .passthrough
            .get("provider_specific_fields"),
        Some(&json!({}))
    );
}

#[test]
fn llama_stream_keeps_a_present_provider_map_and_replaces_a_null_one() {
    let mut state = LlamaStreamState::default();
    let kept = state.push(chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[{"index":0,"delta":{"content":"","provider_specific_fields":{"source":"vertex"}}}]}"#,
    ));
    assert_eq!(
        kept[0].choices[0]
            .delta
            .passthrough
            .get("provider_specific_fields"),
        Some(&json!({"source": "vertex"}))
    );

    let mut other = LlamaStreamState::default();
    let replaced = other.push(chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[{"index":0,"delta":{"content":"","provider_specific_fields":null}}]}"#,
    ));
    assert_eq!(
        replaced[0].choices[0]
            .delta
            .passthrough
            .get("provider_specific_fields"),
        Some(&json!({}))
    );
}

#[test]
fn llama_stream_skips_a_chunk_without_choices() {
    let mut state = LlamaStreamState::default();
    let empty = state.push(chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[]}"#,
    ));
    assert!(empty[0].choices.is_empty());

    let next = state.push(chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[{"index":0,"delta":{"content":"x"}}]}"#,
    ));
    assert_eq!(next[0].choices[0].delta.role.as_deref(), Some("assistant"));
}

#[test]
fn llama_stream_repairs_only_the_first_choice() {
    let mut state = LlamaStreamState::default();
    let chunks = state.push(chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[{"index":0,"delta":{"content":"a"}},{"index":1,"delta":{"content":"b"}}]}"#,
    ));
    assert_eq!(chunks.len(), 1);
    assert_eq!(
        chunks[0].choices[0].delta.role.as_deref(),
        Some("assistant")
    );
    assert_eq!(chunks[0].choices[1].delta.role, None);
}
