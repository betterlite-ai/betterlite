#![cfg(test)]

mod partner_max_tokens_settings;

use std::collections::BTreeSet;

use betterlite_openai::base_helpers::provider_config;
use betterlite_openai::chat_choices::stream::chunk_parser;
use betterlite_openai::chat_choices::wire::{StreamDelta, StreamReply};
use betterlite_openai::openai_base_config::{CatalogCapabilities, Object};
use rstest::rstest;
use serde_json::from_str;
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

// LiteLLM needs an empty map here so the stream wrapper keeps an empty chunk instead of dropping it
fn provider_fields_are_empty(delta: &StreamDelta) -> bool {
    delta
        .passthrough
        .get("provider_specific_fields")
        .and_then(|value| value.as_object())
        .is_some_and(Object::is_empty)
}

fn provider_source(delta: &StreamDelta) -> Option<&str> {
    delta
        .passthrough
        .get("provider_specific_fields")
        .and_then(|value| value.as_object())
        .and_then(|fields| fields.get("source"))
        .and_then(|value| value.as_str())
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

    let mut prior = object(r#"{"max_tokens":1}"#);
    Ai21Config::map_params(
        AI21_MODEL,
        &object(r#"{"max_tokens":8}"#),
        &mut prior,
        &catalog(),
    );
    assert_eq!(prior, object(r#"{"max_tokens":8}"#));
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
    let default = provider_config(&Ai21Config::default().settings)
        .expect("the default partner settings serialize");
    assert!(default.is_empty(), "{default:?}");
    let configured = Llama3Config {
        settings: PartnerSettings {
            max_tokens: Some(512),
        },
    };
    let snapshot =
        provider_config(&configured.settings).expect("the configured partner settings serialize");
    let limit = snapshot
        .get("max_tokens")
        .expect("the snapshot carries max_tokens");
    assert_eq!(limit.as_i64(), Some(512));
    assert_eq!(snapshot.len(), 1);
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
fn llama_stream_adds_the_missing_role_to_the_first_chunk() {
    let mut state = LlamaStreamState::default();
    let first = state.push(chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[{"index":0,"delta":{"content":"first"}}]}"#,
    ));
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].id, "chunk-1");
    assert_eq!(first[0].created, 7);
    assert_eq!(first[0].model.as_deref(), Some("m"));
    let choice = &first[0].choices[0];
    assert_eq!(choice.index, 0);
    assert_eq!(choice.finish_reason, None);
    assert_eq!(choice.logprobs, None);
    assert_eq!(choice.delta.role.as_deref(), Some("assistant"));
    assert_eq!(choice.delta.content.as_deref(), Some("first"));
    assert_eq!(choice.delta.function_call, None);
    assert_eq!(choice.delta.tool_calls, None);
    assert_eq!(choice.delta.audio, None);
    assert!(choice.delta.passthrough.is_empty());
}

#[test]
fn llama_stream_leaves_a_later_chunk_without_the_role() {
    let mut state = LlamaStreamState::default();
    state.push(chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[{"index":0,"delta":{"content":"first"}}]}"#,
    ));
    let later = state.push(chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[{"index":0,"delta":{"content":"second"}}]}"#,
    ));
    assert_eq!(later.len(), 1);
    assert_eq!(later[0].object, "chat.completion.chunk");
    assert_eq!(later[0].choices[0].delta.role, None);
    assert_eq!(later[0].choices[0].delta.content.as_deref(), Some("second"));
}

// The first chunk is also terminal, so the push returns the repaired chunk and the queued terminal.
fn split_terminal_first_chunk() -> Vec<StreamReply> {
    let mut state = LlamaStreamState::default();
    state.push(chunk(
        r#"{"id":"chunk-9","created":7,"model":"m","choices":[{"index":1,"delta":{"role":null,"content":null},"finish_reason":"stop"}]}"#,
    ))
}

#[test]
fn llama_stream_splits_a_terminal_first_chunk() {
    let chunks = split_terminal_first_chunk();
    assert_eq!(chunks.len(), 2);
    let repaired = &chunks[0];
    assert_eq!(repaired.id, "chunk-9");
    assert_eq!(repaired.created, 7);
    assert_eq!(repaired.model.as_deref(), Some("m"));
    let first = &repaired.choices[0];
    assert_eq!(first.index, 1);
    assert_eq!(first.finish_reason, None);
    assert_eq!(first.delta.role.as_deref(), Some("assistant"));
    assert_eq!(first.delta.content.as_deref(), Some(""));
    assert!(provider_fields_are_empty(&first.delta));
}

#[test]
fn llama_stream_queues_the_terminal_chunk_of_a_split() {
    let chunks = split_terminal_first_chunk();
    let queued = &chunks[1];
    assert_eq!(queued.id, "chunk-9");
    assert_eq!(queued.object, "chat.completion.chunk");
    assert_eq!(queued.created, 7);
    assert_eq!(queued.model.as_deref(), Some("m"));
    assert_eq!(queued.usage, None);
    assert_eq!(queued.service_tier, None);
    let terminal = &queued.choices[0];
    assert_eq!(terminal.index, 0);
    assert_eq!(terminal.finish_reason.as_deref(), Some("stop"));
    assert_eq!(terminal.delta.role, None);
    assert_eq!(terminal.delta.content, None);
    assert!(terminal.delta.passthrough.is_empty());
}

#[test]
fn llama_stream_keeps_the_content_of_a_terminal_first_chunk() {
    let mut state = LlamaStreamState::default();
    let chunks = state.push(chunk(
        r#"{"id":"chunk-3","created":7,"model":"m","choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":"stop"}]}"#,
    ));
    assert_eq!(chunks.len(), 2);
    let repaired = &chunks[0].choices[0];
    assert_eq!(repaired.finish_reason, None);
    assert_eq!(repaired.delta.role.as_deref(), Some("assistant"));
    assert_eq!(repaired.delta.content.as_deref(), Some("hi"));
    assert!(provider_fields_are_empty(&repaired.delta));

    let terminal = &chunks[1].choices[0];
    assert_eq!(terminal.index, 0);
    assert_eq!(terminal.finish_reason.as_deref(), Some("stop"));
    assert_eq!(terminal.delta.content, None);
}

#[test]
fn llama_stream_leaves_a_later_terminal_chunk_whole() {
    let mut state = LlamaStreamState::default();
    let first = state.push(chunk(
        r#"{"id":"chunk-4","created":7,"model":"m","choices":[{"index":0,"delta":{"content":"a"}}]}"#,
    ));
    assert_eq!(first.len(), 1);

    let terminal = state.push(chunk(
        r#"{"id":"chunk-4","created":7,"model":"m","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
    ));
    assert_eq!(terminal.len(), 1);
    assert_eq!(
        terminal[0].choices[0].finish_reason.as_deref(),
        Some("stop")
    );
    assert_eq!(terminal[0].choices[0].delta.role, None);
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
    assert!(provider_fields_are_empty(&chunks[0].choices[0].delta));
}

#[test]
fn llama_stream_keeps_a_present_provider_map_and_replaces_a_null_one() {
    let mut state = LlamaStreamState::default();
    let kept = state.push(chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[{"index":0,"delta":{"content":"","provider_specific_fields":{"source":"vertex"}}}]}"#,
    ));
    assert_eq!(provider_source(&kept[0].choices[0].delta), Some("vertex"));

    let mut other = LlamaStreamState::default();
    let replaced = other.push(chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[{"index":0,"delta":{"content":"","provider_specific_fields":null}}]}"#,
    ));
    assert!(provider_fields_are_empty(&replaced[0].choices[0].delta));
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
