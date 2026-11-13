mod url;

use crate::azure_ai_ocr::AzureAiOcrConfig;
use crate::azure_ai_ocr::auth::resolve_api_base;
use crate::cohere_ocr::options::CohereOptions;
use crate::cohere_ocr::wire::CohereRequest;
use crate::cohere_ocr::{CohereParseConfig, validate_document};
use crate::json::{Json, Object};
use crate::ocr_config::OcrProviderConfig;
use crate::ocr_config::{OcrRequestContext, OcrTransform, PreparedOcrRequest};
use crate::ocr_document::{OcrDocument, inline_remote_document, validate_inline_document};
use crate::ocr_error::OcrError;
use crate::ocr_http::{OcrClient, body_document};
use crate::request_format::RequestFormat;
use crate::response::OCRResponse;
use std::future::Future;

pub const AZURE_COHERE_PARSE_PATH: [&str; 4] = ["providers", "cohere", "v2", "parse"];

#[derive(Clone, Copy, Debug, Default)]
pub struct AzureAICohereParseConfig;

impl OcrProviderConfig for AzureAICohereParseConfig {
    type OcrParams = CohereOptions;
    type ProviderRequest = CohereRequest;
    type Environment = Vec<(String, String)>;

    fn get_supported_ocr_params(&self, model: &str) -> &'static [&'static str] {
        CohereParseConfig.get_supported_ocr_params(model)
    }

    fn get_api_key_env_var(&self) -> Option<&'static str> {
        AzureAiOcrConfig.get_api_key_env_var()
    }

    fn secret_names(&self) -> Vec<&'static str> {
        AzureAiOcrConfig.secret_names()
    }

    fn get_health_check_document(&self) -> OcrDocument {
        CohereParseConfig.get_health_check_document()
    }

    fn map_ocr_params(
        &self,
        non_default_params: &Object,
        model: &str,
    ) -> Result<Self::OcrParams, OcrError> {
        CohereParseConfig.map_ocr_params(non_default_params, model)
    }

    fn validate_environment(
        &self,
        request: &PreparedOcrRequest,
        client: &OcrClient,
    ) -> impl Future<Output = Result<Self::Environment, OcrError>> + Send {
        AzureAiOcrConfig.validate_environment(request, client)
    }

    fn get_complete_url(
        &self,
        request: &PreparedOcrRequest,
        _optional_params: &Self::OcrParams,
        _environment: &Self::Environment,
    ) -> Result<String, OcrError> {
        let base = resolve_api_base(request)?;
        url::complete_parse_url(&base)
    }

    fn transform_ocr_request(
        &self,
        document: OcrDocument,
        transform: OcrTransform<'_, Self::OcrParams>,
    ) -> Result<Self::ProviderRequest, OcrError> {
        CohereParseConfig.transform_ocr_request(document, transform)
    }

    fn async_transform_ocr_request(
        &self,
        document: OcrDocument,
        transform: OcrTransform<'_, Self::OcrParams>,
        context: OcrRequestContext<'_>,
    ) -> impl Future<Output = Result<Self::ProviderRequest, OcrError>> + Send {
        inlined_cohere_request(document, transform, context)
    }

    fn transform_ocr_response(
        &self,
        model: &str,
        raw_response: &[u8],
        request_format: RequestFormat,
    ) -> Result<OCRResponse, OcrError> {
        CohereParseConfig.transform_ocr_response(model, raw_response, request_format)
    }

    fn validate_request_body(&self, body: &Json) -> Result<(), OcrError> {
        let document = body_document(body)?;
        validate_document(&document)?;
        validate_inline_document(&document)
    }
}

async fn inlined_cohere_request(
    document: OcrDocument,
    transform: OcrTransform<'_, CohereOptions>,
    context: OcrRequestContext<'_>,
) -> Result<CohereRequest, OcrError> {
    validate_document(&document)?;
    CohereParseConfig.transform_ocr_request(
        inline_remote_document(context.client, document, context.connection).await?,
        transform,
    )
}

mod tests;
