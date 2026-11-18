use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use betterlite_models::decisions::Json;

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
