use std::collections::{BTreeMap, BTreeSet};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use super::cloud_endpoints::LANGFUSE_CLOUD_US_ENDPOINT;
use super::error::LangfuseOtelError;
use super::ingestion_constants::{LANGFUSE_INGESTION_VERSION, LANGFUSE_VERSION_HEADER};
use crate::otel::legacy_policy::config::{
    Exporter, OTLP_HTTP_EXPORTER, OpenTelemetryConfig, Signals,
};

pub type OtelHeaders = IndexMap<&'static str, SecretString>;

fn env_value(env: &BTreeMap<String, String>, name: &str) -> Option<String> {
    match env.get(name) {
        Some(value) if !value.is_empty() => Some(value.clone()),
        _ => None,
    }
}

// An empty LANGFUSE_OTEL_HOST counts as unset, so LANGFUSE_HOST still applies.
#[must_use]
pub fn langfuse_otel_host(env: &BTreeMap<String, String>) -> Option<String> {
    for name in ["LANGFUSE_OTEL_HOST", "LANGFUSE_HOST"] {
        if let Some(host) = env_value(env, name) {
            return Some(host);
        }
    }
    None
}

fn env_credentials(env: &BTreeMap<String, String>) -> Option<(String, SecretString)> {
    let public_key = env_value(env, "LANGFUSE_PUBLIC_KEY")?;
    let secret_key = env_value(env, "LANGFUSE_SECRET_KEY")?;
    Some((public_key, SecretString::from(secret_key)))
}

// Without both keys the logger falls back to OpenTelemetryConfig.from_env(), post-init defaults included.
#[must_use]
pub fn config_from_langfuse_env(env: &BTreeMap<String, String>) -> OpenTelemetryConfig {
    let Some((public_key, secret_key)) = env_credentials(env) else {
        let mut config = OpenTelemetryConfig::from_env(env);
        config.normalize(env);
        return config;
    };
    build_langfuse_otel_config(
        &public_key,
        &secret_key,
        langfuse_otel_host(env).as_deref(),
        env,
    )
}

pub fn get_langfuse_otel_config(
    env: &BTreeMap<String, String>,
) -> Result<OpenTelemetryConfig, LangfuseOtelError> {
    let (public_key, secret_key) =
        env_credentials(env).ok_or(LangfuseOtelError::MissingCredentials)?;
    Ok(build_langfuse_otel_config(
        &public_key,
        &secret_key,
        langfuse_otel_host(env).as_deref(),
        env,
    ))
}

#[must_use]
pub fn build_langfuse_otel_config(
    public_key: &str,
    secret_key: &SecretString,
    langfuse_host: Option<&str>,
    env: &BTreeMap<String, String>,
) -> OpenTelemetryConfig {
    let endpoint = langfuse_host.filter(|host| !host.is_empty()).map_or_else(
        || LANGFUSE_CLOUD_US_ENDPOINT.to_owned(),
        |host| {
            let normalized = if host.starts_with("http") {
                host.to_owned()
            } else {
                format!("https://{host}")
            };
            format!("{}/api/public/otel", normalized.trim_end_matches('/'))
        },
    );
    tracing::debug!(%endpoint, "Using Langfuse OTEL endpoint");
    let auth_header = langfuse_authorization_header(public_key, secret_key);
    let headers = format_otel_headers(&build_langfuse_otel_headers(auth_header));
    let mut config = OpenTelemetryConfig {
        exporter: Exporter::Named(OTLP_HTTP_EXPORTER.to_owned()),
        endpoint: Some(endpoint),
        headers: Some(headers),
        signals: Signals {
            enable_metrics: false,
            enable_events: false,
        },
        service_name: String::new(),
        deployment_environment: String::new(),
        model_id: String::new(),
        ignore_context_propagation: false,
        skip_set_global: false,
        capture_message_content: None,
        semconv_stability_opt_in: BTreeSet::new(),
        baggage_team_metadata_keys: Vec::new(),
        baggage_metadata_keys: Vec::new(),
        attributes: None,
    };
    config.normalize(env);
    config
}

#[must_use]
pub fn langfuse_authorization_header(public_key: &str, secret_key: &SecretString) -> SecretString {
    let encoded = STANDARD.encode(format!("{public_key}:{}", secret_key.expose_secret()));
    SecretString::from(format!("Basic {encoded}"))
}

#[must_use]
pub fn build_langfuse_otel_headers(auth_header: SecretString) -> OtelHeaders {
    let mut headers = OtelHeaders::new();
    headers.insert("Authorization", auth_header);
    headers.insert(
        LANGFUSE_VERSION_HEADER,
        SecretString::from(LANGFUSE_INGESTION_VERSION),
    );
    headers
}

#[must_use]
pub fn format_otel_headers(headers: &OtelHeaders) -> String {
    headers
        .iter()
        .map(|(name, value)| format!("{name}={}", value.expose_secret()))
        .collect::<Vec<_>>()
        .join(",")
}
