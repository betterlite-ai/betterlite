use betterlite_models::identity::provider_id::ProviderId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealtimeConfigTag {
    Gemini,
    Meta,
}

const REALTIME_TAGS: &[(ProviderId, RealtimeConfigTag)] = &[
    (ProviderId::Gemini, RealtimeConfigTag::Gemini),
    (ProviderId::Meta, RealtimeConfigTag::Meta),
];

#[must_use]
pub fn select_realtime_config(_model: &str, provider: ProviderId) -> Option<RealtimeConfigTag> {
    REALTIME_TAGS
        .iter()
        .find_map(|(row, tag)| (*row == provider).then_some(*tag))
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
