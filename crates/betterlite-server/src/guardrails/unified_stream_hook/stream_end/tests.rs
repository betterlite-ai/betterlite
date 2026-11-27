#![cfg(test)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::future::{Future, ready};
use std::marker::PhantomData;
use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::llm_passthrough_dispatch::LlmPassthroughRouteHandler;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;

use super::StreamEndCheck as _;
use crate::guardrails::a2a_translation::adapter::A2ATranslation;
use crate::guardrails::bedrock_passthrough::event_stream::hook::PostCallSuccessHook;
use crate::guardrails::unified_chat::anthropic::AnthropicTranslation;
use crate::guardrails::unified_chat::chat::ChatTranslation;
use crate::guardrails::unified_chat::responses::ResponsesTranslation;
use crate::guardrails::unified_guardrail::other_adapters::RerankTranslation;
use crate::guardrails::unified_guardrail::other_adapters::google_genai::GoogleGenAiTranslation;
use crate::guardrails::unified_guardrail::other_adapters::mcp::McpTranslation;
use crate::guardrails::unified_guardrail::translation::UnifiedTranslation;
use crate::guardrails::unified_passthrough::endpoint::PassthroughEndpointTranslation;
use crate::guardrails::unified_simple::completion::CompletionTranslation;
use crate::guardrails::unified_simple::embeddings::EmbeddingsTranslation;
use crate::guardrails::unified_simple::image::ImageTranslation;
use crate::guardrails::unified_simple::speech::SpeechTranslation;
use crate::guardrails::unified_simple::transcription::TranscriptionTranslation;
use crate::guardrails::unified_simple::video::VideoTranslation;

struct NoGuardrail;

impl GuardrailContract for NoGuardrail {}

struct NoHook;

impl PostCallSuccessHook for NoHook {
    type Error = Infallible;

    fn post_call_success_hook(
        &self,
        _data: &BTreeMap<String, GuardrailValue>,
        _user_api_key_dict: &BTreeMap<String, GuardrailValue>,
        response: GuardrailValue,
    ) -> impl Future<Output = Result<GuardrailValue, Self::Error>> + Send {
        ready(Ok(response))
    }
}

type Probe = UnifiedTranslation<NoGuardrail, (), (), NoHook>;

const CHAT_OPEN: &str = r#"{"object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"a"},"finish_reason":null}]}"#;
const CHAT_DONE: &str = r#"{"object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"a"},"finish_reason":null},{"index":1,"delta":{},"finish_reason":"stop"}]}"#;
const RESPONSES_OPEN: &str = r#"{"type":"response.output_text.delta","delta":"a"}"#;
const RESPONSES_DONE: &str = r#"{"type":"response.completed","response":{"id":"resp_1"}}"#;
const ANTHROPIC_OPEN: &str = r#"{"type":"message_delta","delta":{"stop_reason":null}}"#;
const ANTHROPIC_DONE: &str = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#;
const ANTHROPIC_FRAME: &str = r#""event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n""#;
const ANTHROPIC_DATA_ONLY: &str =
    r#""data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n""#;
const ENDED_EVERYWHERE: &[&str] = &[CHAT_DONE, ANTHROPIC_DONE, ANTHROPIC_FRAME, RESPONSES_DONE];

fn parsed(items: &[&str]) -> Vec<StreamItem> {
    items
        .iter()
        .map(|text| serde_json::from_str(text).map(StreamItem::Json).unwrap())
        .collect()
}

fn responses() -> Probe {
    let diagnostics = Diagnostics {
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()),
        emit: Arc::new(|_event| {}),
    };
    let cache = DualLocalCache::new(
        None,
        None,
        &Settings::default(),
        Arc::new(|| 0.0_f64),
        diagnostics,
    );
    UnifiedTranslation::Responses(ResponsesTranslation {
        stamps: (),
        limits: ApiLimits::default(),
        cache: Arc::new(cache),
        _guardrail: PhantomData,
    })
}

fn chat() -> Probe {
    UnifiedTranslation::Chat(ChatTranslation {
        stamps: (),
        assembler: (),
        _guardrail: PhantomData,
    })
}

