use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use crate::langfuse_exporter::error::LangfuseExporterError;
use crate::langfuse_exporter::{
    INGESTION_VERSION_HEADER, LANGFUSE_INGESTION_VERSION, LANGFUSE_SDK_NAME, LANGFUSE_SDK_VERSION,
    PROTOBUF_CONTENT_TYPE, PUBLIC_KEY_HEADER, SDK_NAME_HEADER, SDK_VERSION_HEADER,
};

// LiteLLM joins the two with rstrip('/') on the base and lstrip('/') on the path, so one slash separates them
#[must_use]
pub fn endpoint(base_url: &str, export_path: &str) -> String {
    format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        export_path.trim_start_matches('/')
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
    for (name, value) in [
        (CONTENT_TYPE.as_str(), PROTOBUF_CONTENT_TYPE),
        (SDK_NAME_HEADER, LANGFUSE_SDK_NAME),
        (SDK_VERSION_HEADER, LANGFUSE_SDK_VERSION),
        (INGESTION_VERSION_HEADER, LANGFUSE_INGESTION_VERSION),
    ] {
        headers.insert(
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
    }
    headers.insert(
        HeaderName::from_static(PUBLIC_KEY_HEADER),
        HeaderValue::try_from(public).map_err(LangfuseExporterError::PublicKey)?,
    );
    Ok(headers)
}
