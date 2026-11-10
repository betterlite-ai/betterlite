use secrecy::{ExposeSecret as _, SecretString};
use std::iter::once;

use super::{AZURE_DI_API_KEY_ENV, AZURE_DI_SUBSCRIPTION_HEADER};
use crate::azure_ai_ocr::auth::{resolve_entra, validate_destination};
use crate::ocr_config::connection::{OcrConnection, OcrInputSource};
use crate::ocr_config::{PreparedOcrRequest, has_header};
use crate::ocr_error::OcrError;
use crate::ocr_http::OcrClient;

pub(super) async fn resolve_headers(
    client: &OcrClient,
    request: &PreparedOcrRequest,
) -> Result<Vec<(String, String)>, OcrError> {
    let connection = &request.connection;
    if has_header(&connection.extra_headers, "authorization")
        || has_header(&connection.extra_headers, AZURE_DI_SUBSCRIPTION_HEADER)
    {
        validate_destination(connection, connection.extra_headers_source)?;
        return Ok(connection.extra_headers.clone());
    }
    if let Some((key, source)) = subscription_key(connection) {
        validate_destination(connection, source)?;
        return Ok(once((
            AZURE_DI_SUBSCRIPTION_HEADER.to_owned(),
            key.expose_secret().to_owned(),
        ))
        .chain(connection.extra_headers.iter().cloned())
        .collect());
    }
    let token = resolve_entra(client, request)
        .await?
        .ok_or(OcrError::MissingDocIntelCredentials)?;
    validate_destination(connection, token.source)?;
    Ok(once((
        "Authorization".to_owned(),
        format!("Bearer {}", token.value.expose_secret()),
    ))
    .chain(connection.extra_headers.iter().cloned())
    .collect())
}

fn subscription_key(connection: &OcrConnection) -> Option<(SecretString, OcrInputSource)> {
    if let Some(key) = connection
        .api_key
        .as_ref()
        .map(|key| key.expose_secret().trim())
        .filter(|key| !key.is_empty())
    {
        return Some((SecretString::from(key), connection.api_key_source));
    }
    let configured = connection.secrets.get(AZURE_DI_API_KEY_ENV)?;
    let key = configured.expose_secret().trim();
    (!key.is_empty()).then(|| (SecretString::from(key), OcrInputSource::Environment))
}
