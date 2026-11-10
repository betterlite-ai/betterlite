mod auth;
mod normalize;
pub mod options;
mod poll;
mod tests;
mod url;
pub mod wire;

use std::future::Future;

use betterlite_http::proxy::StreamResponse;

use crate::json::Object;
use crate::ocr_config::decode::decode_and_normalize_response;
use crate::ocr_config::{OcrProviderConfig, OcrResponseContext, OcrTransform, PreparedOcrRequest};
use crate::ocr_document::OcrDocument;
use crate::ocr_error::OcrError;
use crate::ocr_http::OcrClient;
use crate::ocr_settings::OcrSettings;
use crate::request_format::RequestFormat;
use crate::response::OCRResponse;

pub const AZURE_DI_API_KEY_ENV: &str = "AZURE_DOCUMENT_INTELLIGENCE_API_KEY";
pub const AZURE_DI_ENDPOINT_ENV: &str = "AZURE_DOCUMENT_INTELLIGENCE_ENDPOINT";
pub const AZURE_DI_SUBSCRIPTION_HEADER: &str = "Ocp-Apim-Subscription-Key";
const AZURE_AUTH_SECRET_NAMES: [&str; 8] = [
    "AZURE_AD_TOKEN",
    "AZURE_TENANT_ID",
    "AZURE_CLIENT_ID",
    "AZURE_CLIENT_SECRET",
    "AZURE_SCOPE",
    "AZURE_AUTHORITY_HOST",
    "AZURE_CREDENTIAL",
    "AZURE_FEDERATED_TOKEN_FILE",
];
const AZURE_DI_DEFAULT_WIDTH: f64 = 8.5_f64;
const AZURE_DI_DEFAULT_HEIGHT: f64 = 11.0_f64;

#[derive(Clone, Copy, Debug, Default)]
pub struct AzureDocIntelligenceOcrConfig;

impl AzureDocIntelligenceOcrConfig {
    pub fn analyze_path(model: &str) -> Result<[String; 3], OcrError> {
        Ok([
            "documentintelligence".to_owned(),
            "documentModels".to_owned(),
            format!("{}:analyze", url::model_id(model)?),
        ])
    }
}

impl OcrProviderConfig for AzureDocIntelligenceOcrConfig {
    type Environment = Vec<(String, String)>;
    type OcrParams = options::DocumentIntelligenceParams;
    type ProviderRequest = wire::DocumentIntelligenceRequest;

    fn get_supported_ocr_params(&self, _model: &str) -> &'static [&'static str] {
        &["pages", "features", "req_format"]
    }

    fn get_api_key_env_var(&self) -> Option<&'static str> {
        Some(AZURE_DI_API_KEY_ENV)
    }

    fn secret_names(&self) -> Vec<&'static str> {
        [AZURE_DI_API_KEY_ENV, AZURE_DI_ENDPOINT_ENV]
            .into_iter()
            .chain(AZURE_AUTH_SECRET_NAMES)
            .collect()
    }

    fn map_ocr_params(
        &self,
        non_default_params: &Object,
        _model: &str,
    ) -> Result<Self::OcrParams, OcrError> {
        options::map_params(non_default_params)
    }

    fn validate_environment(
        &self,
        request: &PreparedOcrRequest,
        client: &OcrClient,
    ) -> impl Future<Output = Result<Self::Environment, OcrError>> + Send {
        auth::resolve_headers(client, request)
    }

    fn get_complete_url(
        &self,
        request: &PreparedOcrRequest,
        optional_params: &Self::OcrParams,
        _environment: &Self::Environment,
    ) -> Result<String, OcrError> {
        url::complete_analyze_url(request, optional_params)
    }

    fn transform_ocr_request(
        &self,
        document: OcrDocument,
        _transform: OcrTransform<'_, Self::OcrParams>,
    ) -> Result<Self::ProviderRequest, OcrError> {
        wire::build_request(&document)
    }

    fn transform_ocr_response(
        &self,
        model: &str,
        raw_response: &[u8],
        request_format: RequestFormat,
    ) -> Result<OCRResponse, OcrError> {
        decode_and_normalize_response(
            model,
            raw_response,
            request_format,
            |request_model, response| {
                normalize::normalize(
                    request_model,
                    response,
                    OcrSettings::default().doc_intelligence_dpi,
                )
            },
        )
    }

    fn async_transform_ocr_response(
        &self,
        model: &str,
        raw_response: StreamResponse,
        context: OcrResponseContext<'_>,
    ) -> impl Future<Output = Result<OCRResponse, OcrError>> + Send {
        poll::transform_ocr_response(model, raw_response, context)
    }
}
