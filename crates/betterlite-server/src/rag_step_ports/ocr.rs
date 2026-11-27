use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_core::rag::ingestion::ocr::{
    OcrDocumentKind, OcrError, OcrOperation, OcrRequest, OcrResponse,
};
use betterlite_ocr::json::Object;
use betterlite_ocr::ocr_document::OcrDocument as NativeDocument;
use betterlite_ocr::ocr_error::OcrError as NativeOcrError;
use betterlite_ocr::ocr_http::OcrClient;
use betterlite_ocr::ocr_input::{LiteLLMOcrRequest, OcrDocumentInput};
use betterlite_ocr::ocr_route::aocr;
use betterlite_ocr::response::OCRResponse;

mod tests;

// the litellm.aocr call behind a seam, so the port is tested with a fake provider operation
#[async_trait]
pub trait OcrBackend: Send + Sync {
    async fn aocr(
        &self,
        request: LiteLLMOcrRequest<OcrDocumentInput>,
    ) -> Result<OCRResponse, NativeOcrError>;
}

// a fresh client per call, since the native aocr consumes the OcrClient it is given
pub struct NativeOcr<Factory> {
    pub client: Factory,
}

#[async_trait]
impl<Factory> OcrBackend for NativeOcr<Factory>
where
    Factory: Fn() -> OcrClient + Send + Sync,
{
    async fn aocr(
        &self,
        request: LiteLLMOcrRequest<OcrDocumentInput>,
    ) -> Result<OCRResponse, NativeOcrError> {
        aocr((self.client)(), request).await
    }
}

pub struct OcrCall<Backend> {
    pub backend: Backend,
}

#[async_trait]
impl<Backend> OcrOperation for OcrCall<Backend>
where
    Backend: OcrBackend,
{
    async fn aocr(&self, request: OcrRequest) -> Result<OcrResponse, OcrError> {
        let native = native_request(&request).map_err(|source| OcrError::Failed {
            message: source.to_string(),
        })?;
        let response = self
            .backend
            .aocr(native)
            .await
            .map_err(|source| OcrError::Failed {
                message: source.to_string(),
            })?;
        Ok(OcrResponse::from_pages(
            response
                .pages
                .into_iter()
                .map(|page| page.markdown)
                .collect(),
        ))
    }
}

fn native_request(
    request: &OcrRequest,
) -> Result<LiteLLMOcrRequest<OcrDocumentInput>, NativeOcrError> {
    let document = match request.document.kind {
        OcrDocumentKind::ImageUrl => NativeDocument::ImageUrl {
            image_url: request.document.data_url.clone(),
            extra: BTreeMap::new(),
        },
        OcrDocumentKind::DocumentUrl => NativeDocument::DocumentUrl {
            document_url: request.document.data_url.clone(),
            extra: BTreeMap::new(),
        },
    };
    LiteLLMOcrRequest::new(
        &request.model,
        OcrDocumentInput::Document(document),
        None,
        Object::new(),
    )
}
