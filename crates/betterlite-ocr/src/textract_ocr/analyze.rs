use std::collections::{BTreeSet, HashMap};
use std::future::{Future, ready};

use serde::{Deserialize, Serialize};

use crate::json::{Json, Object};
use crate::ocr_config::decode::{OcrFieldKind, decode_and_normalize_response, decode_value};
use crate::ocr_config::{OcrProviderConfig, OcrRequestContext, OcrTransform, PreparedOcrRequest};
use crate::ocr_document::{OcrDocument, inline_remote_document};
use crate::ocr_error::OcrError;
use crate::ocr_http::OcrClient;
use crate::request_format::RequestFormat;
use crate::response::OCRResponse;
use crate::textract_ocr::TextractOperation;
use crate::textract_ocr::document::{document_bytes, health_check_document};
use crate::textract_ocr::environment::{
    AWS_SECRET_NAMES, TextractEnvironment, endpoint, textract_environment,
};
use crate::textract_ocr::error::textract_error;
use crate::textract_ocr::markdown::layout_markdown;
use crate::textract_ocr::response::{lines_by_page, ocr_response};
use crate::textract_ocr::wire::{Block, FeatureType, TextractDocument, TextractResponse};

const DEFAULT_FEATURE_TYPES: [FeatureType; 2] = [FeatureType::Layout, FeatureType::Tables];
const SUPPORTED_PARAMS: [&str; 1] = ["feature_types"];

#[derive(Debug, Default, Deserialize)]
pub struct AnalyzeDocumentOptions {
    pub(crate) feature_types: Option<Vec<FeatureType>>,
}

#[derive(Debug, Serialize)]
pub struct AnalyzeDocumentRequest {
    #[serde(rename = "Document")]
    pub(crate) document: TextractDocument,
    #[serde(rename = "FeatureTypes")]
    pub(crate) feature_types: Vec<FeatureType>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TextractAnalyzeDocumentConfig;

impl OcrProviderConfig for TextractAnalyzeDocumentConfig {
    type OcrParams = AnalyzeDocumentOptions;
    type ProviderRequest = AnalyzeDocumentRequest;
    type Environment = TextractEnvironment;

    fn get_supported_ocr_params(&self, _model: &str) -> &'static [&'static str] {
        &SUPPORTED_PARAMS
    }

    fn secret_names(&self) -> Vec<&'static str> {
        AWS_SECRET_NAMES.to_vec()
    }

    fn get_health_check_document(&self) -> OcrDocument {
        health_check_document()
    }

    fn map_ocr_params(
        &self,
        non_default_params: &Object,
        _model: &str,
    ) -> Result<Self::OcrParams, OcrError> {
        decode_value(
            &Json::Object(non_default_params.clone()),
            "optional_params",
            OcrFieldKind::Request,
        )
    }

    fn validate_environment(
        &self,
        request: &PreparedOcrRequest,
        client: &OcrClient,
    ) -> impl Future<Output = Result<TextractEnvironment, OcrError>> + Send {
        ready(textract_environment(
            request,
            TextractOperation::AnalyzeDocument,
            client,
        ))
    }

    fn get_complete_url(
        &self,
        request: &PreparedOcrRequest,
        _optional_params: &Self::OcrParams,
        environment: &TextractEnvironment,
    ) -> Result<String, OcrError> {
        Ok(endpoint(request, environment))
    }

    fn transform_ocr_request(
        &self,
        document: OcrDocument,
        transform: OcrTransform<'_, Self::OcrParams>,
    ) -> Result<Self::ProviderRequest, OcrError> {
        let feature_types = transform
            .optional_params
            .feature_types
            .clone()
            .unwrap_or_else(|| DEFAULT_FEATURE_TYPES.to_vec());
        Ok(AnalyzeDocumentRequest {
            document: document_bytes(&document)?,
            feature_types,
        })
    }

    async fn async_transform_ocr_request(
        &self,
        document: OcrDocument,
        transform: OcrTransform<'_, Self::OcrParams>,
        context: OcrRequestContext<'_>,
    ) -> Result<Self::ProviderRequest, OcrError> {
        let inlined = inline_remote_document(context.client, document, context.connection).await?;
        self.transform_ocr_request(inlined, transform)
    }

    fn transform_ocr_response(
        &self,
        model: &str,
        raw_response: &[u8],
        request_format: RequestFormat,
    ) -> Result<OCRResponse, OcrError> {
        decode_and_normalize_response(model, raw_response, request_format, analyze_response)
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

fn analyze_response(model: &str, response: TextractResponse) -> Result<OCRResponse, OcrError> {
    let blocks = &response.blocks;
    let has_layout = blocks.iter().any(|block| block.kind.layout().is_some());
    let page_markdown: Vec<(i64, String)> = if has_layout {
        let by_id: HashMap<&str, &Block> = blocks
            .iter()
            .map(|block| (block.id.as_str(), block))
            .collect();
        let pages: BTreeSet<i64> = blocks.iter().map(|block| block.page).collect();
        let mut page_markdown = Vec::new();
        for page in pages {
            let markdown = layout_markdown(blocks, page, &by_id)?;
            if !markdown.is_empty() {
                page_markdown.push((page, markdown));
            }
        }
        page_markdown
    } else {
        lines_by_page(blocks)
    };
    Ok(ocr_response(
        model,
        page_markdown,
        response.document_metadata,
    ))
}
