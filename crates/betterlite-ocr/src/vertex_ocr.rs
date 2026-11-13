use std::future::Future;

use crate::json::{Json, Object};
use crate::mistral_ocr::{MistralOcrConfig, MistralOcrRequest};
use crate::ocr_config::{
    OcrEnvironment, OcrProviderConfig, OcrRequestContext, OcrTransform, PreparedOcrRequest,
};
use crate::ocr_document::{OcrDocument, inline_remote_document, validate_inline_document};
use crate::ocr_error::OcrError;
use crate::ocr_http::{OcrClient, body_document};
use crate::request_format::RequestFormat;
use crate::response::OCRResponse;

pub(crate) mod common;
mod tests;
pub(crate) mod url;

pub struct VertexOcrEnvironment {
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) project_id: String,
}

impl OcrEnvironment for VertexOcrEnvironment {
    fn headers(&self) -> &[(String, String)] {
        &self.headers
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct VertexAiOcrConfig;

impl OcrProviderConfig for VertexAiOcrConfig {
    type Environment = VertexOcrEnvironment;
    type OcrParams = Object;
    type ProviderRequest = MistralOcrRequest;

    fn get_supported_ocr_params(&self, model: &str) -> &'static [&'static str] {
        MistralOcrConfig.get_supported_ocr_params(model)
    }

    fn get_api_key_env_var(&self) -> Option<&'static str> {
        Some(common::VERTEX_AI_API_KEY)
    }

    fn secret_names(&self) -> Vec<&'static str> {
        common::SECRET_NAMES.to_vec()
    }

    fn map_ocr_params(&self, non_default_params: &Object, model: &str) -> Result<Object, OcrError> {
        MistralOcrConfig.map_ocr_params(non_default_params, model)
    }

    fn validate_environment(
        &self,
        request: &PreparedOcrRequest,
        client: &OcrClient,
    ) -> impl Future<Output = Result<VertexOcrEnvironment, OcrError>> + Send {
        common::environment(request, client)
    }

    fn get_complete_url(
        &self,
        request: &PreparedOcrRequest,
        _optional_params: &Object,
        environment: &VertexOcrEnvironment,
    ) -> Result<String, OcrError> {
        let config = common::vertex_config(request)?;
        let location = common::location(request, &config);
        url::raw_predict_url(
            request.connection.api_base.as_deref(),
            &environment.project_id,
            &location,
            &request.model,
        )
    }

    fn transform_ocr_request(
        &self,
        document: OcrDocument,
        transform: OcrTransform<'_, Object>,
    ) -> Result<MistralOcrRequest, OcrError> {
        MistralOcrConfig.transform_ocr_request(document, transform)
    }

    fn async_transform_ocr_request(
        &self,
        document: OcrDocument,
        transform: OcrTransform<'_, Object>,
        context: OcrRequestContext<'_>,
    ) -> impl Future<Output = Result<MistralOcrRequest, OcrError>> + Send {
        inlined_request(document, transform, context)
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

async fn inlined_request(
    document: OcrDocument,
    transform: OcrTransform<'_, Object>,
    context: OcrRequestContext<'_>,
) -> Result<MistralOcrRequest, OcrError> {
    MistralOcrConfig.transform_ocr_request(
        inline_remote_document(context.client, document, context.connection).await?,
        transform,
    )
}
