use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailMessage;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::guardrail_translation::{
    BaseTranslation, StreamingScanKey, TranslationCall,
};
use betterlite_guardrails::llm_passthrough_dispatch::LlmPassthroughRouteHandler;

use super::failures::AdapterFailure;
use super::other_adapters::RerankTranslation;
use super::other_adapters::google_genai::GoogleGenAiTranslation;
use super::other_adapters::mcp::McpTranslation;
use crate::guardrails::a2a_translation::adapter::A2ATranslation;
use crate::guardrails::bedrock_passthrough::event_stream::hook::PostCallSuccessHook;
use crate::guardrails::chat_guardrail_translation::rewrites::rebuild_ended_stream::ChunkAssembler;
use crate::guardrails::unified_chat::GuardrailToApply;
use crate::guardrails::unified_chat::anthropic::AnthropicTranslation;
use crate::guardrails::unified_chat::chat::ChatTranslation;
use crate::guardrails::unified_chat::responses::ResponsesTranslation;
use crate::guardrails::unified_chat::stamps::StampSource;
use crate::guardrails::unified_passthrough::bedrock::BedrockPassthroughTranslation;
use crate::guardrails::unified_passthrough::endpoint::PassthroughEndpointTranslation;
use crate::guardrails::unified_simple::completion::CompletionTranslation;
use crate::guardrails::unified_simple::embeddings::EmbeddingsTranslation;
use crate::guardrails::unified_simple::image::ImageTranslation;
use crate::guardrails::unified_simple::speech::SpeechTranslation;
use crate::guardrails::unified_simple::transcription::TranscriptionTranslation;
use crate::guardrails::unified_simple::video::VideoTranslation;

type Applied<G> = GuardrailToApply<G>;

// betterlite-guardrails cannot name these adapters, so the registry's T lives here
pub enum UnifiedTranslation<G, S, A, H>
where
    G: GuardrailContract + Send + Sync,
    H: PostCallSuccessHook + Sync,
{
    Completion(CompletionTranslation<Applied<G>>),
    Embeddings(EmbeddingsTranslation<Applied<G>>),
    Image(ImageTranslation<Applied<G>>),
    Speech(SpeechTranslation<Applied<G>>),
    Transcription(TranscriptionTranslation<Applied<G>>),
    Video(VideoTranslation<Applied<G>>),
    Chat(ChatTranslation<G, S, A>),
    Responses(ResponsesTranslation<G, S>),
    Anthropic(AnthropicTranslation<G, S>),
    Endpoint(PassthroughEndpointTranslation<Applied<G>>),
    LlmRoute(LlmPassthroughRouteHandler<BedrockPassthroughTranslation<Applied<G>, H>>),
    A2a(A2ATranslation<Applied<G>>),
    Rerank(RerankTranslation<Applied<G>>),
    GoogleGenAi(GoogleGenAiTranslation<Applied<G>>),
    Mcp(McpTranslation<Applied<G>>),
}

macro_rules! each_variant {
    ($translation:expr, $inner:ident => $body:expr) => {
        match $translation {
            UnifiedTranslation::Completion($inner) => $body,
            UnifiedTranslation::Embeddings($inner) => $body,
            UnifiedTranslation::Image($inner) => $body,
            UnifiedTranslation::Speech($inner) => $body,
            UnifiedTranslation::Transcription($inner) => $body,
            UnifiedTranslation::Video($inner) => $body,
            UnifiedTranslation::Chat($inner) => $body,
            UnifiedTranslation::Responses($inner) => $body,
            UnifiedTranslation::Anthropic($inner) => $body,
            UnifiedTranslation::Endpoint($inner) => $body,
            UnifiedTranslation::LlmRoute($inner) => $body,
            UnifiedTranslation::A2a($inner) => $body,
            UnifiedTranslation::Rerank($inner) => $body,
            UnifiedTranslation::GoogleGenAi($inner) => $body,
            UnifiedTranslation::Mcp($inner) => $body,
        }
    };
}

