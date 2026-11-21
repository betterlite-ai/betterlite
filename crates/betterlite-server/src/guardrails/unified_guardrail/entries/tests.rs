#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_core::routes::CallType;
use rstest::rstest;

use crate::guardrails::a2a_translation::adapter::A2A_CALL_TYPES;
use crate::guardrails::unified_chat::tables::{
    ANTHROPIC_CALL_TYPES, CHAT_CALL_TYPES, RESPONSES_CALL_TYPES,
};
use crate::guardrails::unified_guardrail::other_adapters::{
    GOOGLE_GENAI_CALL_TYPES, MCP_CALL_TYPES, RERANK_CALL_TYPES,
};
use crate::guardrails::unified_guardrail::tests::support::{FakeGuardrail, registry};
use crate::guardrails::unified_guardrail::translation::UnifiedTranslation;
use crate::guardrails::unified_passthrough::kinds::PASSTHROUGH_TABLE;
use crate::guardrails::unified_simple::tables::{
    COMPLETION_CALL_TYPES, EMBEDDINGS_CALL_TYPES, IMAGE_CALL_TYPES, SPEECH_CALL_TYPES,
    TRANSCRIPTION_CALL_TYPES, VIDEO_CALL_TYPES,
};

// the 14 guardrail_translation_mappings dicts at the pin
fn pinned_tables() -> Vec<(&'static str, Vec<CallType>)> {
    let passthrough = PASSTHROUGH_TABLE
        .iter()
        .map(|(call_type, _kind)| *call_type)
        .collect();
    vec![
        ("chat", CHAT_CALL_TYPES.to_vec()),
        ("responses", RESPONSES_CALL_TYPES.to_vec()),
        ("anthropic", ANTHROPIC_CALL_TYPES.to_vec()),
        ("text_completion", COMPLETION_CALL_TYPES.to_vec()),
        ("embeddings", EMBEDDINGS_CALL_TYPES.to_vec()),
        ("image", IMAGE_CALL_TYPES.to_vec()),
        ("speech", SPEECH_CALL_TYPES.to_vec()),
        ("transcription", TRANSCRIPTION_CALL_TYPES.to_vec()),
        ("videos", VIDEO_CALL_TYPES.to_vec()),
        ("pass_through", passthrough),
        ("rerank", RERANK_CALL_TYPES.to_vec()),
        ("google_genai", GOOGLE_GENAI_CALL_TYPES.to_vec()),
        ("a2a", A2A_CALL_TYPES.to_vec()),
        ("mcp", MCP_CALL_TYPES.to_vec()),
    ]
}

#[test]
fn every_pinned_table_is_registered_once() {
    let built = registry::<FakeGuardrail>();
    let tables = pinned_tables();
    assert_eq!(tables.len(), 14);
    let expected: Vec<CallType> = tables
        .into_iter()
        .flat_map(|(_name, call_types)| call_types)
        .collect();
    let registered: Vec<CallType> = built.entries.iter().map(|entry| entry.call_type).collect();
    assert_eq!(registered.len(), expected.len());
    let unique: BTreeSet<String> = registered
        .iter()
        .map(|call_type| format!("{call_type:?}"))
        .collect();
    assert_eq!(
        unique.len(),
        registered.len(),
        "a call type is registered twice"
    );
    let missing: Vec<CallType> = expected
        .into_iter()
        .filter(|call_type| !registered.contains(call_type))
        .collect();
    assert!(missing.is_empty(), "not registered: {missing:?}");
}

#[rstest]
#[case::completion(CallType::Completion, true)]
#[case::acompletion(CallType::Acompletion, true)]
#[case::text_completion(CallType::TextCompletion, false)]
#[case::responses(CallType::Responses, false)]
#[case::anthropic(CallType::AnthropicMessages, false)]
#[case::pass_through(CallType::PassThroughEndpoint, false)]
#[case::llm_route(CallType::AllmPassthroughRoute, false)]
#[case::mcp(CallType::CallMcpTool, false)]
fn only_chat_entries_carry_the_chat_completions_flag(
    #[case] call_type: CallType,
    #[case] flagged: bool,
) {
    let built = registry::<FakeGuardrail>();
    assert_eq!(built.get(call_type).unwrap().chat_completions, flagged);
}

#[rstest]
#[case::completion(CallType::TextCompletion, "Completion")]
#[case::embeddings(CallType::Aembedding, "Embeddings")]
#[case::image(CallType::ImageGeneration, "Image")]
#[case::speech(CallType::Aspeech, "Speech")]
#[case::transcription(CallType::Transcription, "Transcription")]
#[case::video(CallType::AvideoEdit, "Video")]
#[case::chat(CallType::Acompletion, "Chat")]
#[case::responses(CallType::Aresponses, "Responses")]
#[case::anthropic(CallType::AnthropicMessages, "Anthropic")]
#[case::endpoint(CallType::PassThroughEndpoint, "Endpoint")]
#[case::llm_route(CallType::AllmPassthroughRoute, "LlmRoute")]
#[case::a2a(CallType::SendMessage, "A2a")]
#[case::rerank(CallType::Arerank, "Rerank")]
#[case::google_genai(CallType::GenerateContentStream, "GoogleGenAi")]
#[case::mcp(CallType::ListMcpTools, "Mcp")]
fn each_call_type_holds_its_adapter(#[case] call_type: CallType, #[case] variant: &str) {
    let built = registry::<FakeGuardrail>();
    let held = match &built.get(call_type).unwrap().translation {
        UnifiedTranslation::Completion(_) => "Completion",
        UnifiedTranslation::Embeddings(_) => "Embeddings",
        UnifiedTranslation::Image(_) => "Image",
        UnifiedTranslation::Speech(_) => "Speech",
        UnifiedTranslation::Transcription(_) => "Transcription",
        UnifiedTranslation::Video(_) => "Video",
        UnifiedTranslation::Chat(_) => "Chat",
        UnifiedTranslation::Responses(_) => "Responses",
        UnifiedTranslation::Anthropic(_) => "Anthropic",
        UnifiedTranslation::Endpoint(_) => "Endpoint",
        UnifiedTranslation::LlmRoute(_) => "LlmRoute",
        UnifiedTranslation::A2a(_) => "A2a",
        UnifiedTranslation::Rerank(_) => "Rerank",
        UnifiedTranslation::GoogleGenAi(_) => "GoogleGenAi",
        UnifiedTranslation::Mcp(_) => "Mcp",
    };
    assert_eq!(held, variant);
}

#[rstest]
#[case::sync_llm_route(CallType::LlmPassthroughRoute)]
#[case::ocr(CallType::Ocr)]
#[case::aocr(CallType::Aocr)]
fn unmapped_call_types_have_no_entry(#[case] call_type: CallType) {
    assert!(registry::<FakeGuardrail>().get(call_type).is_none());
}
