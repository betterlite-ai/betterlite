#![cfg(test)]

use std::future::{Future, ready};

use rstest::rstest;
use strum::VariantArray as _;

use crate::decisions::Json;
use crate::model_capability_predicates::{CapabilityPredicates, CapabilityRequest};

use super::{ModelCapabilityFlag, is_vision_explicitly_disabled, supports};

const KEYS: [&str; 11] = [
    "supports_web_search",
    "supports_url_context",
    "supports_audio_input",
    "supports_pdf_input",
    "supports_audio_output",
    "supports_prompt_cache_breakpoint",
    "supports_thinking_cache_preservation",
    "supports_computer_use",
    "supports_vision",
    "supports_anthropic_thinking_payload",
    "supports_embedding_image_input",
];

#[derive(Default)]
struct Recorder {
    supported: Vec<String>,
}

impl CapabilityPredicates for Recorder {
    fn supports_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        self.supported.push(key.to_owned());
        ready(true)
    }

    fn declared_value_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(None)
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(key == "supports_vision")
    }

    fn supports_native_streaming(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = Json> + Send {
        ready(Json::Bool(false))
    }

    fn supports_response_schema(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }
}

#[test]
fn flags_name_every_capability_key_once() {
    let keys: Vec<&str> = ModelCapabilityFlag::VARIANTS
        .iter()
        .map(|flag| flag.key())
        .collect();
    assert_eq!(keys, KEYS);
}

#[rstest]
#[case::web_search(ModelCapabilityFlag::WebSearch, "supports_web_search")]
#[case::url_context(ModelCapabilityFlag::UrlContext, "supports_url_context")]
#[case::audio_input(ModelCapabilityFlag::AudioInput, "supports_audio_input")]
#[case::pdf_input(ModelCapabilityFlag::PdfInput, "supports_pdf_input")]
#[case::audio_output(ModelCapabilityFlag::AudioOutput, "supports_audio_output")]
#[case::prompt_cache_breakpoint(
    ModelCapabilityFlag::PromptCacheBreakpoint,
    "supports_prompt_cache_breakpoint"
)]
#[case::thinking_cache_preservation(
    ModelCapabilityFlag::ThinkingCachePreservation,
    "supports_thinking_cache_preservation"
)]
#[case::computer_use(ModelCapabilityFlag::ComputerUse, "supports_computer_use")]
#[case::vision(ModelCapabilityFlag::Vision, "supports_vision")]
#[case::anthropic_thinking_payload(
    ModelCapabilityFlag::AnthropicThinkingPayload,
    "supports_anthropic_thinking_payload"
)]
#[case::embedding_image_input(
    ModelCapabilityFlag::EmbeddingImageInput,
    "supports_embedding_image_input"
)]
#[tokio::test]
async fn supports_asks_the_factory_for_the_flags_key(
    #[case] flag: ModelCapabilityFlag,
    #[case] key: &str,
) {
    let mut recorder = Recorder::default();
    assert!(supports(&mut recorder, "model", Some("deepseek"), flag).await);
    assert_eq!(recorder.supported, [key.to_owned()]);
}

#[tokio::test]
async fn vision_disabled_asks_the_disabled_factory_for_vision() {
    let mut recorder = Recorder::default();
    let request = CapabilityRequest {
        model: "model",
        custom_llm_provider: None,
    };
    assert!(is_vision_explicitly_disabled(&mut recorder, &request).await);
    assert!(recorder.supported.is_empty());
}
