use super::{AZURE_AI_API_BASE_ENV, AZURE_AI_API_KEY_ENV};
use crate::ocr_config::PreparedOcrRequest;
use crate::ocr_config::connection::{OcrConnection, OcrInputSource, OcrSourced};
use crate::ocr_config::has_header;
use crate::ocr_error::{OcrAuthError, OcrError};
use crate::ocr_http::OcrClient;
use secrecy::{ExposeSecret as _, SecretString};
use std::iter::once;

pub(crate) fn resolve_api_base(request: &PreparedOcrRequest) -> Result<String, OcrError> {
    let connection = &request.connection;
    let api_base = connection.api_base.as_deref().and_then(nonblank);
    let environment = connection
        .secrets
        .get(AZURE_AI_API_BASE_ENV)
        .and_then(|value| nonblank(value.expose_secret()));
    api_base.or(environment).ok_or_else(|| {
        OcrError::Auth(OcrAuthError::MissingApiBase {
            provider: "Azure AI",
            guidance: "Set AZURE_AI_API_BASE environment variable or pass api_base parameter",
        })
    })
}

pub(crate) async fn resolve_headers(
    client: &OcrClient,
    request: &PreparedOcrRequest,
) -> Result<Vec<(String, String)>, OcrError> {
    resolve_api_base(request)?;
    let connection = &request.connection;
    if has_header(&connection.extra_headers, "authorization") {
        if request.azure_ad_token_provider.is_some() {
            let _token = resolve_entra(client, request).await?;
        }
        validate_destination(connection, connection.extra_headers_source)?;
        return Ok(connection.extra_headers.clone());
    }
    if let Some(key) = key_credential(connection) {
        validate_destination(connection, key.source)?;
        return Ok(bearer_headers(connection, key.value.expose_secret()));
    }
    let token = resolve_entra(client, request)
        .await?
        .ok_or(OcrError::MissingAzureAiCredentials)?;
    validate_destination(connection, token.source)?;
    Ok(bearer_headers(connection, token.value.expose_secret()))
}

pub async fn resolve_entra(
    client: &OcrClient,
    request: &PreparedOcrRequest,
) -> Result<Option<OcrSourced<SecretString>>, OcrError> {
    // LiteLLM treats an empty token from a caller provider as no token at all
    match client.azure.token(request).await? {
        Some(token) if token.value.expose_secret().is_empty() => Ok(None),
        token => Ok(token),
    }
}

pub fn validate_destination(
    connection: &OcrConnection,
    credential_source: OcrInputSource,
) -> Result<(), OcrError> {
    if connection.api_base.is_some()
        && connection.api_base_source == OcrInputSource::Request
        && credential_source != OcrInputSource::Request
    {
        return Err(OcrError::Auth(OcrAuthError::InvalidConfiguration {
            detail: "host credentials cannot be sent to a request-controlled Azure endpoint"
                .to_owned(),
        }));
    }
    Ok(())
}

fn key_credential(connection: &OcrConnection) -> Option<OcrSourced<SecretString>> {
    connection
        .api_key
        .as_ref()
        .and_then(|key| nonblank(key.expose_secret()))
        .map(|value| OcrSourced {
            value: SecretString::from(value),
            source: connection.api_key_source,
        })
        .or_else(|| {
            connection
                .secrets
                .get(AZURE_AI_API_KEY_ENV)
                .and_then(|value| nonblank(value.expose_secret()))
                .map(|value| OcrSourced {
                    value: SecretString::from(value),
                    source: OcrInputSource::Environment,
                })
        })
}

fn bearer_headers(connection: &OcrConnection, key: &str) -> Vec<(String, String)> {
    once(("Authorization".to_owned(), format!("Bearer {key}")))
        .chain(connection.extra_headers.iter().cloned())
        .collect()
}

fn nonblank(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_owned())
}
