use betterlite_config::tls::Verification;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::realtime::normalizer::RealtimeEventNormalizer;
use betterlite_openai::realtime::ssl::RealtimeSsl;
use betterlite_openai::realtime::url::QueryParams;
use secrecy::SecretString;

use super::error::RealtimeError;
use super::headers::auth_headers;
use super::protocol::{TRANSCRIPTION_INTENT, uses_beta_protocol};
use super::ssl::ssl_config;
use super::url::{construct_url, intent};

pub struct RealtimeConnectSettings<'settings> {
    pub api_base: Option<&'settings str>,
    pub api_key: Option<&'settings SecretString>,
    pub azure_ad_token: Option<&'settings SecretString>,
    pub model: &'settings str,
    pub api_version: Option<&'settings str>,
    pub realtime_protocol: Option<&'settings str>,
    pub query_params: Option<&'settings QueryParams>,
    pub ssl_verification: &'settings Verification,
    pub max_message_size_bytes: usize,
}

pub struct RealtimeConnectionPlan {
    pub url: String,
    pub headers: Headers,
    pub ssl: RealtimeSsl,
    pub max_message_size_bytes: usize,
    pub backend_uses_beta_protocol: bool,
    pub force_transcription_model: Option<String>,
    pub event_normalizer: Option<Box<dyn RealtimeEventNormalizer>>,
}

pub fn prepare_connection(
    settings: &RealtimeConnectSettings<'_>,
) -> Result<RealtimeConnectionPlan, RealtimeError> {
    let api_base = settings.api_base.ok_or(RealtimeError::MissingApiBase)?;
    let beta = uses_beta_protocol(settings.realtime_protocol);
    if beta && settings.api_version.is_none() {
        return Err(RealtimeError::MissingApiVersion);
    }
    let url = construct_url(
        api_base,
        settings.model,
        settings.api_version,
        settings.realtime_protocol,
        settings.query_params,
    )?;
    let headers = auth_headers(settings.api_key, settings.azure_ad_token)?;
    let force_transcription_model = intent(settings.query_params)
        .filter(|value| *value == TRANSCRIPTION_INTENT)
        .map(|_intent| settings.model.to_owned());
    Ok(RealtimeConnectionPlan {
        url,
        headers,
        ssl: ssl_config(settings.ssl_verification),
        max_message_size_bytes: settings.max_message_size_bytes,
        backend_uses_beta_protocol: beta,
        force_transcription_model,
        event_normalizer: None,
    })
}
