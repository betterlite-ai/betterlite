#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_config::rag::document::ingest::RAGIngestOCROptions;
use betterlite_core::rag::ingestion::config::IngestionOptions;
use betterlite_core::rag::ingestion::ocr::{
    OcrDocumentKind, OcrError, OcrOperation, OcrRequest, OcrResponse,
};
use betterlite_core::rag::ingestion::upload::{FetchedResponse, UrlFetch, UrlFetchError};
use betterlite_core::rag::ingestion::{IngestionPorts, IngestionState};
use betterlite_ocr::json::Object;
use betterlite_ocr::ocr_document::OcrDocument as NativeDocument;
use betterlite_ocr::ocr_error::OcrError as NativeOcrError;
use betterlite_ocr::ocr_input::{LiteLLMOcrRequest, OcrDocumentInput};
use betterlite_ocr::ocr_provider::OcrConfigKind;
use betterlite_ocr::response::{OCRPage, OCRResponse};

use super::{OcrBackend, OcrCall};
use crate::credentials::registry::CredentialRegistry;
use crate::rag_step_ports::credential::CredentialLookup as RegistryCredentialLookup;

struct ScriptedRouter {
    requests: Mutex<Vec<OcrRequest>>,
    pages: Vec<String>,
}

#[async_trait]
impl OcrOperation for ScriptedRouter {
    async fn aocr(&self, request: OcrRequest) -> Result<OcrResponse, OcrError> {
        self.requests.lock().unwrap().push(request);
        Ok(OcrResponse::from_pages(self.pages.clone()))
    }
}

#[derive(Clone)]
struct ScriptedBackend {
    requests: Arc<Mutex<Vec<LiteLLMOcrRequest<OcrDocumentInput>>>>,
    pages: Arc<Vec<(i64, String)>>,
    failure: Arc<Mutex<Option<NativeOcrError>>>,
}

#[async_trait]
impl OcrBackend for ScriptedBackend {
    async fn aocr(
        &self,
        request: LiteLLMOcrRequest<OcrDocumentInput>,
    ) -> Result<OCRResponse, NativeOcrError> {
        self.requests.lock().unwrap().push(request);
        match self.failure.lock().unwrap().take() {
            Some(error) => Err(error),
            None => Ok(native_answer(&self.pages)),
        }
    }
}

struct NoFetch;

#[async_trait]
impl UrlFetch for NoFetch {
    async fn fetch(&self, _url: &str) -> Result<FetchedResponse, UrlFetchError> {
        Err(UrlFetchError::Failed {
            message: "the OCR state never fetches a file".to_owned(),
        })
    }
}

fn scripted_backend(pages: &[(i64, &str)]) -> ScriptedBackend {
    ScriptedBackend {
        requests: Arc::new(Mutex::new(Vec::new())),
        pages: Arc::new(
            pages
                .iter()
                .map(|(index, markdown)| (*index, (*markdown).to_owned()))
                .collect(),
        ),
        failure: Arc::new(Mutex::new(None)),
    }
}

fn failing_backend(error: NativeOcrError) -> ScriptedBackend {
    ScriptedBackend {
        requests: Arc::new(Mutex::new(Vec::new())),
        pages: Arc::new(Vec::new()),
        failure: Arc::new(Mutex::new(Some(error))),
    }
}

fn native_answer(pages: &[(i64, String)]) -> OCRResponse {
    OCRResponse {
        pages: pages
            .iter()
            .map(|(index, markdown)| OCRPage {
                index: *index,
                markdown: markdown.clone(),
                images: None,
                dimensions: None,
                extra: Object::new(),
            })
            .collect(),
        model: "mistral-ocr-latest".to_owned(),
        document_annotation: None,
        usage_info: None,
        content: None,
        tables: None,
        key_value_pairs: None,
        object: "ocr".to_owned(),
        extra: Object::new(),
        provider_native_response: None,
    }
}

