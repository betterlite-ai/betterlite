use crate::json::{Json, Object};
use crate::ocr_config::connection::OcrConnection;
use crate::ocr_config::decode::decode_and_normalize_response;
use crate::ocr_config::{OcrProviderConfig, OcrTransform, PreparedOcrRequest, has_header};
use crate::ocr_document::OcrDocument;
use crate::ocr_error::{OcrAuthError, OcrError};
use crate::ocr_http::OcrClient;
use crate::request_format::RequestFormat;
use crate::response::{OCRPage, OCRResponse, OCRUsageInfo};
use secrecy::ExposeSecret as _;
use serde::{Deserialize, Serialize};
use std::future::{Future, ready};
use std::iter::once;
use url::Url;

const MISTRAL_OCR_API_BASE: &str = "https://api.mistral.ai/v1";
const MISTRAL_OCR_API_KEY_ENV_VAR: &str = "MISTRAL_API_KEY";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MistralOcrRequest {
    pub model: String,
    pub document: OcrDocument,
    #[serde(flatten)]
    pub params: Object,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct MistralOcrResponse {
    #[serde(default)]
    pub pages: Vec<OCRPage>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    pub model: Option<Option<String>>,
    #[serde(default)]
    pub document_annotation: Option<Json>,
    #[serde(default)]
    pub usage_info: Option<OCRUsageInfo>,
    #[serde(flatten)]
    pub extra_fields: Object,
}

#[derive(Clone, Debug, Default)]
pub struct MistralOcrConfig;

impl OcrProviderConfig for MistralOcrConfig {
    type OcrParams = Object;
    type ProviderRequest = MistralOcrRequest;
    type Environment = Vec<(String, String)>;

    fn get_supported_ocr_params(&self, _model: &str) -> &'static [&'static str] {
        &[
            "pages",
            "include_image_base64",
            "image_limit",
            "image_min_size",
            "bbox_annotation_format",
            "document_annotation_format",
            "document_annotation_prompt",
            "extract_header",
            "extract_footer",
            "table_format",
            "confidence_scores_granularity",
            "include_blocks",
            "id",
        ]
    }

    fn get_api_key_env_var(&self) -> Option<&'static str> {
        Some(MISTRAL_OCR_API_KEY_ENV_VAR)
    }

    fn secret_names(&self) -> Vec<&'static str> {
        vec![
            MISTRAL_OCR_API_KEY_ENV_VAR,
            "MISTRAL_AZURE_API_KEY",
            "MISTRAL_AZURE_API_BASE",
        ]
    }

    fn map_ocr_params(
        &self,
        non_default_params: &Object,
        model: &str,
    ) -> Result<Self::OcrParams, OcrError> {
        let supported = self.get_supported_ocr_params(model);
        Ok(non_default_params
            .iter()
            .filter(|(name, _)| supported.contains(&name.as_str()))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect())
    }

    fn validate_environment(
        &self,
        request: &PreparedOcrRequest,
        _client: &OcrClient,
    ) -> impl Future<Output = Result<Self::Environment, OcrError>> + Send {
        ready(resolve_headers(&request.connection))
    }

    fn get_complete_url(
        &self,
        request: &PreparedOcrRequest,
        _optional_params: &Self::OcrParams,
        _environment: &Self::Environment,
    ) -> Result<String, OcrError> {
        build_ocr_url(request.connection.api_base.as_deref())
    }

    fn transform_ocr_request(
        &self,
        document: OcrDocument,
        transform: OcrTransform<'_, Self::OcrParams>,
    ) -> Result<Self::ProviderRequest, OcrError> {
        Ok(MistralOcrRequest {
            model: transform.model.to_owned(),
            document,
            params: transform.optional_params.clone(),
        })
    }

    fn transform_ocr_response(
        &self,
        model: &str,
        raw_response: &[u8],
        request_format: RequestFormat,
    ) -> Result<OCRResponse, OcrError> {
        decode_and_normalize_response(model, raw_response, request_format, normalize_response)
    }
}

fn resolve_headers(connection: &OcrConnection) -> Result<Vec<(String, String)>, OcrError> {
    if has_header(&connection.extra_headers, "authorization") {
        return Ok(connection.extra_headers.clone());
    }
    let explicit = connection
        .api_key
        .as_ref()
        .map(|key| key.expose_secret().trim())
        .filter(|key| !key.is_empty());
    let environment = connection
        .secrets
        .get(MISTRAL_OCR_API_KEY_ENV_VAR)
        .map(secrecy::ExposeSecret::expose_secret)
        .filter(|key| !key.trim().is_empty());
    let api_key = explicit
        .or(environment)
        .ok_or(OcrError::Auth(OcrAuthError::MissingApiKey {
            provider: "Mistral",
            environment_variable: MISTRAL_OCR_API_KEY_ENV_VAR,
        }))?;
    Ok(
        once(("Authorization".to_owned(), format!("Bearer {api_key}")))
            .chain(connection.extra_headers.iter().cloned())
            .collect(),
    )
}

fn build_ocr_url(api_base: Option<&str>) -> Result<String, OcrError> {
    let request_field = || OcrError::RequestField {
        path: "api_base".to_owned(),
    };
    let base = api_base
        .map(str::trim)
        .filter(|base| !base.is_empty())
        .unwrap_or(MISTRAL_OCR_API_BASE);
    let mut url = Url::parse(base).map_err(|_source| request_field())?;
    let existing: Vec<String> = url
        .path_segments()
        .ok_or_else(request_field)?
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .collect();
    let target = ["v1", "ocr"];
    let overlap = (1..=existing.len().min(target.len()))
        .rev()
        .find(|&length| {
            existing
                .iter()
                .skip(existing.len().saturating_sub(length))
                .map(String::as_str)
                .eq(target.iter().copied().take(length))
        })
        .unwrap_or(0);
    url.path_segments_mut()
        .map_err(|()| request_field())?
        .pop_if_empty()
        .extend(target.iter().copied().skip(overlap));
    Ok(String::from(url))
}

pub fn normalize_response(
    model: &str,
    response: MistralOcrResponse,
) -> Result<OCRResponse, OcrError> {
    let response_model = match response.model {
        Some(Some(returned)) => returned,
        Some(None) => {
            return Err(OcrError::ResponseField {
                path: "model".to_owned(),
            });
        }
        None => model.to_owned(),
    };
    Ok(OCRResponse {
        pages: response.pages,
        model: response_model,
        document_annotation: response.document_annotation,
        usage_info: response.usage_info,
        content: None,
        tables: None,
        key_value_pairs: None,
        object: "ocr".to_owned(),
        extra: response.extra_fields,
        provider_native_response: None,
    })
}

mod tests;
