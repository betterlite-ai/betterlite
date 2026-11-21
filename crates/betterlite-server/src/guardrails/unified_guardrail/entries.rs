use std::marker::PhantomData;
use std::sync::Arc;

use betterlite_context::local_ttl_cache::dual::DualLocalCache;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::routes::CallType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::llm_passthrough_dispatch::LlmPassthroughRouteHandler;
use betterlite_guardrails::unified_dispatch::registry::{TranslationEntry, TranslationRegistry};
use betterlite_models::decisions::Json;

use super::other_adapters::{
    GOOGLE_GENAI_CALL_TYPES, GoogleGenAiTranslation, MCP_CALL_TYPES, McpTranslation,
    RERANK_CALL_TYPES, RerankTranslation,
};
use super::translation::UnifiedTranslation;
use crate::guardrails::a2a_translation::adapter::{A2A_CALL_TYPES, A2ATranslation};
use crate::guardrails::bedrock_passthrough::event_stream::hook::PostCallSuccessHook;
use crate::guardrails::bedrock_passthrough::text_paths::SkipFlags;
use crate::guardrails::unified_chat::anthropic::AnthropicTranslation;
use crate::guardrails::unified_chat::chat::ChatTranslation;
use crate::guardrails::unified_chat::responses::ResponsesTranslation;
use crate::guardrails::unified_chat::tables::{
    ANTHROPIC_CALL_TYPES, CHAT_CALL_TYPES, CHAT_COMPLETIONS_CALL_TYPES, RESPONSES_CALL_TYPES,
};
use crate::guardrails::unified_passthrough::bedrock::BedrockPassthroughTranslation;
use crate::guardrails::unified_passthrough::endpoint::PassthroughEndpointTranslation;
use crate::guardrails::unified_passthrough::kinds::{
    PASSTHROUGH_TABLE, PassthroughKind, passthrough_provider_list,
};
use crate::guardrails::unified_simple::completion::CompletionTranslation;
use crate::guardrails::unified_simple::embeddings::EmbeddingsTranslation;
use crate::guardrails::unified_simple::image::ImageTranslation;
use crate::guardrails::unified_simple::speech::SpeechTranslation;
use crate::guardrails::unified_simple::tables::{
    COMPLETION_CALL_TYPES, EMBEDDINGS_CALL_TYPES, IMAGE_CALL_TYPES, SPEECH_CALL_TYPES,
    TRANSCRIPTION_CALL_TYPES, VIDEO_CALL_TYPES,
};
use crate::guardrails::unified_simple::transcription::TranscriptionTranslation;
use crate::guardrails::unified_simple::video::VideoTranslation;

type Entries<G, S, A, H> = Vec<TranslationEntry<UnifiedTranslation<G, S, A, H>>>;

// What the adapters are built from; nothing here is read from config
pub struct RegistryConfig<S, A, H> {
    pub stamps: S,
    pub assembler: A,
    pub limits: ApiLimits,
    pub cache: Arc<DualLocalCache>,
    pub hook: H,
    pub bedrock_skip: SkipFlags,
    pub guardrail_name: Option<String>,
    pub passthrough_guardrails_config: Option<Json>,
}

fn push_table<T, F>(entries: &mut Vec<TranslationEntry<T>>, call_types: &[CallType], make: F)
where
    F: Fn() -> T,
{
    for call_type in call_types {
        entries.push(TranslationEntry {
            call_type: *call_type,
            translation: make(),
            chat_completions: CHAT_COMPLETIONS_CALL_TYPES.contains(call_type),
        });
    }
}