fn ingestion_state(
    router: Option<Arc<dyn OcrOperation>>,
    backend: &ScriptedBackend,
    model: Option<&str>,
) -> IngestionState {
    let options = IngestionOptions {
        name: None,
        ocr: Some(RAGIngestOCROptions {
            model: model.map(str::to_owned),
        }),
        chunking_strategy: None,
        embedding: None,
        vector_store: None,
    };
    let ports = IngestionPorts {
        ocr: Arc::new(OcrCall {
            backend: backend.clone(),
        }),
        uploader: Arc::new(NoFetch),
        credentials: Arc::new(RegistryCredentialLookup {
            registry: Arc::new(CredentialRegistry::default()),
        }),
    };
    IngestionState::new(options, router, ports).expect("the ingestion state builds")
}

#[tokio::test]
async fn ocr_call_prefers_the_router_and_returns_its_pages_in_order() {
    let router = Arc::new(ScriptedRouter {
        requests: Mutex::new(Vec::new()),
        pages: vec!["router first".to_owned(), "router second".to_owned()],
    });
    let port: Arc<dyn OcrOperation> = Arc::clone(&router);
    let backend = scripted_backend(&[]);
    let state = ingestion_state(Some(port), &backend, Some("mistral/mistral-ocr-latest"));
    let text = state
        .ocr(Some(b"%PDF-1.7"), Some("image/png"))
        .await
        .expect("the OCR call succeeds");
    assert_eq!(text.as_deref(), Some("router first\n\nrouter second"));
    let requests = router.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].model, "mistral/mistral-ocr-latest");
    assert_eq!(requests[0].document.kind, OcrDocumentKind::ImageUrl);
    assert_eq!(
        requests[0].document.data_url,
        format!("data:image/png;base64,{}", STANDARD.encode(b"%PDF-1.7"))
    );
    assert!(backend.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn ocr_call_uses_the_native_operation_without_a_router() {
    let backend = scripted_backend(&[(5, "page five"), (1, "page one")]);
    let state = ingestion_state(None, &backend, Some("mistral/mistral-ocr-latest"));
    let text = state
        .ocr(Some(b"%PDF"), Some("application/pdf"))
        .await
        .expect("the OCR call succeeds");
    assert_eq!(text.as_deref(), Some("page five\n\npage one"));
    let requests = backend.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.model, "mistral-ocr-latest");
    assert!(request.custom_llm_provider.is_none());
    assert!(request.optional_params.is_empty());
    assert_eq!(request.config, OcrConfigKind::Mistral);
    assert_eq!(
        request.document,
        OcrDocumentInput::Document(NativeDocument::DocumentUrl {
            document_url: format!("data:application/pdf;base64,{}", STANDARD.encode(b"%PDF")),
            extra: BTreeMap::new(),
        })
    );
}

#[tokio::test]
async fn ocr_call_passes_the_default_model_to_the_native_operation() {
    let backend = scripted_backend(&[(0, "page")]);
    let state = ingestion_state(None, &backend, None);
    let text = state
        .ocr(Some(b"content"), Some("text/plain"))
        .await
        .expect("the OCR call succeeds");
    assert_eq!(text.as_deref(), Some("page"));
    let requests = backend.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].model, "mistral-ocr-latest");
}

#[tokio::test]
async fn ocr_call_reports_a_native_request_that_cannot_be_built() {
    let backend = scripted_backend(&[]);
    let state = ingestion_state(None, &backend, Some("unknown-provider/ocr-model"));
    let error = state
        .ocr(Some(b"%PDF"), Some("application/pdf"))
        .await
        .expect_err("the model names no OCR provider");
    assert_eq!(
        error,
        OcrError::Failed {
            message: "invalid OCR provider: unknown-provider".to_owned()
        }
    );
    assert!(backend.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn ocr_call_reports_a_native_operation_failure() {
    let backend = failing_backend(NativeOcrError::InvalidProvider("broken".to_owned()));
    let state = ingestion_state(None, &backend, Some("mistral/mistral-ocr-latest"));
    let error = state
        .ocr(Some(b"%PDF"), Some("application/pdf"))
        .await
        .expect_err("the native operation fails");
    assert_eq!(
        error,
        OcrError::Failed {
            message: "invalid OCR provider: broken".to_owned()
        }
    );
    assert_eq!(backend.requests.lock().unwrap().len(), 1);
}
