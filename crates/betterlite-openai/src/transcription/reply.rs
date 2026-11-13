use betterlite_models::call_helpers::headers::ProviderHeaderProjection;
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;

use crate::api::transcription_contract::TranscriptionResponse;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptionReply {
    pub response: TranscriptionResponse,
    pub original_response: Json,
    pub provider_headers: ProviderHeaderProjection,
    pub model: Option<String>,
    pub custom_llm_provider: &'static str,
    pub duration: StoredDuration,
}

// LiteLLM stores the duration under _audio_transcription_duration only on the async textual reply
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredDuration {
    Absent,
    Known(Option<Decimal>),
}
