use super::{OCR_RESPONSE_MAX_BYTES, Secrets};
use crate::ocr_settings::OcrSettings;
use secrecy::{ExposeSecret as _, SecretString};
use std::time::Duration;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OcrInputSource {
    Request,
    Environment,
    #[default]
    Deployment,
}
#[derive(Clone)]
pub struct OcrSourced<T> {
    pub value: T,
    pub source: OcrInputSource,
}
#[derive(Clone, Default)]
pub struct OcrCredentialInputs {
    pub api_key: Option<OcrSourced<SecretString>>,
    pub dynamic_api_key: Option<OcrSourced<SecretString>>,
    pub api_base: Option<OcrSourced<String>>,
    pub dynamic_api_base: Option<OcrSourced<String>>,
}
impl OcrCredentialInputs {
    #[must_use]
    pub fn new(
        api_key: Option<SecretString>,
        api_key_source: OcrInputSource,
        api_base: Option<String>,
        api_base_source: OcrInputSource,
    ) -> Self {
        Self {
            api_key: nonblank_secret(api_key).map(|value| OcrSourced {
                value,
                source: api_key_source,
            }),
            dynamic_api_key: None,
            api_base: nonblank(api_base).map(|value| OcrSourced {
                value,
                source: api_base_source,
            }),
            dynamic_api_base: None,
        }
    }
}
#[derive(Clone, Default)]
pub struct ResolvedOcrCredentials {
    pub api_key: Option<OcrSourced<SecretString>>,
    pub api_base: Option<OcrSourced<String>>,
}
#[derive(Clone)]
pub struct OcrTransportConfig {
    pub extra_headers: Vec<(String, String)>,
    pub extra_headers_source: OcrInputSource,
    pub timeout: Option<Duration>,
    pub max_response_bytes: usize,
}
impl Default for OcrTransportConfig {
    fn default() -> Self {
        Self {
            extra_headers: Vec::new(),
            extra_headers_source: OcrInputSource::Deployment,
            timeout: None,
            max_response_bytes: OCR_RESPONSE_MAX_BYTES,
        }
    }
}
#[derive(Clone)]
pub struct OcrConnection {
    pub api_key: Option<SecretString>,
    pub api_key_source: OcrInputSource,
    pub api_base: Option<String>,
    pub api_base_source: OcrInputSource,
    pub extra_headers: Vec<(String, String)>,
    pub extra_headers_source: OcrInputSource,
    pub timeout: Duration,
    pub max_response_bytes: usize,
    pub settings: OcrSettings,
    pub secrets: Secrets,
}
impl OcrConnection {
    #[must_use]
    pub fn new(
        credentials: ResolvedOcrCredentials,
        transport: OcrTransportConfig,
        settings: OcrSettings,
        secrets: Secrets,
    ) -> Self {
        let api_key_source = credentials
            .api_key
            .as_ref()
            .map_or(OcrInputSource::Deployment, |value| value.source);
        let api_base_source = credentials
            .api_base
            .as_ref()
            .map_or(OcrInputSource::Deployment, |value| value.source);
        Self {
            api_key: credentials.api_key.map(|value| value.value),
            api_key_source,
            api_base: credentials.api_base.map(|value| value.value),
            api_base_source,
            extra_headers: transport.extra_headers,
            extra_headers_source: transport.extra_headers_source,
            timeout: transport
                .timeout
                .filter(|timeout| !timeout.is_zero())
                .unwrap_or(settings.request_timeout),
            max_response_bytes: transport.max_response_bytes,
            settings,
            secrets,
        }
    }
}
fn nonblank(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}
fn nonblank_secret(value: Option<SecretString>) -> Option<SecretString> {
    value
        .map(|secret| SecretString::from(secret.expose_secret().trim()))
        .filter(|secret| !secret.expose_secret().is_empty())
}
