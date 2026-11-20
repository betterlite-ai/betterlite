#![cfg(test)]

use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::RawValue;

use super::{
    AttachFile, CreateStore, ExpiresAfter, FileUploadPort, OpenAiIngestion, OpenAiIngestionError,
    PortError, UploadFile, VectorFilePort, VectorStorePort,
};
use crate::rag::ingestion::credentials::{CredentialLookup, CredentialLookupError};
use crate::rag::ingestion::ocr::{OcrError, OcrOperation, OcrRequest, OcrResponse};
use crate::rag::ingestion::store::{RagStore as _, StoreRequest};
use crate::rag::ingestion::upload::{FetchedResponse, UrlFetch, UrlFetchError};
use crate::rag::ingestion::{IngestionPorts, IngestionState};
use betterlite_config::credentials::CredentialValues;

struct NoOcr;

#[async_trait]
impl OcrOperation for NoOcr {
    async fn aocr(&self, _request: OcrRequest) -> Result<OcrResponse, OcrError> {
        Err(OcrError::Failed {
            message: "these tests never run OCR".to_owned(),
        })
    }
}

struct NoFetch;

#[async_trait]
impl UrlFetch for NoFetch {
    async fn fetch(&self, _url: &str) -> Result<FetchedResponse, UrlFetchError> {
        Err(UrlFetchError::Failed {
            message: "these tests never fetch a URL".to_owned(),
        })
    }
}

struct NoCredentials;

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

#[derive(Debug, Clone, PartialEq, Eq)]
enum RecordedCall {
    Create {
        name: String,
        expires_after: Option<ExpiresAfter>,
        api_key: Option<String>,
        api_base: Option<String>,
    },
    Attach {
        vector_store_id: String,
        file_id: String,
        chunking_strategy: String,
        api_key: Option<String>,
        api_base: Option<String>,
    },
    Upload {
        filename: String,
        content: Vec<u8>,
        content_type: String,
        purpose: String,
        api_key: Option<String>,
        api_base: Option<String>,
    },
}

#[derive(Default)]
struct Calls(Mutex<Vec<RecordedCall>>);

impl Calls {
    fn push(&self, call: RecordedCall) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }

    fn snapshot(&self) -> Vec<RecordedCall> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn port_failure(message: &str) -> PortError {
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
                self.calls.push($record);
                self.$reply()
            }
        }
    };
}

struct FakeStore {
    calls: Arc<Calls>,
    id: Option<String>,
    failure: Option<String>,
}

impl FakeStore {
    fn store_reply(&self) -> Result<Option<String>, PortError> {
        if let Some(message) = &self.failure {
            return Err(port_failure(message));
        }
        Ok(self.id.clone())
    }
}

scripted_port!(
    VectorStorePort,
    create_store,
    FakeStore,
    request: CreateStore<'_>,
    Result<Option<String>, PortError>,
    store_reply,
    RecordedCall::Create {
        name: request.name.to_owned(),
        expires_after: request.expires_after,
        api_key: request.api_key.map(|key| key.expose_secret().to_owned()),
        api_base: request.api_base.map(str::to_owned),
    }
);

struct FakeFile {
    calls: Arc<Calls>,
    failure: Option<String>,
}

impl FakeFile {
    fn file_reply(&self) -> Result<(), PortError> {
        if let Some(message) = &self.failure {
            return Err(port_failure(message));
        }
        Ok(())
    }
}

scripted_port!(
    VectorFilePort,
    attach,
    FakeFile,
    request: AttachFile<'_>,
    Result<(), PortError>,
    file_reply,
    RecordedCall::Attach {
        vector_store_id: request.vector_store_id.to_owned(),
        file_id: request.file_id.to_owned(),
        chunking_strategy: request.chunking_strategy.get().to_owned(),
        api_key: request.api_key.map(|key| key.expose_secret().to_owned()),
        api_base: request.api_base.map(str::to_owned),
    }
);

struct FakeUpload {
    calls: Arc<Calls>,
    failure: Option<String>,
}

impl FakeUpload {
    fn upload_reply(&self) -> Result<String, PortError> {
        if let Some(message) = &self.failure {
            return Err(port_failure(message));
        }
        Ok("file_new".to_owned())
    }
}

