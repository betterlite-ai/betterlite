#![cfg(test)]

use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::identity::provider_id::ProviderId;
use rstest::rstest;

use super::audio_transcription::{AudioTranscriptionConfigTag, select_transcription_config};
use super::image_edit::{ImageEditConfigTag, select_image_edit};
use super::image_generation::{ImageGenerationConfigTag, select_image_generation};
use super::text_to_speech::{TextToSpeechConfigTag, select_text_to_speech};

fn catalog(source: &str) -> ModelCatalog {
    ModelCatalog::parse(source.as_bytes(), UnknownFields::Warn).expect("catalog")
}

const LIVE: &str = r#"{
    "azure-speech": {"audio_transcription_config": "azure_speech"},
    "azure/azure-prefixed": {"audio_transcription_config": "azure_speech"},
    "azure/azure-other": {"audio_transcription_config": "other"},
    "azure/live-wins": {"audio_transcription_config": "other"}
}"#;

const BUNDLED: &str = r#"{
    "azure/bundled": {"audio_transcription_config": "azure_speech"},
    "bundled-prefixed": {"audio_transcription_config": "azure_speech"},
    "azure/live-wins": {"audio_transcription_config": "azure_speech"}
}"#;

fn transcription(model: &str, provider: ProviderId, expected: Option<AudioTranscriptionConfigTag>) {
    let live = catalog(LIVE);
    let bundled = catalog(BUNDLED);
    assert_eq!(
        select_transcription_config(model, provider, &live, &bundled),
        expected
    );
}

#[rstest]
#[case::azure_live_bare(
    "azure-speech",
    ProviderId::Azure,
    Some(AudioTranscriptionConfigTag::AzureSpeech)
)]
#[case::azure_live_prefixed(
    "azure-prefixed",
    ProviderId::Azure,
    Some(AudioTranscriptionConfigTag::AzureSpeech)
)]
#[case::azure_bundled_prefixed(
    "bundled",
    ProviderId::Azure,
    Some(AudioTranscriptionConfigTag::AzureSpeech)
)]
#[case::azure_bundled_bare(
    "bundled-prefixed",
    ProviderId::Azure,
    Some(AudioTranscriptionConfigTag::AzureSpeech)
)]
#[case::azure_live_missing_falls_through_to_bundled("live-wins", ProviderId::Azure, None)]
#[case::azure_other_config("azure-other", ProviderId::Azure, None)]
#[case::azure_absent_model("unknown", ProviderId::Azure, None)]
#[case::deepgram(
    "nova-3",
    ProviderId::Deepgram,
    Some(AudioTranscriptionConfigTag::Deepgram)
)]
#[case::elevenlabs(
    "scribe",
    ProviderId::Elevenlabs,
    Some(AudioTranscriptionConfigTag::ElevenLabs)
)]
#[case::xai("xai-stt", ProviderId::Xai, Some(AudioTranscriptionConfigTag::XAI))]
#[case::openai_gpt_audio(
    "gpt-4o-transcribe",
    ProviderId::Openai,
    Some(AudioTranscriptionConfigTag::OpenAIGPT)
)]
#[case::openai_whisper(
    "whisper-1",
    ProviderId::Openai,
    Some(AudioTranscriptionConfigTag::OpenAIWhisper)
)]
#[case::hosted_vllm(
    "whisper",
    ProviderId::HostedVllm,
    Some(AudioTranscriptionConfigTag::HostedVLLM)
)]
#[case::watsonx(
    "whisper",
    ProviderId::Watsonx,
    Some(AudioTranscriptionConfigTag::IBMWatsonX)
)]
#[case::ovhcloud(
    "whisper",
    ProviderId::Ovhcloud,
    Some(AudioTranscriptionConfigTag::OVHCloud)
)]
#[case::scaleway(
    "whisper",
    ProviderId::Scaleway,
    Some(AudioTranscriptionConfigTag::Scaleway)
)]
#[case::mistral(
    "voxtral",
    ProviderId::Mistral,
    Some(AudioTranscriptionConfigTag::Mistral)
)]
#[case::nvidia_riva(
    "whisper",
    ProviderId::NvidiaRiva,
    Some(AudioTranscriptionConfigTag::NvidiaRiva)
)]
#[case::soniox("stt", ProviderId::Soniox, Some(AudioTranscriptionConfigTag::Soniox))]
#[case::vertex_gemini_transcribe(
    "vertex_ai/gemini-1.5-transcribe",
    ProviderId::VertexAi,
    Some(AudioTranscriptionConfigTag::VertexGemini)
)]
#[case::vertex_gemini_plain(
    "gemini-1.5-transcribe",
    ProviderId::VertexAi,
    Some(AudioTranscriptionConfigTag::VertexAI)
)]
#[case::vertex_other(
    "whisper",
    ProviderId::VertexAi,
    Some(AudioTranscriptionConfigTag::VertexAI)
)]
#[case::gemini(
    "gemini-transcribe",
    ProviderId::Gemini,
    Some(AudioTranscriptionConfigTag::Gemini)
)]
#[case::edenai(
    "transcribe",
    ProviderId::Edenai,
    Some(AudioTranscriptionConfigTag::EdenAI)
)]
#[case::outside_list("whisper-large-v3", ProviderId::Groq, None)]
fn transcription_selection(
    #[case] model: &str,
    #[case] provider: ProviderId,
    #[case] expected: Option<AudioTranscriptionConfigTag>,
) {
    transcription(model, provider, expected);
}