// each fallible async method calls the held adapter and brings its failure to GuardrailFailure
macro_rules! forward_fallible {
    ($method:ident($($arg:ident: $ty:ty),+ $(,)?) -> $output:ty) => {
        async fn $method(&self, $($arg: $ty),+) -> Result<$output, Self::Failure> {
            each_variant!(self, inner => inner.$method($($arg),+).await.map_err(AdapterFailure::into_failure))
        }
    };
}

// LiteLLM's two class flags are consts of each adapter type, so the enum reads them per held variant
pub trait VariantFlags {
    fn delivers_ended_stream_rewrites(&self) -> bool;
    fn assembles_streamed_response(&self) -> bool;
}

trait ClassFlags {
    fn class_flags(&self) -> [bool; 2];
}

impl<T: BaseTranslation> ClassFlags for T {
    fn class_flags(&self) -> [bool; 2] {
        [
            T::DELIVERS_ENDED_STREAM_REWRITES,
            T::ASSEMBLES_STREAMED_RESPONSE,
        ]
    }
}

impl<G, S, A, H> VariantFlags for UnifiedTranslation<G, S, A, H>
where
    G: GuardrailContract + Send + Sync,
    S: StampSource + Send + Sync,
    A: ChunkAssembler + Send + Sync,
    H: PostCallSuccessHook + Sync,
{
    fn delivers_ended_stream_rewrites(&self) -> bool {
        let [delivers, _assembles] = each_variant!(self, inner => inner.class_flags());
        delivers
    }

    fn assembles_streamed_response(&self) -> bool {
        let [_delivers, assembles] = each_variant!(self, inner => inner.class_flags());
        assembles
    }
}

impl<G, S, A, H> BaseTranslation for UnifiedTranslation<G, S, A, H>
where
    G: GuardrailContract + Send + Sync,
    S: StampSource + Send + Sync,
    A: ChunkAssembler + Send + Sync,
    H: PostCallSuccessHook + Sync,
{
    type Guardrail = Applied<G>;
    type LogTarget = GuardrailLogTarget;
    type Failure = GuardrailFailure;

    fn post_call_hook_response(&self, response: GuardrailValue) -> GuardrailValue {
        each_variant!(self, inner => inner.post_call_hook_response(response))
    }

    forward_fallible!(process_input_messages(
        data: &mut BTreeMap<String, GuardrailValue>,
        guardrail: &Self::Guardrail,
        logging_obj: Option<&mut Self::LogTarget>,
    ) -> ());

    forward_fallible!(process_output_response(
        response: GuardrailValue,
        guardrail: &Self::Guardrail,
        call: TranslationCall<'_, Self::LogTarget>,
    ) -> GuardrailValue);

    forward_fallible!(process_streaming_response(
        responses_so_far: &mut Vec<StreamItem>,
        guardrail: &Self::Guardrail,
        call: TranslationCall<'_, Self::LogTarget>,
    ) -> ());

    fn get_streaming_scan_key(&self, responses_so_far: &[StreamItem]) -> Option<StreamingScanKey> {
        each_variant!(self, inner => inner.get_streaming_scan_key(responses_so_far))
    }

    fn build_block_sse_chunks(
        &self,
        failure: &Self::Failure,
        stream_started: bool,
        responses_so_far: Option<&[StreamItem]>,
    ) -> Option<Vec<Vec<u8>>> {
        each_variant!(self, inner => inner.build_block_sse_chunks(&AdapterFailure::from_failure(failure.clone()), stream_started, responses_so_far))
    }

    fn build_stream_error_items(
        &self,
        failure: &Self::Failure,
        responses_so_far: Option<&[StreamItem]>,
    ) -> Option<Vec<StreamItem>> {
        each_variant!(self, inner => inner.build_stream_error_items(&AdapterFailure::from_failure(failure.clone()), responses_so_far))
    }

    fn get_structured_messages(
        &self,
        data: &BTreeMap<String, GuardrailValue>,
    ) -> Option<Vec<GuardrailMessage>> {
        each_variant!(self, inner => inner.get_structured_messages(data))
    }

    fn extract_request_tool_names(&self, data: &BTreeMap<String, GuardrailValue>) -> Vec<String> {
        each_variant!(self, inner => inner.extract_request_tool_names(data))
    }
}

mod tests;
