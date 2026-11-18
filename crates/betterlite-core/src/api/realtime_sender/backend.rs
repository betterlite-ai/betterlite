use super::error::RealtimeSenderError;
use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_contracts::transport::{RealtimeBackend, RealtimeMessage};
use betterlite_models::decisions::Json;
use std::error::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FrameKind {
    Setup,
    Content,
    Plain,
}

pub(super) fn classify_text<Config>(provider: &Config, text: &str) -> FrameKind
where
    Config: BaseRealtimeConfig,
{
    let decoded = serde_json::from_str::<Json>(text).ok();
    match &decoded {
        Some(value @ Json::Object(_)) => {
            if provider.is_setup_message(value) {
                FrameKind::Setup
            } else if provider.is_content_message(value) {
                FrameKind::Content
            } else {
                FrameKind::Plain
            }
        }
        _ => FrameKind::Plain,
    }
}

// Borrow only the backend socket, so this send future does not capture the rest of the non-Send connection state.
pub(super) async fn send_backend<ConfigError, CallbackError>(
    connection: &mut Option<Box<dyn RealtimeBackend>>,
    frame: RealtimeMessage,
) -> Result<(), RealtimeSenderError<ConfigError, CallbackError>>
where
    ConfigError: Error + 'static,
    CallbackError: Error + 'static,
{
    let Some(socket) = connection.as_mut() else {
        return Err(RealtimeSenderError::BackendMissing);
    };
    socket
        .send(frame)
        .await
        .map_err(RealtimeSenderError::Backend)
}