#[rstest]
#[case::chat_any_choice_finished(chat(), &[CHAT_OPEN, CHAT_DONE], true)]
#[case::chat_finished_before_the_last_item(chat(), &[CHAT_DONE, CHAT_OPEN], true)]
#[case::chat_still_open(chat(), &[CHAT_OPEN, CHAT_OPEN], false)]
#[case::chat_nothing_yet(chat(), &[], false)]
#[case::responses_terminal_last(responses(), &[RESPONSES_OPEN, RESPONSES_DONE], true)]
#[case::responses_terminal_not_last(responses(), &[RESPONSES_DONE, RESPONSES_OPEN], false)]
#[case::responses_nothing_yet(responses(), &[], false)]
#[case::anthropic_stop_reason_event(UnifiedTranslation::Anthropic(AnthropicTranslation { stamps: (), _guardrail: PhantomData }), &[ANTHROPIC_OPEN, ANTHROPIC_DONE], true)]
#[case::anthropic_text_frame_is_not_read(UnifiedTranslation::Anthropic(AnthropicTranslation { stamps: (), _guardrail: PhantomData }), &[ANTHROPIC_OPEN, ANTHROPIC_FRAME], false)]
#[case::anthropic_data_line_without_event(UnifiedTranslation::Anthropic(AnthropicTranslation { stamps: (), _guardrail: PhantomData }), &[ANTHROPIC_DATA_ONLY], false)]
#[case::anthropic_null_stop_reason(UnifiedTranslation::Anthropic(AnthropicTranslation { stamps: (), _guardrail: PhantomData }), &[ANTHROPIC_OPEN, CHAT_DONE], false)]
#[case::completion(
    UnifiedTranslation::Completion(CompletionTranslation::default()),
    ENDED_EVERYWHERE,
    false
)]
#[case::embeddings(
    UnifiedTranslation::Embeddings(EmbeddingsTranslation::default()),
    ENDED_EVERYWHERE,
    false
)]
#[case::image(
    UnifiedTranslation::Image(ImageTranslation::default()),
    ENDED_EVERYWHERE,
    false
)]
#[case::speech(
    UnifiedTranslation::Speech(SpeechTranslation::default()),
    ENDED_EVERYWHERE,
    false
)]
#[case::transcription(
    UnifiedTranslation::Transcription(TranscriptionTranslation::default()),
    ENDED_EVERYWHERE,
    false
)]
#[case::video(
    UnifiedTranslation::Video(VideoTranslation::default()),
    ENDED_EVERYWHERE,
    false
)]
#[case::a2a(UnifiedTranslation::A2a(A2ATranslation { guardrail: PhantomData }), ENDED_EVERYWHERE, false)]
#[case::rerank(
    UnifiedTranslation::Rerank(RerankTranslation(PhantomData)),
    ENDED_EVERYWHERE,
    false
)]
#[case::google_genai(
    UnifiedTranslation::GoogleGenAi(GoogleGenAiTranslation(PhantomData)),
    ENDED_EVERYWHERE,
    false
)]
#[case::mcp(
    UnifiedTranslation::Mcp(McpTranslation(PhantomData)),
    ENDED_EVERYWHERE,
    false
)]
#[case::endpoint(
    UnifiedTranslation::Endpoint(PassthroughEndpointTranslation::new(None, None)),
    ENDED_EVERYWHERE,
    false
)]
#[case::llm_route(
    UnifiedTranslation::LlmRoute(LlmPassthroughRouteHandler::new(Vec::new())),
    ENDED_EVERYWHERE,
    false
)]
fn each_variant_reports_its_own_stream_end(
    #[case] probe: Probe,
    #[case] items: &[&str],
    #[case] ended: bool,
) {
    assert_eq!(probe.streaming_has_ended(&parsed(items)), ended);
}

#[test]
fn frame_bytes_end_the_anthropic_stream() {
    let probe = UnifiedTranslation::Anthropic(AnthropicTranslation {
        stamps: (),
        _guardrail: PhantomData,
    });
    let frame = String::from(
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
    );
    assert!(probe.streaming_has_ended(&[StreamItem::Bytes(frame.into_bytes())]));
}

#[test]
fn frame_text_does_not_end_the_anthropic_stream() {
    let probe = UnifiedTranslation::Anthropic(AnthropicTranslation {
        stamps: (),
        _guardrail: PhantomData,
    });
    let frame = String::from(
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
    );
    assert!(!probe.streaming_has_ended(&[StreamItem::Json(GuardrailValue::Text(frame))]));
}
