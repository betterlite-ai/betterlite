use secrecy::ExposeSecret as _;
use std::iter::once;
use url::Url;

use super::options::DocumentIntelligenceParams;
use super::{AZURE_DI_ENDPOINT_ENV, AzureDocIntelligenceOcrConfig};
use crate::azure_ai_ocr::url::complete_provider_path;
use crate::ocr_config::PreparedOcrRequest;
use crate::ocr_error::{OcrAuthError, OcrError};

pub(super) fn model_id(model: &str) -> Result<&str, OcrError> {
    let identifier = model.rsplit('/').next().unwrap_or(model);
    if matches!(identifier, "." | "..") {
        return Err(OcrError::DotModel);
    }
    Ok(identifier)
}

pub(super) fn complete_analyze_url(
    request: &PreparedOcrRequest,
    params: &DocumentIntelligenceParams,
) -> Result<String, OcrError> {
    let connection = &request.connection;
    let endpoint = connection
        .api_base
        .as_deref()
        .map(str::trim)
        .filter(|base| !base.is_empty())
        .or_else(|| {
            connection
                .secrets
                .get(AZURE_DI_ENDPOINT_ENV)
                .map(|value| value.expose_secret().trim())
                .filter(|base| !base.is_empty())
        })
        .ok_or_else(|| {
            OcrError::Auth(OcrAuthError::MissingApiBase {
                provider: "Azure Document Intelligence",
                guidance: "Set AZURE_DOCUMENT_INTELLIGENCE_ENDPOINT environment variable or pass api_base parameter",
            })
        })?;
    let path = AzureDocIntelligenceOcrConfig::analyze_path(&request.model)?;
    let completed = complete_provider_path(endpoint, &path.each_ref().map(String::as_str))?;
    let mut url = Url::parse(&completed).map_err(|_source| OcrError::RequestField {
        path: "api_base".to_owned(),
    })?;
    url.query_pairs_mut().extend_pairs(
        once((
            "api-version",
            connection.settings.doc_intelligence_api_version.as_str(),
        ))
        .chain(params.pages.as_deref().map(|pages| ("pages", pages)))
        .chain(
            params
                .features
                .as_deref()
                .map(|features| ("features", features)),
        ),
    );
    Ok(url.to_string())
}
