use thiserror::Error;
use url::Url;

const MAVVRIK_SUFFIXES: [&str; 3] = [".mavvrik.dev", ".mavvrik.ai", ".mavvrik.app"];
const GCS_HOST: &str = "storage.googleapis.com";

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ValidationError {
    #[error(
        "{key} must be provided for Mavvrik FOCUS destination (set {key} env var or pass in destination_config)"
    )]
    MissingSetting { key: &'static str },
    #[error("MAVVRIK_API_ENDPOINT must be an HTTPS URL")]
    EndpointNotHttps,
    #[error(
        "MAVVRIK_API_ENDPOINT host must be a Mavvrik domain (e.g. https://api.mavvrik.dev/<tenant_id>)"
    )]
    EndpointNotMavvrik,
    #[error("Mavvrik FOCUS destination: {label} is not a valid URL")]
    GcsUrlUnparsable {
        label: &'static str,
        #[source]
        source: url::ParseError,
    },
    #[error("Mavvrik FOCUS destination: {label} must be HTTPS, got scheme '{scheme}'")]
    GcsUrlNotHttps { label: &'static str, scheme: String },
    #[error(
        "Mavvrik FOCUS destination: {label} must be a GCS endpoint (storage.googleapis.com), got '{host}'"
    )]
    GcsUrlNotStorage { label: &'static str, host: String },
}

pub fn validate_api_endpoint(api_endpoint: &str) -> Result<(), ValidationError> {
    if !api_endpoint.starts_with("https://") {
        return Err(ValidationError::EndpointNotHttps);
    }
    let host = Url::parse(api_endpoint)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_default();
    if !MAVVRIK_SUFFIXES.iter().any(|suffix| host.ends_with(suffix)) {
        return Err(ValidationError::EndpointNotMavvrik);
    }
    Ok(())
}

pub fn validate_gcs_url(url: &str, label: &'static str) -> Result<(), ValidationError> {
    let parsed =
        Url::parse(url).map_err(|source| ValidationError::GcsUrlUnparsable { label, source })?;
    if parsed.scheme() != "https" {
        return Err(ValidationError::GcsUrlNotHttps {
            label,
            scheme: parsed.scheme().to_owned(),
        });
    }
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    if host != GCS_HOST && !host.ends_with(&format!(".{GCS_HOST}")) {
        return Err(ValidationError::GcsUrlNotStorage { label, host });
    }
    Ok(())
}
