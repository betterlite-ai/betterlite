use betterlite_azure::realtime::connect::{RealtimeConnectSettings, prepare_connection};
use betterlite_azure::realtime::protocol::GA_PROTOCOL;
use betterlite_config::tls::Verification;
use betterlite_models::decisions::Json;
use betterlite_models::metadata::identity::Mode;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::realtime::url::QueryParams;
use secrecy::{ExposeSecret as _, SecretString};

use super::metadata::RealtimeModelMetadata;
use super::parameters::truthy;
use super::{ProbeConnection, RealtimeProbeError, realtime_ssl};

pub const TRANSCRIPTION_INTENT: &str = "transcription";
pub const INTENT_KEY: &str = "intent";
pub const TRANSCRIPTION_ENDPOINT: &str = "/v1/realtime/transcription_sessions";
pub const AUDIO_TRANSCRIPTION_MODE: &str = "audio_transcription";

// B529's Azure AD resolution, which reads the merged deployment parameters and the configured sources
pub trait AzureAdTokens: Send + Sync {
    fn token(&self, params: &JsonObject) -> Result<Option<SecretString>, RealtimeProbeError>;
}

pub struct AzureProbeConnection<'probe> {
    pub model: &'probe str,
    pub api_key: Option<&'probe SecretString>,
    pub ad_token: Option<&'probe SecretString>,
    pub api_base: Option<&'probe str>,
    pub api_version: &'probe str,
    pub protocol: &'probe str,
    pub query_params: Option<&'probe QueryParams>,
    pub max_message_size_bytes: usize,
    pub verification: &'probe Verification,
}

// B545's URL and auth helpers under the health check's defaults: an empty base and 2024-10-01-preview
pub fn azure_connection(
    connection: &AzureProbeConnection<'_>,
) -> Result<ProbeConnection, RealtimeProbeError> {
    let plan = prepare_connection(&RealtimeConnectSettings {
        api_base: Some(connection.api_base.unwrap_or("")),
        api_key: connection.api_key,
        azure_ad_token: connection.ad_token,
        model: connection.model,
        api_version: Some(connection.api_version),
        realtime_protocol: Some(connection.protocol),
        query_params: connection.query_params,
        ssl_verification: connection.verification,
        max_message_size_bytes: connection.max_message_size_bytes,
    })?;
    let ssl = realtime_ssl(&plan.url, connection.verification);
    Ok(ProbeConnection {
        url: plan.url,
        headers: plan.headers,
        ssl,
        max_size: plan.max_message_size_bytes,
    })
}

// _azure_realtime_health_protocol: named, then model params, then the environment; no configured string is GA
#[must_use]
pub fn azure_protocol(
    named: Option<&str>,
    merged: Option<&Json>,
    environment: Option<&str>,
) -> String {
    let named_value = named.map(|value| Json::Text(value.to_owned()));
    let environment_value = environment.map(|value| Json::Text(value.to_owned()));
    let selected = [named_value.as_ref(), merged, environment_value.as_ref()]
        .into_iter()
        .flatten()
        .find(|value| truthy(value))
        .or(environment_value.as_ref());
    match selected {
        Some(Json::Text(text)) => text.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => String::from(GA_PROTOCOL),
    }
}

// a transcription-only Azure model supplies intent=transcription and nothing else
#[must_use]
pub fn transcription_query(
    model: &str,
    metadata: &dyn RealtimeModelMetadata,
) -> Option<QueryParams> {
    is_transcription_only(model, metadata).then(|| {
        QueryParams::from([(
            String::from(INTENT_KEY),
            Some(String::from(TRANSCRIPTION_INTENT)),
        )])
    })
}

// _is_transcription_only_realtime_model: a failing lookup is not a transcription model
fn is_transcription_only(model: &str, metadata: &dyn RealtimeModelMetadata) -> bool {
    let Ok(Some(info)) = metadata.model_info(model, super::AZURE_PROVIDER) else {
        return false;
    };
    info.mode == Some(Mode::AudioTranscription)
        || info
            .supported_endpoints
            .iter()
            .any(|endpoint| endpoint == TRANSCRIPTION_ENDPOINT)
}

// `None if api_key else get_azure_ad_token(...)`: a truthy key never resolves an AD token
pub fn ad_token(
    api_key: Option<&SecretString>,
    merged: &JsonObject,
    azure: &dyn AzureAdTokens,
) -> Result<Option<SecretString>, RealtimeProbeError> {
    if api_key.is_some_and(|key| !key.expose_secret().is_empty()) {
        return Ok(None);
    }
    azure.token(merged)
}