scripted_port!(
    FileUploadPort,
    upload,
    FakeUpload,
    request: UploadFile<'_>,
    Result<String, PortError>,
    upload_reply,
    RecordedCall::Upload {
        filename: request.filename.to_owned(),
        content: request.content.to_vec(),
        content_type: request.content_type.to_owned(),
        purpose: request.purpose.to_owned(),
        api_key: request.api_key.map(|key| key.expose_secret().to_owned()),
        api_base: request.api_base.map(str::to_owned),
    }
);

struct Script {
    create_id: Option<String>,
    create_failure: Option<String>,
    attach_failure: Option<String>,
    upload_failure: Option<String>,
}

impl Default for Script {
    fn default() -> Self {
        Self {
            create_id: Some("vs_created".to_owned()),
            create_failure: None,
            attach_failure: None,
            upload_failure: None,
        }
    }
}

struct Harness {
    ingestion: OpenAiIngestion,
    calls: Arc<Calls>,
}

fn state(options: &str) -> IngestionState {
    IngestionState::new(
        serde_json::from_str(options).expect("the ingest options parse"),
        None,
        IngestionPorts {
            ocr: Arc::new(NoOcr),
            uploader: Arc::new(NoFetch),
            credentials: Arc::new(NoCredentials),
        },
    )
    .expect("the ingest state builds")
}

fn harness(options: &str) -> Harness {
    harness_with(Script::default(), options)
}

fn harness_with(script: Script, options: &str) -> Harness {
    let calls = Arc::new(Calls::default());
    let ingestion = OpenAiIngestion {
        state: state(options),
        vector_store: Arc::new(FakeStore {
            calls: Arc::clone(&calls),
            id: script.create_id,
            failure: script.create_failure,
        }),
        vector_file: Arc::new(FakeFile {
            calls: Arc::clone(&calls),
            failure: script.attach_failure,
        }),
        file_upload: Arc::new(FakeUpload {
            calls: Arc::clone(&calls),
            failure: script.upload_failure,
        }),
    };
    Harness { ingestion, calls }
}

fn request<'request>(
    file_content: Option<&'request [u8]>,
    filename: Option<&'request str>,
    content_type: Option<&'request str>,
    existing_file_id: Option<&'request str>,
) -> StoreRequest<'request> {
    StoreRequest {
        file_content,
        filename,
        content_type,
        chunks: &[],
        embeddings: None,
        existing_file_id,
    }
}

