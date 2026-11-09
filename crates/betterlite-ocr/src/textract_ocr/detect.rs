use std::future::{Future, ready};

use serde::Serialize;

use crate::json::Object;
use crate::ocr_config::decode::decode_and_normalize_response;
use crate::ocr_config::{OcrProviderConfig, OcrRequestContext, OcrTransform, PreparedOcrRequest};
use crate::ocr_document::OcrDocument;
use crate::ocr_error::OcrError;
use crate::ocr_http::OcrClient;
use crate::request_format::RequestFormat;
use crate::response::OCRResponse;
use crate::textract_ocr::TextractOperation;
use crate::textract_ocr::document::{document_bytes, health_check_document, inlined_request};
use crate::textract_ocr::environment::{
    AWS_SECRET_NAMES, TextractEnvironment, endpoint, textract_environment,
};
use crate::textract_ocr::error::textract_error;
use crate::textract_ocr::response::{lines_by_page, ocr_response};
use crate::textract_ocr::wire::{TextractDocument, TextractResponse};

#[derive(Debug, Serialize)]
pub struct DetectDocumentTextRequest {
    #[serde(rename = "Document")]
    pub(crate) document: TextractDocument,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TextractDetectTextConfig;

impl OcrProviderConfig for TextractDetectTextConfig {
    type OcrParams = ();
    type ProviderRequest = DetectDocumentTextRequest;
    type Environment = TextractEnvironment;

    fn secret_names(&self) -> Vec<&'static str> {
        AWS_SECRET_NAMES.to_vec()
    }

    fn get_health_check_document(&self) -> OcrDocument {
        health_check_document()
    }

    // the unit params make Ok(Default::default()) the same value as Ok(())
    #[cfg_attr(test, mutants::skip)]
    fn map_ocr_params(
        &self,
        _non_default_params: &Object,
        _model: &str,
    ) -> Result<Self::OcrParams, OcrError> {
        Ok(())
    }

    fn validate_environment(
        &self,
        request: &PreparedOcrRequest,
        client: &OcrClient,
    ) -> impl Future<Output = Result<TextractEnvironment, OcrError>> + Send {
        ready(textract_environment(
            request,
            TextractOperation::DetectDocumentText,
            client,
        ))
    }

    fn get_complete_url(
        &self,
        request: &PreparedOcrRequest,
        _optional_params: &Self::OcrParams,
        environment: &TextractEnvironment,
    ) -> Result<String, OcrError> {
        endpoint(request, environment)
    }

    fn transform_ocr_request(
        &self,
        document: OcrDocument,
        _transform: OcrTransform<'_, Self::OcrParams>,
    ) -> Result<Self::ProviderRequest, OcrError> {
        Ok(DetectDocumentTextRequest {
            document: document_bytes(&document)?,
        })
    }

    fn async_transform_ocr_request(
        &self,
        document: OcrDocument,
        transform: OcrTransform<'_, Self::OcrParams>,
        context: OcrRequestContext<'_>,
    ) -> impl Future<Output = Result<Self::ProviderRequest, OcrError>> + Send {
        inlined_request(self, document, transform, context)
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
            |request_model, response| Ok(detect_response(request_model, response)),
        )
    }

    fn get_error_class(
        &self,
        error_message: String,
        status_code: u16,
        headers: Vec<(String, String)>,
    ) -> OcrError {
        textract_error(error_message, status_code, headers)
    }
}

fn detect_response(model: &str, response: TextractResponse) -> OCRResponse {
    ocr_response(
        model,
        lines_by_page(&response.blocks),
        response.document_metadata,
    )
}
