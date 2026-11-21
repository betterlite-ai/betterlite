use betterlite_context::media_config_selection::audio_transcription::AudioTranscriptionConfigTag;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::transcription::config::TranscriptionFamily;

use crate::api::transcription_call::call::SuppliedClientKind;
use crate::api::transcription_call::target::{InjectedTranscription, TranscriptionTarget};

// which injected operation a branch reaches, and the target variant it wraps
#[derive(Clone, Copy)]
pub(super) enum InjectedTarget {
    Riva,
    Soniox,
    Bedrock,
    Native,
}

impl InjectedTarget {
    pub(super) const fn wrap(
        self,
        operation: InjectedTranscription<'_>,
    ) -> TranscriptionTarget<'_> {
        match self {
            Self::Riva => TranscriptionTarget::NvidiaRiva(operation),
            Self::Soniox => TranscriptionTarget::Soniox(operation),
            Self::Bedrock => TranscriptionTarget::Bedrock(operation),
            Self::Native => TranscriptionTarget::Native(operation),
        }
    }
}

// Soniox and the generic native handler keep only the two HTTPHandler classes
pub(super) const fn http_client(client: Option<SuppliedClientKind>) -> Option<SuppliedClientKind> {
    match client {
        Some(SuppliedClientKind::HttpHandler | SuppliedClientKind::AsyncHttpHandler) => client,
        Some(
            SuppliedClientKind::OpenAi
            | SuppliedClientKind::AsyncOpenAi
            | SuppliedClientKind::AzureOpenAi
            | SuppliedClientKind::AsyncAzureOpenAi,
        )
        | None => None,
    }
}

// the OpenAI branch's family is the provider config B157 selected
pub(super) const fn transcription_family(
    config: Option<AudioTranscriptionConfigTag>,
) -> Option<TranscriptionFamily> {
    match config {
        Some(AudioTranscriptionConfigTag::OpenAIWhisper) => Some(TranscriptionFamily::Whisper),
        Some(AudioTranscriptionConfigTag::OpenAIGPT) => Some(TranscriptionFamily::Gpt),
        Some(
            AudioTranscriptionConfigTag::AzureSpeech
            | AudioTranscriptionConfigTag::Deepgram
            | AudioTranscriptionConfigTag::ElevenLabs
            | AudioTranscriptionConfigTag::XAI
            | AudioTranscriptionConfigTag::HostedVLLM
            | AudioTranscriptionConfigTag::IBMWatsonX
            | AudioTranscriptionConfigTag::OVHCloud
            | AudioTranscriptionConfigTag::Scaleway
            | AudioTranscriptionConfigTag::Mistral
            | AudioTranscriptionConfigTag::NvidiaRiva
            | AudioTranscriptionConfigTag::Soniox
            | AudioTranscriptionConfigTag::VertexGemini
            | AudioTranscriptionConfigTag::VertexAI
            | AudioTranscriptionConfigTag::Gemini
            | AudioTranscriptionConfigTag::EdenAI,
        )
        | None => None,
    }
}

// optional_params["extra_headers"] = extra_headers, which keeps a None as null
pub(super) fn headers_value(headers: Option<&Headers>) -> JsonValue {
    headers.map_or(JsonValue::Null, |fields| {
        JsonValue::Object(
            fields
                .iter()
                .map(|(name, value)| (name.clone(), JsonValue::Text(value.clone())))
                .collect(),
        )
    })
}