#[rstest]
#[case::openai("gpt-image-1", ProviderId::Openai, ImageGenerationConfigTag::OpenAi)]
#[case::azure("dall-e-3", ProviderId::Azure, ImageGenerationConfigTag::Azure)]
#[case::azure_ai("gpt-image-1", ProviderId::AzureAi, ImageGenerationConfigTag::AzureAi)]
#[case::xinference("flux", ProviderId::Xinference, ImageGenerationConfigTag::Xinference)]
#[case::recraft("recraft-v3", ProviderId::Recraft, ImageGenerationConfigTag::Recraft)]
#[case::aiml("flux", ProviderId::Aiml, ImageGenerationConfigTag::Aiml)]
#[case::comet_api("flux", ProviderId::Cometapi, ImageGenerationConfigTag::CometApi)]
#[case::gemini("imagen-3", ProviderId::Gemini, ImageGenerationConfigTag::Gemini)]
#[case::fal_ai("flux", ProviderId::FalAi, ImageGenerationConfigTag::FalAi)]
#[case::stability("sd3", ProviderId::Stability, ImageGenerationConfigTag::Stability)]
#[case::runway_ml("gen4", ProviderId::Runwayml, ImageGenerationConfigTag::RunwayMl)]
#[case::black_forest_labs(
    "flux",
    ProviderId::BlackForestLabs,
    ImageGenerationConfigTag::BlackForestLabs
)]
#[case::vertex_ai("imagen-3", ProviderId::VertexAi, ImageGenerationConfigTag::VertexAi)]
#[case::open_router("flux", ProviderId::Openrouter, ImageGenerationConfigTag::OpenRouter)]
#[case::modelscope("flux", ProviderId::Modelscope, ImageGenerationConfigTag::ModelScope)]
fn image_generation_model_selection(
    #[case] model: &str,
    #[case] provider: ProviderId,
    #[case] build: fn(String) -> ImageGenerationConfigTag,
) {
    assert_eq!(
        select_image_generation(model, provider),
        Some(build(model.to_owned()))
    );
}

#[rstest]
#[case::litellm_proxy(ProviderId::LitellmProxy, ImageGenerationConfigTag::LiteLlmProxy)]
#[case::edenai(ProviderId::Edenai, ImageGenerationConfigTag::EdenAi)]
fn image_generation_direct_selection(
    #[case] provider: ProviderId,
    #[case] expected: ImageGenerationConfigTag,
) {
    assert_eq!(
        select_image_generation("any-model", provider),
        Some(expected)
    );
}

#[rstest]
#[case::dashscope(ProviderId::Dashscope, "dashscope")]
#[case::qwencloud(ProviderId::Qwencloud, "qwencloud")]
#[case::qwen_ai_platform(ProviderId::QwenAiPlatform, "qwen_ai_platform")]
fn image_generation_family_selection(#[case] provider: ProviderId, #[case] name: &str) {
    assert_eq!(
        select_image_generation("any-model", provider),
        Some(ImageGenerationConfigTag::DashscopeFamily(name.to_owned()))
    );
}

#[test]
fn image_generation_outside_list() {
    assert_eq!(select_image_generation("any-model", ProviderId::Groq), None);
}

