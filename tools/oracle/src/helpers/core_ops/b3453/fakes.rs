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

pub(super) enum RecordedCall {
    Create(CreateCall),
    Attach(AttachCall),
    Upload(UploadCall),
}

pub(super) struct CreateCall {
    name: String,
    expires_after: Option<ExpiresAfter>,
    api_key: bool,
    api_base: Option<String>,
}

pub(super) struct AttachCall {
    vector_store_id: String,
    file_id: String,
    chunking_strategy: String,
    api_key: bool,
    api_base: Option<String>,
}

pub(super) struct UploadCall {
    filename: String,
    size_bytes: usize,
    content_type: String,
    purpose: String,
    api_key: bool,
    api_base: Option<String>,
}

#[derive(Default)]
pub(super) struct CallRecorder {
    pub(super) calls: Mutex<Vec<RecordedCall>>,
}

impl CallRecorder {
    fn push(&self, call: RecordedCall) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }
}

pub(super) fn call_json(call: &RecordedCall) -> Result<Value, Box<HelperError>> {
    match call {
        RecordedCall::Create(created) => Ok(json!({
            "port": "vector_store_create",
            "name": created.name,
            "expires_after": created.expires_after.map(|expires| {
                json!({"anchor": expires.anchor, "days": expires.days})
            }),
            "api_base": created.api_base,
            "api_key": created.api_key,
        })),
        RecordedCall::Attach(attached) => {
            let strategy: Value = serde_json::from_str(&attached.chunking_strategy)?;
            Ok(json!({
                "port": "vector_file_attach",
                "vector_store_id": attached.vector_store_id,
                "file_id": attached.file_id,
                "chunking_strategy": strategy,
                "api_base": attached.api_base,
                "api_key": attached.api_key,
            }))
        }
        RecordedCall::Upload(uploaded) => Ok(json!({
            "port": "file_upload",
            "filename": uploaded.filename,
            "size_bytes": uploaded.size_bytes,
            "content_type": uploaded.content_type,
            "purpose": uploaded.purpose,
            "api_base": uploaded.api_base,
            "api_key": uploaded.api_key,
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

macro_rules! scripted_port {
    (
        $port:ident, $method:ident, $fake:ident,
        $request:ident: $request_type:ty, $result:ty, $reply:ident,
        $record:expr $(,)?
    ) => {
        #[async_trait]
        impl $port for $fake {
            async fn $method(&self, $request: $request_type) -> $result {
                self.recorder.push($record);
                self.$reply()
            }
        }
    };
}

pub(super) struct ScriptedStore {
    pub(super) recorder: Arc<CallRecorder>,
    pub(super) response: Option<String>,
    pub(super) failure: Option<String>,
}

impl ScriptedStore {
    fn store_reply(&self) -> Result<Option<String>, PortError> {
        if let Some(message) = &self.failure {
            return Err(scripted_error(message));
        }
        Ok(self.response.clone())
    }
}

scripted_port!(
    VectorStorePort,
    create_store,
    ScriptedStore,
    request: CreateStore<'_>,
    Result<Option<String>, PortError>,
    store_reply,
    RecordedCall::Create(CreateCall {
        name: request.name.to_owned(),
        expires_after: request.expires_after,
        api_key: request.api_key.is_some(),
        api_base: request.api_base.map(str::to_owned),
    })
);

pub(super) struct ScriptedFile {
    pub(super) recorder: Arc<CallRecorder>,
    pub(super) failure: Option<String>,
}

impl ScriptedFile {
    fn file_reply(&self) -> Result<(), PortError> {
        if let Some(message) = &self.failure {
            return Err(scripted_error(message));
        }
        Ok(())
    }
}

scripted_port!(
    VectorFilePort,
    attach,
    ScriptedFile,
    request: AttachFile<'_>,
    Result<(), PortError>,
    file_reply,
    RecordedCall::Attach(AttachCall {
        vector_store_id: request.vector_store_id.to_owned(),
        file_id: request.file_id.to_owned(),
        chunking_strategy: request.chunking_strategy.get().to_owned(),
        api_key: request.api_key.is_some(),
        api_base: request.api_base.map(str::to_owned),
    })
);

pub(super) struct ScriptedUpload {
    pub(super) recorder: Arc<CallRecorder>,
    pub(super) id: String,
    pub(super) failure: Option<String>,
}

impl ScriptedUpload {
    fn upload_reply(&self) -> Result<String, PortError> {
        if let Some(message) = &self.failure {
            return Err(scripted_error(message));
        }
        Ok(self.id.clone())
    }
}

scripted_port!(
    FileUploadPort,
    upload,
    ScriptedUpload,
    request: UploadFile<'_>,
    Result<String, PortError>,
    upload_reply,
    RecordedCall::Upload(UploadCall {
        filename: request.filename.to_owned(),
        size_bytes: request.content.len(),
        content_type: request.content_type.to_owned(),
        purpose: request.purpose.to_owned(),
        api_key: request.api_key.is_some(),
        api_base: request.api_base.map(str::to_owned),
    })
);

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
