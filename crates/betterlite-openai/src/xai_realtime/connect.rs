use crate::openai_base_config::Headers;
use crate::realtime::connect::{RealtimeConnectSettings, prepare_connection as openai_connection};
use crate::realtime::error::RealtimeError;
use crate::realtime::logging::RealtimeLogger;
use crate::realtime::ssl::RealtimeSsl;

use super::normalizer::XaiRealtimeNormalizer;

pub const XAI_API_BASE: &str = "https://api.x.ai/v1";

#[derive(Debug)]
pub struct XaiRealtimeConnection {
    pub url: String,
    pub headers: Headers,
    pub ssl: RealtimeSsl,
    pub max_message_size_bytes: usize,
    pub force_transcription_model: Option<String>,
    pub normalizer: XaiRealtimeNormalizer,
}

pub fn prepare_connection(
    settings: &RealtimeConnectSettings<'_>,
    logger: &mut dyn RealtimeLogger,
) -> Result<XaiRealtimeConnection, RealtimeError> {
    // xAI's header override drops the client's OpenAI-Beta header, so beta mode is never forwarded
    let plan = openai_connection(
        &RealtimeConnectSettings {
            api_base: Some(settings.api_base.unwrap_or(XAI_API_BASE)),
            api_key: settings.api_key,
            model: settings.model,
            query_params: settings.query_params,
            beta_realtime: false,
            ssl_verification: settings.ssl_verification,
            max_message_size_bytes: settings.max_message_size_bytes,
        },
        logger,
    )?;
    Ok(XaiRealtimeConnection {
        url: plan.url,
        headers: plan.headers,
        ssl: plan.ssl,
        max_message_size_bytes: plan.max_message_size_bytes,
        force_transcription_model: plan.force_transcription_model,
        normalizer: XaiRealtimeNormalizer::default(),
    })
}
