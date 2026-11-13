pub mod auth;
pub(crate) mod url;

use crate::json::{Json, Object};
use crate::mistral_ocr::{MistralOcrConfig, MistralOcrRequest};
use crate::ocr_config::{OcrProviderConfig, OcrRequestContext, OcrTransform, PreparedOcrRequest};
use crate::ocr_document::{OcrDocument, inline_remote_document, validate_inline_document};
use crate::ocr_error::OcrError;
use crate::ocr_http::{OcrClient, body_document};
use crate::request_format::RequestFormat;
use crate::response::OCRResponse;
use std::future::Future;

pub const AZURE_AI_API_KEY_ENV: &str = "AZURE_AI_API_KEY";
pub const AZURE_AI_API_BASE_ENV: &str = "AZURE_AI_API_BASE";
pub const AZURE_AI_OCR_PATH: [&str; 4] = ["providers", "mistral", "azure", "ocr"];
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

#[derive(Clone, Copy, Debug, Default)]
pub struct AzureAiOcrConfig;

impl OcrProviderConfig for AzureAiOcrConfig {
    type OcrParams = Object;
    type ProviderRequest = MistralOcrRequest;
    type Environment = Vec<(String, String)>;

    fn get_supported_ocr_params(&self, model: &str) -> &'static [&'static str] {
        MistralOcrConfig.get_supported_ocr_params(model)
    }

    fn get_api_key_env_var(&self) -> Option<&'static str> {
        Some(AZURE_AI_API_KEY_ENV)
    }

    fn secret_names(&self) -> Vec<&'static str> {
        [AZURE_AI_API_KEY_ENV, AZURE_AI_API_BASE_ENV]
            .into_iter()
            .chain(AZURE_AUTH_SECRET_NAMES)
            .collect()
    }

    fn map_ocr_params(&self, non_default_params: &Object, model: &str) -> Result<Object, OcrError> {
        MistralOcrConfig.map_ocr_params(non_default_params, model)
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
        _optional_params: &Self::OcrParams,
        _environment: &Self::Environment,
    ) -> Result<String, OcrError> {
        let base = auth::resolve_api_base(request)?;
        url::complete_provider_path(&base, &AZURE_AI_OCR_PATH)
    }

    fn transform_ocr_request(
        &self,
        document: OcrDocument,
        transform: OcrTransform<'_, Self::OcrParams>,
    ) -> Result<Self::ProviderRequest, OcrError> {
        MistralOcrConfig.transform_ocr_request(document, transform)
    }

    fn async_transform_ocr_request(
        &self,
        document: OcrDocument,
        transform: OcrTransform<'_, Self::OcrParams>,
        context: OcrRequestContext<'_>,
    ) -> impl Future<Output = Result<Self::ProviderRequest, OcrError>> + Send {
        inlined_mistral_request(document, transform, context)
    }

    fn transform_ocr_response(
        &self,
        model: &str,
        raw_response: &[u8],
        request_format: RequestFormat,
    ) -> Result<OCRResponse, OcrError> {
        MistralOcrConfig.transform_ocr_response(model, raw_response, request_format)
    }

    fn validate_request_body(&self, body: &Json) -> Result<(), OcrError> {
        validate_inline_document(&body_document(body)?)
    }
}

async fn inlined_mistral_request(
    document: OcrDocument,
    transform: OcrTransform<'_, Object>,
    context: OcrRequestContext<'_>,
) -> Result<MistralOcrRequest, OcrError> {
    MistralOcrConfig.transform_ocr_request(
        inline_remote_document(context.client, document, context.connection).await?,
        transform,
    )
}

mod tests;
