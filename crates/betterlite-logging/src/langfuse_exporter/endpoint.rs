use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use crate::langfuse_exporter::error::LangfuseExporterError;
use crate::langfuse_exporter::{
    DEFAULT_BASE_URL, DEFAULT_EXPORT_PATH, INGESTION_VERSION_HEADER, LANGFUSE_INGESTION_VERSION,
    LANGFUSE_SDK_NAME, LANGFUSE_SDK_VERSION, PROTOBUF_CONTENT_TYPE, PUBLIC_KEY_HEADER,
    SDK_NAME_HEADER, SDK_VERSION_HEADER,
};

// LiteLLM joins the two with rstrip('/') on the base and lstrip('/') on the path, so one slash separates them
#[must_use]
pub fn endpoint(base_url: Option<&str>, export_path: Option<&str>) -> String {
    let base = base_url
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_BASE_URL);
    let path = export_path
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_EXPORT_PATH);
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

// The v4 SDK's span processor sends these on every export; the key pair travels only inside Authorization
pub fn request_headers(
    public_key: Option<&str>,
    secret_key: Option<&SecretString>,
) -> Result<HeaderMap, LangfuseExporterError> {
    let public = public_key.unwrap_or_default();
    let secret = secret_key.map_or("", |secret| secret.expose_secret());
    let pair = SecretString::from(format!("{public}:{secret}"));
    let authorization =
        SecretString::from(format!("Basic {}", STANDARD.encode(pair.expose_secret())));
    let mut authorization_value = HeaderValue::try_from(authorization.expose_secret())
        .map_err(LangfuseExporterError::Authorization)?;
    authorization_value.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, authorization_value);
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static(PROTOBUF_CONTENT_TYPE),
    );
    headers.insert(
        HeaderName::from_static(SDK_NAME_HEADER),
        HeaderValue::from_static(LANGFUSE_SDK_NAME),
    );
    headers.insert(
        HeaderName::from_static(SDK_VERSION_HEADER),
        HeaderValue::from_static(LANGFUSE_SDK_VERSION),
    );
    headers.insert(
        HeaderName::from_static(PUBLIC_KEY_HEADER),
        HeaderValue::try_from(public).map_err(LangfuseExporterError::PublicKey)?,
    );
    headers.insert(
        HeaderName::from_static(INGESTION_VERSION_HEADER),
        HeaderValue::from_static(LANGFUSE_INGESTION_VERSION),
    );
    Ok(headers)
}