#[test]
fn flags_support_existing_files_and_embed_nothing() {
    let harness = harness(r#"{}"#);
    let chunks = vec!["one".to_owned(), "two".to_owned()];
    assert!(OpenAiIngestion::SUPPORTS_EXISTING_FILE_ID);
    assert!(harness.ingestion.embed(&chunks).is_none());
    assert!(harness.ingestion.embed(&[]).is_none());
}

#[tokio::test]
async fn existing_file_without_store_id_is_rejected() {
    let harness = harness(r#"{}"#);
    let error = harness
        .ingestion
        .store(request(None, None, None, Some("file_1")))
        .await
        .expect_err("the missing store id");
    assert!(matches!(&error, OpenAiIngestionError::MissingVectorStoreId));
    assert_eq!(
        error.to_string(),
        "vector_store_id is required when ingesting an existing file_id"
    );
    assert!(harness.calls.snapshot().is_empty());
}

#[tokio::test]
async fn existing_file_with_an_empty_store_id_is_rejected() {
    let harness = harness(r#"{"vector_store": {"vector_store_id": ""}}"#);
    let error = harness
        .ingestion
        .store(request(None, None, None, Some("file_1")))
        .await
        .expect_err("the empty store id counts as absent");
    assert!(matches!(&error, OpenAiIngestionError::MissingVectorStoreId));
    assert!(harness.calls.snapshot().is_empty());
}

#[tokio::test]
async fn existing_file_attaches_with_chunking_and_skips_upload() {
    let harness = harness(
        r#"{"chunking_strategy": {"chunk_size": 900, "chunk_overlap": 50}, "vector_store": {"vector_store_id": "vs_1"}}"#,
    );
    let (store_id, file_id) = harness
        .ingestion
        .store(request(None, None, None, Some("file_1")))
        .await
        .expect("the store");
    assert_eq!(store_id.as_deref(), Some("vs_1"));
    assert_eq!(file_id.as_deref(), Some("file_1"));
    assert_eq!(
        harness.calls.snapshot(),
        vec![RecordedCall::Attach {
            vector_store_id: "vs_1".to_owned(),
            file_id: "file_1".to_owned(),
            chunking_strategy: r#"{"chunk_overlap":50,"chunk_size":900}"#.to_owned(),
            api_key: None,
            api_base: None,
        }]
    );
}

#[tokio::test]
async fn existing_file_with_an_empty_id_creates_a_store() {
    let harness = harness(r#"{}"#);
    let (store_id, file_id) = harness
        .ingestion
        .store(request(None, None, None, Some("")))
        .await
        .expect("the store");
    assert_eq!(store_id.as_deref(), Some("vs_created"));
    assert!(file_id.is_none());
    assert_eq!(
        harness.calls.snapshot(),
        vec![RecordedCall::Create {
            name: "litellm-rag-ingest".to_owned(),
            expires_after: None,
            api_key: None,
            api_base: None,
        }]
    );
}

#[tokio::test]
async fn store_create_defaults_the_name_and_sends_no_expiry() {
    let harness = harness(r#"{}"#);
    let (store_id, file_id) = harness
        .ingestion
        .store(request(None, None, None, None))
        .await
        .expect("the store");
    assert_eq!(store_id.as_deref(), Some("vs_created"));
    assert!(file_id.is_none());
    assert_eq!(
        harness.calls.snapshot(),
        vec![RecordedCall::Create {
            name: "litellm-rag-ingest".to_owned(),
            expires_after: None,
            api_key: None,
            api_base: None,
        }]
    );
}

#[tokio::test]
async fn store_create_uses_the_ingest_name() {
    let harness = harness(r#"{"name": "docs"}"#);
    harness
        .ingestion
        .store(request(None, None, None, None))
        .await
        .expect("the store");
    assert_eq!(
        harness.calls.snapshot(),
        vec![RecordedCall::Create {
            name: "docs".to_owned(),
            expires_after: None,
            api_key: None,
            api_base: None,
        }]
    );
}

#[tokio::test]
async fn store_create_falls_back_for_an_empty_ingest_name() {
    let harness = harness(r#"{"name": ""}"#);
    harness
        .ingestion
        .store(request(None, None, None, None))
        .await
        .expect("the store");
    assert_eq!(
        harness.calls.snapshot(),
        vec![RecordedCall::Create {
            name: "litellm-rag-ingest".to_owned(),
            expires_after: None,
            api_key: None,
            api_base: None,
        }]
    );
}

#[tokio::test]
async fn store_create_ttl_days_become_last_active_at_expiry() {
    let harness = harness(r#"{"vector_store": {"ttl_days": 30}}"#);
    harness
        .ingestion
        .store(request(None, None, None, None))
        .await
        .expect("the store");
    assert_eq!(
        harness.calls.snapshot(),
        vec![RecordedCall::Create {
            name: "litellm-rag-ingest".to_owned(),
            expires_after: Some(ExpiresAfter {
                anchor: "last_active_at",
                days: 30,
            }),
            api_key: None,
            api_base: None,
        }]
    );
}

#[rstest]
#[case::zero("0")]
#[case::absent("null")]
#[tokio::test]
async fn store_create_sends_no_expiry_without_ttl_days(#[case] ttl: &str) {
    let harness = harness(&format!(r#"{{"vector_store": {{"ttl_days": {ttl}}}}}"#));
    harness
        .ingestion
        .store(request(None, None, None, None))
        .await
        .expect("the store");
    assert_eq!(
        harness.calls.snapshot(),
        vec![RecordedCall::Create {
            name: "litellm-rag-ingest".to_owned(),
            expires_after: None,
            api_key: None,
            api_base: None,
        }]
    );
}

#[tokio::test]
async fn store_create_sends_a_negative_ttl_days() {
    let harness = harness(r#"{"vector_store": {"ttl_days": -3}}"#);
    harness
        .ingestion
        .store(request(None, None, None, None))
        .await
        .expect("the store");
    assert_eq!(
        harness.calls.snapshot(),
        vec![RecordedCall::Create {
            name: "litellm-rag-ingest".to_owned(),
            expires_after: Some(ExpiresAfter {
                anchor: "last_active_at",
                days: -3,
            }),
            api_key: None,
            api_base: None,
        }]
    );
}

#[tokio::test]
async fn store_create_keeps_an_existing_store() {
    let harness = harness(r#"{"vector_store": {"vector_store_id": "vs_1"}}"#);
    let (store_id, file_id) = harness
        .ingestion
        .store(request(None, None, None, None))
        .await
        .expect("the store");
    assert_eq!(store_id.as_deref(), Some("vs_1"));
    assert!(file_id.is_none());
    assert!(harness.calls.snapshot().is_empty());
}

#[tokio::test]
async fn store_create_without_an_id_uploads_nothing() {
    let script = Script {
        create_id: None,
        create_failure: None,
        attach_failure: None,
        upload_failure: None,
    };
    let harness = harness_with(script, r#"{}"#);
    let (store_id, file_id) = harness
        .ingestion
        .store(request(Some(b"hello"), Some("a.txt"), None, None))
        .await
        .expect("the store");
    assert!(store_id.is_none());
    assert!(file_id.is_none());
    assert_eq!(
        harness.calls.snapshot(),
        vec![RecordedCall::Create {
            name: "litellm-rag-ingest".to_owned(),
            expires_after: None,
            api_key: None,
            api_base: None,
        }]
    );
}

#[tokio::test]
async fn store_create_with_an_empty_id_uploads_nothing() {
    let script = Script {
        create_id: Some(String::new()),
        create_failure: None,
        attach_failure: None,
        upload_failure: None,
    };
    let harness = harness_with(script, r#"{}"#);
    let (store_id, file_id) = harness
        .ingestion
        .store(request(Some(b"hello"), Some("a.txt"), None, None))
        .await
        .expect("the store");
    assert_eq!(store_id.as_deref(), Some(""));
    assert!(file_id.is_none());
    assert_eq!(harness.calls.snapshot().len(), 1);
}

#[tokio::test]
async fn upload_and_attach_return_the_uploaded_file() {
    let harness = harness(r#"{"vector_store": {"vector_store_id": "vs_1"}}"#);
    let (store_id, file_id) = harness
        .ingestion
        .store(request(Some(b"hello"), Some("a.txt"), None, None))
        .await
        .expect("the store");
    assert_eq!(store_id.as_deref(), Some("vs_1"));
    assert_eq!(file_id.as_deref(), Some("file_new"));
    assert_eq!(
        harness.calls.snapshot(),
        vec![
            RecordedCall::Upload {
                filename: "a.txt".to_owned(),
                content: b"hello".to_vec(),
                content_type: "application/octet-stream".to_owned(),
                purpose: "assistants".to_owned(),
                api_key: None,
                api_base: None,
            },
            RecordedCall::Attach {
                vector_store_id: "vs_1".to_owned(),
                file_id: "file_new".to_owned(),
                chunking_strategy: r#"{"type":"auto"}"#.to_owned(),
                api_key: None,
                api_base: None,
            },
        ]
    );
}

#[tokio::test]
async fn upload_keeps_a_given_content_type() {
    let harness = harness(r#"{"vector_store": {"vector_store_id": "vs_1"}}"#);
    harness
        .ingestion
        .store(request(
            Some(b"hello"),
            Some("a.txt"),
            Some("text/plain"),
            None,
        ))
        .await
        .expect("the store");
    assert!(matches!(
        harness.calls.snapshot().as_slice(),
        [RecordedCall::Upload { content_type, .. }] if content_type == "text/plain"
    ));
}

#[tokio::test]
async fn upload_empty_content_type_falls_back() {
    let harness = harness(r#"{"vector_store": {"vector_store_id": "vs_1"}}"#);
    harness
        .ingestion
        .store(request(Some(b"hello"), Some("a.txt"), Some(""), None))
        .await
        .expect("the store");
    assert!(matches!(
        harness.calls.snapshot().as_slice(),
        [RecordedCall::Upload { content_type, .. }] if content_type == "application/octet-stream"
    ));
}

#[tokio::test]
async fn upload_empty_bytes_still_uploads() {
    let harness = harness(r#"{"vector_store": {"vector_store_id": "vs_1"}}"#);
    let (_, file_id) = harness
        .ingestion
        .store(request(Some(b""), Some("a.txt"), None, None))
        .await
        .expect("the store");
    assert_eq!(file_id.as_deref(), Some("file_new"));
    assert_eq!(harness.calls.snapshot().len(), 2);
}

#[tokio::test]
async fn upload_ignores_chunks_and_embeddings() {
    let harness = harness(r#"{"vector_store": {"vector_store_id": "vs_1"}}"#);
    let chunks: &[String] = &["one".to_owned()];
    let embeddings: &[Vec<f64>] = &[vec![0.5_f64]];
    let (store_id, file_id) = harness
        .ingestion
        .store(StoreRequest {
            file_content: Some(b"hello"),
            filename: Some("a.txt"),
            content_type: None,
            chunks,
            embeddings: Some(embeddings),
            existing_file_id: None,
        })
        .await
        .expect("the store");
    assert_eq!(store_id.as_deref(), Some("vs_1"));
    assert_eq!(file_id.as_deref(), Some("file_new"));
    assert_eq!(harness.calls.snapshot().len(), 2);
}

#[rstest]
#[case::without_content(None, Some("a.txt"))]
#[case::without_filename(Some(&b"hello"[..]), None)]
#[case::with_empty_filename(Some(&b"hello"[..]), Some(""))]
#[tokio::test]
async fn upload_skipped_without_content_or_filename(
    #[case] content: Option<&[u8]>,
    #[case] filename: Option<&str>,
) {
    let harness = harness(r#"{"vector_store": {"vector_store_id": "vs_1"}}"#);
    let (store_id, file_id) = harness
        .ingestion
        .store(request(content, filename, None, None))
        .await
        .expect("the store");
    assert_eq!(store_id.as_deref(), Some("vs_1"));
    assert!(file_id.is_none());
    assert!(harness.calls.snapshot().is_empty());
}

#[tokio::test]
async fn upload_skips_an_empty_filename() {
    let harness = harness(r#"{"vector_store": {"vector_store_id": "vs_1"}}"#);
    let (store_id, file_id) = harness
        .ingestion
        .store(request(Some(b"hello"), Some(""), None, None))
        .await
        .expect("the store");
    assert_eq!(store_id.as_deref(), Some("vs_1"));
    assert!(file_id.is_none());
    assert!(harness.calls.snapshot().is_empty());
}

#[tokio::test]
async fn credentials_reach_every_port() {
    let harness = harness(
        r#"{"vector_store": {"api_key": "sk-live", "api_base": "https://openai.test/v1"}}"#,
    );
    let (store_id, file_id) = harness
        .ingestion
        .store(request(Some(b"hello"), Some("a.txt"), None, None))
        .await
        .expect("the store");
    assert_eq!(store_id.as_deref(), Some("vs_created"));
    assert_eq!(file_id.as_deref(), Some("file_new"));
    assert_eq!(
        harness.calls.snapshot(),
        vec![
            RecordedCall::Create {
                name: "litellm-rag-ingest".to_owned(),
                expires_after: None,
                api_key: Some("sk-live".to_owned()),
                api_base: Some("https://openai.test/v1".to_owned()),
            },
            RecordedCall::Upload {
                filename: "a.txt".to_owned(),
                content: b"hello".to_vec(),
                content_type: "application/octet-stream".to_owned(),
                purpose: "assistants".to_owned(),
                api_key: Some("sk-live".to_owned()),
                api_base: Some("https://openai.test/v1".to_owned()),
            },
            RecordedCall::Attach {
                vector_store_id: "vs_created".to_owned(),
                file_id: "file_new".to_owned(),
                chunking_strategy: r#"{"type":"auto"}"#.to_owned(),
                api_key: Some("sk-live".to_owned()),
                api_base: Some("https://openai.test/v1".to_owned()),
            },
        ]
    );
}

#[test]
fn credentials_stay_out_of_debug_output() {
    let secret = SecretString::from("sk-live-secret".to_owned());
    let chunking: Box<RawValue> =
        serde_json::from_str(r#"{"type": "auto"}"#).expect("the chunking strategy");
    let created = CreateStore {
        name: "docs",
        expires_after: None,
        api_key: Some(&secret),
        api_base: None,
    };
    let attached = AttachFile {
        vector_store_id: "vs_1",
        file_id: "file_1",
        chunking_strategy: &chunking,
        api_key: Some(&secret),
        api_base: None,
    };
    let uploaded = UploadFile {
        filename: "a.txt",
        content: b"hello",
        content_type: "text/plain",
        purpose: "assistants",
        api_key: Some(&secret),
        api_base: None,
    };
    assert!(!format!("{created:?}").contains("sk-live-secret"));
    assert!(!format!("{attached:?}").contains("sk-live-secret"));
    assert!(!format!("{uploaded:?}").contains("sk-live-secret"));
}

#[tokio::test]
async fn credentials_stay_out_of_error_text() {
    let script = Script {
        create_id: Some("vs_created".to_owned()),
        create_failure: None,
        attach_failure: None,
        upload_failure: Some("upload failed".to_owned()),
    };
    let harness = harness_with(
        script,
        r#"{"vector_store": {"vector_store_id": "vs_1", "api_key": "sk-live-secret"}}"#,
    );
    let error = harness
        .ingestion
        .store(request(Some(b"hello"), Some("a.txt"), None, None))
        .await
        .expect_err("the upload failure");
    assert_eq!(error.to_string(), "upload failed");
    assert!(!format!("{error:?}").contains("sk-live-secret"));
}

#[tokio::test]
async fn port_error_stops_at_the_failing_create() {
    let script = Script {
        create_id: None,
        create_failure: Some("create refused".to_owned()),
        attach_failure: None,
        upload_failure: None,
    };
    let harness = harness_with(script, r#"{}"#);
    let error = harness
        .ingestion
        .store(request(Some(b"hello"), Some("a.txt"), None, None))
        .await
        .expect_err("the create failure");
    assert!(matches!(&error, OpenAiIngestionError::Port(_)));
    assert_eq!(error.to_string(), "create refused");
    assert_eq!(harness.calls.snapshot().len(), 1);
}

#[tokio::test]
async fn port_error_keeps_the_created_store_and_skips_attach() {
    let script = Script {
        create_id: Some("vs_created".to_owned()),
        create_failure: None,
        attach_failure: None,
        upload_failure: Some("upload failed".to_owned()),
    };
    let harness = harness_with(script, r#"{}"#);
    let error = harness
        .ingestion
        .store(request(Some(b"hello"), Some("a.txt"), None, None))
        .await
        .expect_err("the upload failure");
    assert!(matches!(&error, OpenAiIngestionError::Port(_)));
    assert_eq!(
        harness.calls.snapshot(),
        vec![
            RecordedCall::Create {
                name: "litellm-rag-ingest".to_owned(),
                expires_after: None,
                api_key: None,
                api_base: None,
            },
            RecordedCall::Upload {
                filename: "a.txt".to_owned(),
                content: b"hello".to_vec(),
                content_type: "application/octet-stream".to_owned(),
                purpose: "assistants".to_owned(),
                api_key: None,
                api_base: None,
            },
        ]
    );
}

#[tokio::test]
async fn port_error_stops_at_the_failing_attach() {
    let script = Script {
        create_id: Some("vs_created".to_owned()),
        create_failure: None,
        attach_failure: Some("attach refused".to_owned()),
        upload_failure: None,
    };
    let harness = harness_with(script, r#"{"vector_store": {"vector_store_id": "vs_1"}}"#);
    let error = harness
        .ingestion
        .store(request(None, None, None, Some("file_1")))
        .await
        .expect_err("the attach failure");
    assert!(matches!(&error, OpenAiIngestionError::Port(_)));
    assert_eq!(error.to_string(), "attach refused");
    assert_eq!(harness.calls.snapshot().len(), 1);
}
