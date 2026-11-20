use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::credentials::CredentialValues;
use betterlite_core::rag::ingestion::credentials::{CredentialLookup, CredentialLookupError};
use betterlite_core::rag::ingestion::ocr::{OcrError, OcrOperation, OcrRequest, OcrResponse};
use betterlite_core::rag::ingestion::upload::{FetchedResponse, UrlFetch, UrlFetchError};
use betterlite_core::rag::openai_ingestion::{
    AttachFile, CreateStore, ExpiresAfter, FileUploadPort, PortError, UploadFile, VectorFilePort,
    VectorStorePort,
};
use betterlite_oracle_helper_api::error::HelperError;
use serde_json::{Value, json};

enum RecordedCall {
    Create {
        name: String,
        expires_after: Option<ExpiresAfter>,
        api_key: bool,
        api_base: Option<String>,
    },
    Attach {
        vector_store_id: String,
        file_id: String,
        chunking_strategy: String,
        api_key: bool,
        api_base: Option<String>,
    },
    Upload {
        filename: String,
        size_bytes: usize,
        content_type: String,
        purpose: String,
        api_key: bool,
        api_base: Option<String>,
    },
}

#[derive(Default)]
pub(super) struct CallRecorder {
    calls: Mutex<Vec<RecordedCall>>,
}

impl CallRecorder {
    fn push(&self, call: RecordedCall) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }

    pub(super) fn calls_json(&self) -> Result<Vec<Value>, Box<HelperError>> {
        let calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        let mut values = Vec::with_capacity(calls.len());
        for call in calls.iter() {
            values.push(call_json(call)?);
        }
        Ok(values)
    }
}

fn call_json(call: &RecordedCall) -> Result<Value, Box<HelperError>> {
    match call {
        RecordedCall::Create {
            name,
            expires_after,
            api_key,
            api_base,
        } => {
            let expires = expires_after
                .map(|expires| json!({"anchor": expires.anchor, "days": expires.days}));
            Ok(json!({
                "port": "vector_store_create",
                "name": name,
                "expires_after": expires,
                "api_base": api_base,
                "api_key": api_key,
            }))
        }
        RecordedCall::Attach {
            vector_store_id,
            file_id,
            chunking_strategy,
            api_key,
            api_base,
        } => {
            let strategy: Value = serde_json::from_str(chunking_strategy)?;
            Ok(json!({
                "port": "vector_file_attach",
                "vector_store_id": vector_store_id,
                "file_id": file_id,
                "chunking_strategy": strategy,
                "api_base": api_base,
                "api_key": api_key,
            }))
        }
        RecordedCall::Upload {
            filename,
            size_bytes,
            content_type,
            purpose,
            api_key,
            api_base,
        } => Ok(json!({
            "port": "file_upload",
            "filename": filename,
            "size_bytes": size_bytes,
            "content_type": content_type,
            "purpose": purpose,
            "api_base": api_base,
            "api_key": api_key,
        })),
    }
}

// every port failure carries the scripted message so the projection shows which call stopped the store
fn scripted_error(message: &str) -> PortError {
    PortError {
        message: message.to_owned(),
        source: Box::new(io::Error::other(message.to_owned())),
    }
}

pub(super) struct ScriptedStore {
    pub(super) recorder: Arc<CallRecorder>,
    pub(super) response: Option<String>,
    pub(super) failure: Option<String>,
}

#[async_trait]
impl VectorStorePort for ScriptedStore {
    async fn create_store(&self, request: CreateStore<'_>) -> Result<Option<String>, PortError> {
        self.recorder.push(RecordedCall::Create {
            name: request.name.to_owned(),
            expires_after: request.expires_after,
            api_key: request.api_key.is_some(),
            api_base: request.api_base.map(str::to_owned),
        });
        if let Some(message) = &self.failure {
            return Err(scripted_error(message));
        }
        Ok(self.response.clone())
    }
}

pub(super) struct ScriptedFile {
    pub(super) recorder: Arc<CallRecorder>,
    pub(super) failure: Option<String>,
}

#[async_trait]
impl VectorFilePort for ScriptedFile {
    async fn attach(&self, request: AttachFile<'_>) -> Result<(), PortError> {
        self.recorder.push(RecordedCall::Attach {
            vector_store_id: request.vector_store_id.to_owned(),
            file_id: request.file_id.to_owned(),
            chunking_strategy: request.chunking_strategy.get().to_owned(),
            api_key: request.api_key.is_some(),
            api_base: request.api_base.map(str::to_owned),
        });
        if let Some(message) = &self.failure {
            return Err(scripted_error(message));
        }
        Ok(())
    }
}

pub(super) struct ScriptedUpload {
    pub(super) recorder: Arc<CallRecorder>,
    pub(super) id: String,
    pub(super) failure: Option<String>,
}

#[async_trait]
impl FileUploadPort for ScriptedUpload {
    async fn upload(&self, request: UploadFile<'_>) -> Result<String, PortError> {
        self.recorder.push(RecordedCall::Upload {
            filename: request.filename.to_owned(),
            size_bytes: request.content.len(),
            content_type: request.content_type.to_owned(),
            purpose: request.purpose.to_owned(),
            api_key: request.api_key.is_some(),
            api_base: request.api_base.map(str::to_owned),
        });
        if let Some(message) = &self.failure {
            return Err(scripted_error(message));
        }
        Ok(self.id.clone())
    }
}

pub(super) struct NoOcr;

#[async_trait]
impl OcrOperation for NoOcr {
    async fn aocr(&self, _request: OcrRequest) -> Result<OcrResponse, OcrError> {
        Err(OcrError::Failed {
            message: "the store never runs OCR".to_owned(),
        })
    }
}

pub(super) struct NoFetch;

#[async_trait]
impl UrlFetch for NoFetch {
    async fn fetch(&self, _url: &str) -> Result<FetchedResponse, UrlFetchError> {
        Err(UrlFetchError::Failed {
            message: "the store never fetches a URL".to_owned(),
        })
    }
}

pub(super) struct NoCredentials;

impl CredentialLookup for NoCredentials {
    fn credential_values(
        &self,
        credential_name: &str,
    ) -> Result<CredentialValues, CredentialLookupError> {
        Err(CredentialLookupError::NotFound {
            credential_name: credential_name.to_owned(),
        })
    }
}