#[must_use]
pub fn build_registry<G, S, A, H>(
    config: &RegistryConfig<S, A, H>,
) -> TranslationRegistry<UnifiedTranslation<G, S, A, H>>
where
    G: GuardrailContract + Send + Sync,
    S: Clone,
    A: Clone,
    H: PostCallSuccessHook + Sync + Clone,
{
    let mut entries = Vec::new();
    push_simple_tables(&mut entries);
    push_chat_tables(&mut entries, config);
    push_passthrough_table(&mut entries, config);
    push_table(&mut entries, &A2A_CALL_TYPES, || {
        UnifiedTranslation::A2a(A2ATranslation {
            guardrail: PhantomData,
        })
    });
    push_table(&mut entries, &RERANK_CALL_TYPES, || {
        UnifiedTranslation::Rerank(RerankTranslation(PhantomData))
    });
    push_table(&mut entries, &GOOGLE_GENAI_CALL_TYPES, || {
        UnifiedTranslation::GoogleGenAi(GoogleGenAiTranslation(PhantomData))
    });
    push_table(&mut entries, &MCP_CALL_TYPES, || {
        UnifiedTranslation::Mcp(McpTranslation(PhantomData))
    });
    TranslationRegistry { entries }
}

fn push_simple_tables<G, S, A, H>(entries: &mut Entries<G, S, A, H>)
where
    G: GuardrailContract + Send + Sync,
    H: PostCallSuccessHook + Sync,
{
    push_table(entries, &COMPLETION_CALL_TYPES, || {
        UnifiedTranslation::Completion(CompletionTranslation::default())
    });
    push_table(entries, &EMBEDDINGS_CALL_TYPES, || {
        UnifiedTranslation::Embeddings(EmbeddingsTranslation::default())
    });
    push_table(entries, &IMAGE_CALL_TYPES, || {
        UnifiedTranslation::Image(ImageTranslation::default())
    });
    push_table(entries, &SPEECH_CALL_TYPES, || {
        UnifiedTranslation::Speech(SpeechTranslation::default())
    });
    push_table(entries, &TRANSCRIPTION_CALL_TYPES, || {
        UnifiedTranslation::Transcription(TranscriptionTranslation::default())
    });
    push_table(entries, &VIDEO_CALL_TYPES, || {
        UnifiedTranslation::Video(VideoTranslation::default())
    });
}

fn push_chat_tables<G, S, A, H>(entries: &mut Entries<G, S, A, H>, config: &RegistryConfig<S, A, H>)
where
    G: GuardrailContract + Send + Sync,
    S: Clone,
    A: Clone,
    H: PostCallSuccessHook + Sync,
{
    push_table(entries, &CHAT_CALL_TYPES, || {
        UnifiedTranslation::Chat(ChatTranslation {
            stamps: config.stamps.clone(),
            assembler: config.assembler.clone(),
            _guardrail: PhantomData,
        })
    });
    push_table(entries, &RESPONSES_CALL_TYPES, || {
        UnifiedTranslation::Responses(ResponsesTranslation {
            stamps: config.stamps.clone(),
            limits: config.limits,
            cache: Arc::clone(&config.cache),
            _guardrail: PhantomData,
        })
    });
    push_table(entries, &ANTHROPIC_CALL_TYPES, || {
        UnifiedTranslation::Anthropic(AnthropicTranslation {
            stamps: config.stamps.clone(),
            _guardrail: PhantomData,
        })
    });
}

// the sync llm_passthrough_route call type has no row in B6162's table, so it stays unmapped
fn push_passthrough_table<G, S, A, H>(
    entries: &mut Entries<G, S, A, H>,
    config: &RegistryConfig<S, A, H>,
) where
    G: GuardrailContract + Send + Sync,
    H: PostCallSuccessHook + Sync + Clone,
{
    for (call_type, kind) in PASSTHROUGH_TABLE {
        let translation = match kind {
            PassthroughKind::Endpoint => {
                UnifiedTranslation::Endpoint(PassthroughEndpointTranslation::new(
                    config.guardrail_name.clone(),
                    config.passthrough_guardrails_config.clone(),
                ))
            }
            PassthroughKind::LlmRoute => {
                let bedrock = BedrockPassthroughTranslation::new(
                    config.hook.clone(),
                    config.bedrock_skip,
                    config.guardrail_name.clone(),
                    config.passthrough_guardrails_config.clone(),
                );
                UnifiedTranslation::LlmRoute(LlmPassthroughRouteHandler::new(
                    passthrough_provider_list(bedrock),
                ))
            }
        };
        entries.push(TranslationEntry {
            call_type,
            translation,
            chat_completions: false,
        });
    }
}

mod tests;
