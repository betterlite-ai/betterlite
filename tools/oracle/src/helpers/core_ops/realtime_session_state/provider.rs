use betterlite_core::api::realtime_contracts::events::AudioTranscriptionUsage;
use betterlite_core::api::realtime_session_state::RealtimeProviderConfig;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ProviderInput {
    pub(crate) requires_session_configuration: bool,
    #[serde(default)]
    pub(crate) unbilled_usage: Option<AudioTranscriptionUsage>,
}

impl RealtimeProviderConfig for ProviderInput {
    fn requires_session_configuration(&self) -> bool {
        self.requires_session_configuration
    }

    fn unbilled_usage_on_close(&self, _model: &str) -> Option<AudioTranscriptionUsage> {
        self.unbilled_usage.clone()
    }
}
