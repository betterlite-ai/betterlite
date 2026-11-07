use betterlite_models::catalog::ModelCatalog;
use betterlite_models::identity::provider_id::ProviderId;

use super::cost_entry::provider_cost_entry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioTranscriptionConfigTag {
    AzureSpeech,
    Deepgram,
    ElevenLabs,
    XAI,
    OpenAIGPT,
    OpenAIWhisper,
    HostedVLLM,
    IBMWatsonX,
    OVHCloud,
    Scaleway,
    Mistral,
    NvidiaRiva,
    Soniox,
    VertexGemini,
    VertexAI,
    Gemini,
    EdenAI,
}

const DIRECT_TAGS: &[(ProviderId, AudioTranscriptionConfigTag)] = &[
    (ProviderId::Deepgram, AudioTranscriptionConfigTag::Deepgram),
    (
        ProviderId::Elevenlabs,
        AudioTranscriptionConfigTag::ElevenLabs,
    ),
    (ProviderId::Xai, AudioTranscriptionConfigTag::XAI),
    (
        ProviderId::HostedVllm,
        AudioTranscriptionConfigTag::HostedVLLM,
    ),
    (ProviderId::Watsonx, AudioTranscriptionConfigTag::IBMWatsonX),
    (ProviderId::Ovhcloud, AudioTranscriptionConfigTag::OVHCloud),
    (ProviderId::Scaleway, AudioTranscriptionConfigTag::Scaleway),
    (ProviderId::Mistral, AudioTranscriptionConfigTag::Mistral),
    (
        ProviderId::NvidiaRiva,
        AudioTranscriptionConfigTag::NvidiaRiva,
    ),
    (ProviderId::Soniox, AudioTranscriptionConfigTag::Soniox),
    (ProviderId::Gemini, AudioTranscriptionConfigTag::Gemini),
    (ProviderId::Edenai, AudioTranscriptionConfigTag::EdenAI),
];

#[must_use]
pub fn select_transcription_config(
    model: &str,
    provider: ProviderId,
    live: &ModelCatalog,
    bundled: &ModelCatalog,
) -> Option<AudioTranscriptionConfigTag> {
    if provider == ProviderId::Azure {
        // LiteLLM reads the cost entry once for every provider, but only Azure branches on it
        let speech = provider_cost_entry(model, provider, live, bundled)
            .and_then(|entry| entry.fields.metadata.audio_transcription_config.as_deref())
            == Some("azure_speech");
        return speech.then_some(AudioTranscriptionConfigTag::AzureSpeech);
    }
    if provider == ProviderId::Openai {
        let tag = if model.contains("gpt-4o") {
            AudioTranscriptionConfigTag::OpenAIGPT
        } else {
            AudioTranscriptionConfigTag::OpenAIWhisper
        };
        return Some(tag);
    }
    if provider == ProviderId::VertexAi {
        let bare_model = model.strip_prefix("vertex_ai/").unwrap_or(model);
        let tag = if bare_model.starts_with("gemini") && bare_model.contains("transcribe") {
            AudioTranscriptionConfigTag::VertexGemini
        } else {
            AudioTranscriptionConfigTag::VertexAI
        };
        return Some(tag);
    }
    DIRECT_TAGS
        .iter()
        .find_map(|(row, tag)| (*row == provider).then_some(*tag))
}