#[rstest]
#[case::openai("gpt-image-1", ProviderId::Openai, ImageEditConfigTag::OpenAi("gpt-image-1".to_owned()))]
#[case::hosted_vllm("flux", ProviderId::HostedVllm, ImageEditConfigTag::HostedVllm("flux".to_owned()))]
#[case::fal_ai("flux", ProviderId::FalAi, ImageEditConfigTag::FalAi("flux".to_owned()))]
#[case::azure_ai("gpt-image-1", ProviderId::AzureAi, ImageEditConfigTag::AzureAi("gpt-image-1".to_owned()))]
#[case::gemini("imagen-3", ProviderId::Gemini, ImageEditConfigTag::Gemini("imagen-3".to_owned()))]
#[case::vertex_ai("imagen-3", ProviderId::VertexAi, ImageEditConfigTag::VertexAi("imagen-3".to_owned()))]
#[case::stability("sd3", ProviderId::Stability, ImageEditConfigTag::Stability("sd3".to_owned()))]
#[case::bedrock("stability.sd3", ProviderId::Bedrock, ImageEditConfigTag::Bedrock("stability.sd3".to_owned()))]
#[case::open_router("flux", ProviderId::Openrouter, ImageEditConfigTag::OpenRouter("flux".to_owned()))]
fn image_edit_model_selection(
    #[case] model: &str,
    #[case] provider: ProviderId,
    #[case] expected: ImageEditConfigTag,
) {
    assert_eq!(select_image_edit(model, provider), Some(expected));
}

#[rstest]
#[case::azure(ProviderId::Azure, ImageEditConfigTag::Azure)]
#[case::recraft(ProviderId::Recraft, ImageEditConfigTag::Recraft)]
#[case::black_forest_labs(ProviderId::BlackForestLabs, ImageEditConfigTag::BlackForestLabs)]
#[case::litellm_proxy(ProviderId::LitellmProxy, ImageEditConfigTag::LiteLlmProxy)]
fn image_edit_direct_selection(#[case] provider: ProviderId, #[case] expected: ImageEditConfigTag) {
    assert_eq!(select_image_edit("any-model", provider), Some(expected));
}

#[test]
fn image_edit_outside_list() {
    assert_eq!(select_image_edit("any-model", ProviderId::Groq), None);
}

#[rstest]
#[case::azure_speech_service(
    "speech/tts",
    ProviderId::Azure,
    false,
    Some(TextToSpeechConfigTag::AzureAva)
)]
#[case::azure_openai_tts("azure-tts", ProviderId::Azure, false, None)]
#[case::elevenlabs(
    "eleven",
    ProviderId::Elevenlabs,
    false,
    Some(TextToSpeechConfigTag::ElevenLabs)
)]
#[case::runway_ml(
    "gen4",
    ProviderId::Runwayml,
    false,
    Some(TextToSpeechConfigTag::RunwayML)
)]
#[case::vertex_gemini("gemini-tts", ProviderId::VertexAi, true, None)]
#[case::vertex_lyria(
    "lyria",
    ProviderId::VertexAi,
    true,
    Some(TextToSpeechConfigTag::VertexAILyria)
)]
#[case::vertex_plain(
    "cloud-tts",
    ProviderId::VertexAi,
    false,
    Some(TextToSpeechConfigTag::VertexAI)
)]
#[case::minimax(
    "speech-2",
    ProviderId::Minimax,
    false,
    Some(TextToSpeechConfigTag::Minimax)
)]
#[case::mistral(
    "voxtral",
    ProviderId::Mistral,
    false,
    Some(TextToSpeechConfigTag::Mistral)
)]
#[case::aws_polly(
    "polly",
    ProviderId::AwsPolly,
    false,
    Some(TextToSpeechConfigTag::AWSPolly)
)]
#[case::edenai("tts", ProviderId::Edenai, false, Some(TextToSpeechConfigTag::EdenAI))]
#[case::outside_list("tts", ProviderId::Groq, false, None)]
fn text_to_speech_selection(
    #[case] model: &str,
    #[case] provider: ProviderId,
    #[case] is_lyria: bool,
    #[case] expected: Option<TextToSpeechConfigTag>,
) {
    assert_eq!(select_text_to_speech(model, provider, is_lyria), expected);
}

#[rstest]
#[case::openai(
    ImageGenerationConfigTag::OpenAi(String::new()),
    "OpenAIImageGenerationConfig"
)]
#[case::dashscope(
    ImageGenerationConfigTag::DashscopeFamily(String::new()),
    "DashscopeFamilyImageGenerationConfig"
)]
#[case::edenai(ImageGenerationConfigTag::EdenAi, "EdenAIImageGenerationConfig")]
fn image_generation_class_names(#[case] tag: ImageGenerationConfigTag, #[case] name: &str) {
    assert_eq!(tag.class_name(), name);
}

#[rstest]
#[case::bedrock(ImageEditConfigTag::Bedrock(String::new()), "BedrockImageEditConfig")]
#[case::azure(ImageEditConfigTag::Azure, "AzureImageEditConfig")]
fn image_edit_class_names(#[case] tag: ImageEditConfigTag, #[case] name: &str) {
    assert_eq!(tag.class_name(), name);
}
