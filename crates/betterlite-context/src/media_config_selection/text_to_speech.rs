use betterlite_models::identity::provider_id::ProviderId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextToSpeechConfigTag {
    AzureAva,
    ElevenLabs,
    RunwayML,
    VertexAILyria,
    VertexAI,
    Minimax,
    Mistral,
    AWSPolly,
    EdenAI,
}

const DIRECT_TAGS: &[(ProviderId, TextToSpeechConfigTag)] = &[
    (ProviderId::Elevenlabs, TextToSpeechConfigTag::ElevenLabs),
    (ProviderId::Runwayml, TextToSpeechConfigTag::RunwayML),
    (ProviderId::Minimax, TextToSpeechConfigTag::Minimax),
    (ProviderId::Mistral, TextToSpeechConfigTag::Mistral),
    (ProviderId::AwsPolly, TextToSpeechConfigTag::AWSPolly),
    (ProviderId::Edenai, TextToSpeechConfigTag::EdenAI),
];

#[must_use]
pub fn select_text_to_speech(
    model: &str,
    provider: ProviderId,
    is_lyria: bool,
) -> Option<TextToSpeechConfigTag> {
    if provider == ProviderId::Azure {
        return model
            .starts_with("speech/")
            .then_some(TextToSpeechConfigTag::AzureAva);
    }
    if provider == ProviderId::VertexAi {
        if model.contains("gemini") {
            return None;
        }
        let tag = if is_lyria {
            TextToSpeechConfigTag::VertexAILyria
        } else {
            TextToSpeechConfigTag::VertexAI
        };
        return Some(tag);
    }
    DIRECT_TAGS
        .iter()
        .find_map(|(row, tag)| (*row == provider).then_some(*tag))
}
