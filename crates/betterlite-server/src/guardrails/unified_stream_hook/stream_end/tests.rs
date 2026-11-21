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
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;

use super::StreamEndCheck;
use crate::guardrails::a2a_translation::adapter::A2ATranslation;
use crate::guardrails::bedrock_passthrough::event_stream::hook::PostCallSuccessHook;
use crate::guardrails::unified_chat::anthropic::AnthropicTranslation;
use crate::guardrails::unified_chat::chat::ChatTranslation;
use crate::guardrails::unified_chat::responses::ResponsesTranslation;
use crate::guardrails::unified_guardrail::other_adapters::RerankTranslation;
use crate::guardrails::unified_guardrail::other_adapters::google_genai::GoogleGenAiTranslation;
use crate::guardrails::unified_guardrail::other_adapters::mcp::McpTranslation;
use crate::guardrails::unified_guardrail::translation::UnifiedTranslation;
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

fn parsed(items: &[&str]) -> Vec<GuardrailValue> {
    items
        .iter()
        .map(|text| serde_json::from_str(text).unwrap())
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

#[rstest]
#[case::any_choice_finished(&[CHAT_OPEN, CHAT_DONE], true)]
#[case::still_open(&[CHAT_OPEN, CHAT_OPEN], false)]
#[case::nothing_yet(&[], false)]
fn chat_ends_once_a_choice_has_a_finish_reason(#[case] items: &[&str], #[case] ended: bool) {
    let probe: Probe = UnifiedTranslation::Chat(ChatTranslation {
        stamps: (),
        assembler: (),
        _guardrail: PhantomData,
    });
    assert_eq!(probe.streaming_has_ended(&parsed(items)), ended);
}

#[rstest]
#[case::terminal_last(&[RESPONSES_OPEN, RESPONSES_DONE], true)]
#[case::terminal_not_last(&[RESPONSES_DONE, RESPONSES_OPEN], false)]
#[case::nothing_yet(&[], false)]
fn responses_ends_on_a_terminal_last_event(#[case] items: &[&str], #[case] ended: bool) {
    assert_eq!(responses().streaming_has_ended(&parsed(items)), ended);
}

#[rstest]
#[case::stop_reason_event(&[ANTHROPIC_OPEN, ANTHROPIC_DONE], true)]
#[case::sse_frame(&[ANTHROPIC_OPEN, ANTHROPIC_FRAME], true)]
#[case::data_line_without_event(&[ANTHROPIC_DATA_ONLY], false)]
#[case::null_stop_reason(&[ANTHROPIC_OPEN, CHAT_DONE], false)]
fn anthropic_ends_on_a_message_delta_with_a_stop_reason(
    #[case] items: &[&str],
    #[case] ended: bool,
) {
    let probe: Probe = UnifiedTranslation::Anthropic(AnthropicTranslation {
        stamps: (),
        _guardrail: PhantomData,
    });
    assert_eq!(probe.streaming_has_ended(&parsed(items)), ended);
}

#[rstest]
#[case::completion(UnifiedTranslation::Completion(CompletionTranslation::default()))]
#[case::embeddings(UnifiedTranslation::Embeddings(EmbeddingsTranslation::default()))]
#[case::image(UnifiedTranslation::Image(ImageTranslation::default()))]
#[case::speech(UnifiedTranslation::Speech(SpeechTranslation::default()))]
#[case::transcription(UnifiedTranslation::Transcription(TranscriptionTranslation::default()))]
#[case::video(UnifiedTranslation::Video(VideoTranslation::default()))]
#[case::a2a(UnifiedTranslation::A2a(A2ATranslation { guardrail: PhantomData }))]
#[case::rerank(UnifiedTranslation::Rerank(RerankTranslation(PhantomData)))]
#[case::google_genai(UnifiedTranslation::GoogleGenAi(GoogleGenAiTranslation(PhantomData)))]
#[case::mcp(UnifiedTranslation::Mcp(McpTranslation(PhantomData)))]
fn other_variants_never_end_the_stream(#[case] probe: Probe) {
    let ended_everywhere = parsed(&[CHAT_DONE, ANTHROPIC_DONE, ANTHROPIC_FRAME, RESPONSES_DONE]);
    assert!(!probe.streaming_has_ended(&ended_everywhere));
}
