use betterlite_models::identity::provider_id::ProviderId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealtimeConfigTag {
    Gemini,
    Meta,
}

#[must_use]
pub fn select_realtime_config(_model: &str, provider: ProviderId) -> Option<RealtimeConfigTag> {
    if provider == ProviderId::Gemini {
        Some(RealtimeConfigTag::Gemini)
    } else if provider == ProviderId::Meta {
        Some(RealtimeConfigTag::Meta)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealtimeHttpConfigTag {
    OpenAI,
    Azure,
}

#[must_use]
pub fn select_realtime_http_config(
    _model: &str,
    provider: ProviderId,
) -> Option<RealtimeHttpConfigTag> {
    if provider == ProviderId::Openai {
        Some(RealtimeHttpConfigTag::OpenAI)
    } else if provider == ProviderId::Azure {
        Some(RealtimeHttpConfigTag::Azure)
    } else {
        None
    }
}
