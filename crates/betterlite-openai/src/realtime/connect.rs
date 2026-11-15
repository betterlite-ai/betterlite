use betterlite_config::tls::Verification;
use secrecy::SecretString;

use super::error::RealtimeError;
use super::headers::additional_headers;
use super::logging::{RealtimeLogRecord, RealtimeLogger};
use super::normalizer::RealtimeEventNormalizer;
use super::ssl::{RealtimeSsl, ssl_config};
use super::url::{QueryParams, construct_url};
use crate::openai_base_config::Headers;

pub const DEFAULT_API_BASE: &str = "https://api.openai.com/";

pub struct RealtimeConnectSettings<'settings> {
    pub api_base: Option<&'settings str>,
    pub api_key: Option<&'settings SecretString>,
    pub model: &'settings str,
    pub query_params: Option<&'settings QueryParams>,
    pub beta_realtime: bool,
    pub ssl_verification: &'settings Verification,
    pub max_message_size_bytes: usize,
}

pub struct RealtimeConnectionPlan {
    pub url: String,
    pub headers: Headers,
    pub ssl: RealtimeSsl,
    pub max_message_size_bytes: usize,
    pub force_transcription_model: Option<String>,
    pub event_normalizer: Option<Box<dyn RealtimeEventNormalizer>>,
}

pub fn prepare_connection(
    settings: &RealtimeConnectSettings<'_>,
    logger: &mut dyn RealtimeLogger,
) -> Result<RealtimeConnectionPlan, RealtimeError> {
    let api_base = settings.api_base.unwrap_or(DEFAULT_API_BASE);
    let api_key = settings.api_key.ok_or(RealtimeError::MissingApiKey)?;
    let fallback = QueryParams::from([(String::from("model"), Some(settings.model.to_owned()))]);
    let query_params = settings.query_params.unwrap_or(&fallback);
    let url = construct_url(api_base, query_params)?;
    let headers = additional_headers(api_key, settings.beta_realtime);
    let ssl = ssl_config(&url, settings.ssl_verification);
    logger
        .pre_call(RealtimeLogRecord {
            api_base: &url,
            headers: &headers,
            query_params,
        })
        .map_err(|source| RealtimeError::Logging { source })?;
    let force_transcription_model = query_params
        .get("intent")
        .and_then(Option::as_deref)
        .filter(|intent| *intent == "transcription")
        .map(|_intent| settings.model.to_owned());
    Ok(RealtimeConnectionPlan {
        url,
        headers,
        ssl,
        max_message_size_bytes: settings.max_message_size_bytes,
        force_transcription_model,
        event_normalizer: None,
    })
}
