pub mod chunk;
pub mod config;
pub mod credentials;
pub mod ocr;
pub mod store;
pub mod upload;

#[path = "ingestion/tests.rs"]
mod ingestion_state;

use std::collections::BTreeMap;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_config::rag::document::ingest::{RAGIngestEmbeddingOptions, RAGIngestOCROptions};
use serde_json::value::RawValue;
use uuid::Uuid;

use chunk::ChunkError;
use config::{IngestionOptions, IngestionSetupError};
use ocr::{OcrDocument, OcrDocumentKind, OcrError, OcrOperation, OcrRequest};
use upload::{FileData, UploadError, UploadedFile, UrlFetch};

pub struct IngestionPorts {
    pub ocr: Arc<dyn OcrOperation>,
    pub uploader: Arc<dyn UrlFetch>,
    pub credentials: Arc<dyn credentials::CredentialLookup>,
}

#[expect(
    clippy::partial_pub_fields,
    reason = "the ingest id and the resolved configs are read by callers while the router, OCR operation and URL fetcher stay internal"
)]
pub struct IngestionState {
    pub ingest_id: String,
    pub ingest_name: Option<String>,
    pub ocr_config: Option<RAGIngestOCROptions>,
    pub chunking_strategy: Box<RawValue>,
    pub embedding_config: Option<RAGIngestEmbeddingOptions>,
    pub vector_store_config: BTreeMap<String, Box<RawValue>>,
    router: Option<Arc<dyn OcrOperation>>,
    ocr_operation: Arc<dyn OcrOperation>,
    uploader: Arc<dyn UrlFetch>,
}

impl IngestionState {
    pub const SUPPORTS_EXISTING_FILE_ID: bool = false;

    pub fn new(
        options: IngestionOptions,
        router: Option<Arc<dyn OcrOperation>>,
        ports: IngestionPorts,
    ) -> Result<Self, IngestionSetupError> {
        let IngestionPorts {
            ocr: ocr_operation,
            uploader,
            credentials,
        } = ports;
        let IngestionOptions {
            name,
            ocr,
            chunking_strategy,
            embedding,
            vector_store,
        } = options;
        let mut vector_store_config = vector_store.unwrap_or_default();
        credentials::load_from_config(&mut vector_store_config, credentials.as_ref())?;
        Ok(Self {
            ingest_id: format!("ingest_{}", Uuid::new_v4()),
            ingest_name: name,
            ocr_config: ocr,
            chunking_strategy: config::chunking_strategy_value(chunking_strategy)?,
            embedding_config: embedding,
            vector_store_config,
            router,
            ocr_operation,
            uploader,
        })
    }

    pub async fn upload(
        &self,
        file_data: Option<FileData>,
        file_url: Option<&str>,
        file_id: Option<&str>,
    ) -> Result<UploadedFile, UploadError> {
        if let Some(data) = file_data {
            return Ok(UploadedFile {
                filename: Some(data.filename),
                file_content: Some(data.content),
                content_type: Some(data.content_type),
                existing_file_id: None,
            });
        }
        if let Some(url) = file_url {
            let fetched = self.uploader.fetch(url).await?;
            if !(200..300).contains(&fetched.status) {
                return Err(UploadError::HttpStatus {
                    status: fetched.status,
                });
            }
            return Ok(UploadedFile {
                filename: Some(upload::filename_from_url(url)),
                content_type: Some(upload::content_type_from_headers(&fetched.headers)),
                file_content: Some(fetched.body),
                existing_file_id: None,
            });
        }
        if let Some(id) = file_id {
            return Ok(UploadedFile {
                filename: None,
                file_content: None,
                content_type: None,
                existing_file_id: Some(id.to_owned()),
            });
        }
        Err(UploadError::MissingSource)
    }

    pub async fn ocr(
        &self,
        file_content: Option<&[u8]>,
        content_type: Option<&str>,
    ) -> Result<Option<String>, OcrError> {
        let Some(config) = &self.ocr_config else {
            return Ok(None);
        };
        let Some(content) = file_content.filter(|bytes| !bytes.is_empty()) else {
            return Ok(None);
        };
        let model = config
            .model
            .clone()
            .unwrap_or_else(|| ocr::DEFAULT_OCR_MODEL.to_owned());
        let kind = if content_type.is_some_and(|value| value.contains("image")) {
            OcrDocumentKind::ImageUrl
        } else {
            OcrDocumentKind::DocumentUrl
        };
        let data_url = format!(
            "data:{};base64,{}",
            content_type.unwrap_or_default(),
            BASE64.encode(content)
        );
        let request = OcrRequest {
            model,
            document: OcrDocument { kind, data_url },
        };
        let operation: &dyn OcrOperation = self
            .router
            .as_deref()
            .unwrap_or_else(|| self.ocr_operation.as_ref());
        let response = operation.aocr(request).await?;
        Ok(ocr::joined_markdown(&response))
    }

    pub fn chunk(
        &self,
        text: Option<&str>,
        file_content: Option<&[u8]>,
        ocr_was_used: bool,
    ) -> Result<Vec<String>, ChunkError> {
        let source = match text {
            Some(value) => Some(value.to_owned()),
            None if ocr_was_used => None,
            None => file_content.and_then(chunk::decode_file_content),
        };
        let Some(resolved_text) = source.filter(|value| !value.is_empty()) else {
            return Ok(Vec::new());
        };
        let plan = chunk::ChunkPlan::from_strategy(&self.chunking_strategy)?;
        chunk::split(&resolved_text, plan)
    }
}
