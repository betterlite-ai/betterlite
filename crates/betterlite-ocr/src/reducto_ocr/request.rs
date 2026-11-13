use crate::json::{Json, Object};
use crate::ocr_config::PreparedOcrRequest;
use crate::ocr_config::has_header;
use crate::ocr_error::OcrError;
use secrecy::ExposeSecret as _;
use serde::Serialize;

use super::{REDUCTO_API_KEY_ENV, REDUCTO_ID_PREFIX};

// the V3 body is the file id under "input" with the selected options beside it
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct ReductoV3Request(pub(super) Object);

#[derive(Debug, Serialize)]
pub struct ReductoLegacyRequest {
    pub(super) document_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) options: Option<ReductoLegacyOptions>,
}

#[derive(Debug, Serialize)]
pub(super) struct ReductoLegacyOptions {
    pub(super) enhance: Json,
}

pub(super) fn select_params(non_default_params: &Object, supported: &[&str]) -> Object {
    non_default_params
        .iter()
        .filter(|(name, _)| supported.contains(&name.as_str()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

pub(super) fn reducto_id(source: &str) -> Result<String, OcrError> {
    let Some(id) = source.strip_prefix(REDUCTO_ID_PREFIX) else {
        return Err(OcrError::ReductoSource);
    };
    if id.trim().is_empty() {
        return Err(OcrError::RequestField {
            path: "document file id".to_owned(),
        });
    }
    Ok(source.to_owned())
}

pub(super) fn resolve_headers(
    request: &PreparedOcrRequest,
) -> Result<Vec<(String, String)>, OcrError> {
    let connection = &request.connection;
    if has_header(&connection.extra_headers, "authorization") {
        return Ok(connection.extra_headers.clone());
    }
    let api_key = connection
        .api_key
        .as_ref()
        .map(|key| key.expose_secret().trim().to_owned())
        .filter(|key| !key.is_empty())
        .or_else(|| {
            connection
                .secrets
                .get(REDUCTO_API_KEY_ENV)
                .map(|key| key.expose_secret().trim().to_owned())
                .filter(|key| !key.is_empty())
        })
        .ok_or(OcrError::MissingReductoApiKey)?;
    let mut headers = Vec::with_capacity(connection.extra_headers.len().saturating_add(1));
    headers.push(("Authorization".to_owned(), format!("Bearer {api_key}")));
    headers.extend_from_slice(&connection.extra_headers);
    Ok(headers)
}
