#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_config::credentials::CredentialValues;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::rag::document::ingest::RAGIngestOCROptions;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};
use http::{HeaderMap, HeaderValue};
use rstest::rstest;
use thiserror::Error;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

use super::chunk::{ChunkError, ChunkPlan};
use super::config::{IngestionOptions, IngestionSetupError};
use super::credentials::{CredentialLookup, CredentialLookupError};
use super::ocr::{OcrDocumentKind, OcrError, OcrOperation, OcrRequest, OcrResponse};
use super::store::{RagStore, StoreRequest, StoreResult};
use super::upload::{
    DEFAULT_CONTENT_TYPE, FetchedResponse, FileData, HttpUrlFetch, UploadError, UrlFetch,
    UrlFetchError, content_type_from_headers, filename_from_url,
};
use super::{IngestionPorts, IngestionState};

#[derive(Default)]
struct FakeOcr {
    calls: Mutex<Vec<OcrRequest>>,
    pages: Vec<String>,
    failure: Option<String>,
}

impl FakeOcr {
    fn returning(pages: Vec<String>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            pages,
            failure: None,
        }
    }

    fn failing(message: &str) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            pages: Vec::new(),
            failure: Some(message.to_owned()),
        }
    }
}

#[async_trait]
impl OcrOperation for FakeOcr {
    async fn aocr(&self, request: OcrRequest) -> Result<OcrResponse, OcrError> {
        self.calls.lock().unwrap().push(request);
        if let Some(message) = &self.failure {
            return Err(OcrError::Failed {
                message: message.clone(),
            });
        }
        Ok(OcrResponse::from_pages(self.pages.clone()))
    }
}

struct FakeFetch {
    calls: Mutex<Vec<String>>,
    status: u16,
    headers: HeaderMap,
    body: Vec<u8>,
    failure: Option<String>,
}

impl Default for FakeFetch {
    fn default() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            status: 200,
            headers: HeaderMap::new(),
            body: Vec::new(),
            failure: None,
        }
    }
}

impl FakeFetch {
    fn reply(mut self, status: u16, headers: HeaderMap, body: &[u8]) -> Self {
        self.status = status;
        self.headers = headers;
        self.body = body.to_vec();
        self
    }

    fn failing(mut self, message: &str) -> Self {
        self.failure = Some(message.to_owned());
        self
    }
}

#[async_trait]
impl UrlFetch for FakeFetch {
    async fn fetch(&self, url: &str) -> Result<FetchedResponse, UrlFetchError> {
        self.calls.lock().unwrap().push(url.to_owned());
        if let Some(message) = &self.failure {
            return Err(UrlFetchError::Failed {
                message: message.clone(),
            });
        }
        Ok(FetchedResponse {
            status: self.status,
            headers: self.headers.clone(),
            body: self.body.clone(),
        })
    }
}

#[derive(Default)]
struct FakeCredentials {
    values: Option<CredentialValues>,
}

impl CredentialLookup for FakeCredentials {
    fn credential_values(
        &self,
        credential_name: &str,
    ) -> Result<CredentialValues, CredentialLookupError> {
        match &self.values {
            Some(values) => Ok(values.clone()),
            None => Err(CredentialLookupError::NotFound {
                credential_name: credential_name.to_owned(),
            }),
        }
    }
}

struct Harness {
    ocr: Arc<FakeOcr>,
    fetch: Arc<FakeFetch>,
    credentials: Arc<FakeCredentials>,
}

impl Default for Harness {
    fn default() -> Self {
        Self {
            ocr: Arc::new(FakeOcr::default()),
            fetch: Arc::new(FakeFetch::default()),
            credentials: Arc::new(FakeCredentials::default()),
        }
    }
}

impl Harness {
    fn with_ocr(mut self, ocr: FakeOcr) -> Self {
        self.ocr = Arc::new(ocr);
        self
    }

    fn with_fetch(mut self, fetch: FakeFetch) -> Self {
        self.fetch = Arc::new(fetch);
        self
    }

    fn with_credentials(mut self, values: &str) -> Self {
        self.credentials = Arc::new(FakeCredentials {
            values: Some(serde_json::from_str(values).expect("the credential values parse")),
        });
        self
    }

    fn state(&self, options: IngestionOptions) -> IngestionState {
        self.routed(options, None)
    }

    fn routed(
        &self,
        options: IngestionOptions,
        router: Option<Arc<dyn OcrOperation>>,
    ) -> IngestionState {
        IngestionState::new(
            options,
            router,
            IngestionPorts {
                ocr: self.ocr.clone(),
                uploader: self.fetch.clone(),
                credentials: self.credentials.clone(),
            },
        )
        .expect("the state builds")
    }
}

fn header(name: &'static str, value: &'static str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(name, HeaderValue::from_static(value));
    headers
}

fn text_of(config: &BTreeMap<String, Box<serde_json::value::RawValue>>, key: &str) -> String {
    let raw = config.get(key).expect("the key");
    serde_json::from_str(raw.get()).expect("a string")
}

#[test]
fn init_generates_a_prefixed_uuid_id() {
    let state = Harness::default().state(IngestionOptions::default());
    let suffix = state.ingest_id.strip_prefix("ingest_").expect("the prefix");
    assert_eq!(suffix.len(), 36);
    assert!(uuid::Uuid::parse_str(suffix).is_ok());
}

#[test]
fn init_stores_the_ingest_options() {
    let parsed = serde_json::from_str(
        r#"{
            "name": "docs",
            "ocr": {"model": "mistral/mistral-ocr-latest"},
            "embedding": {"model": "text-embedding-3-small"},
            "vector_store": {"custom_llm_provider": "openai", "vector_store_id": "vs_1"}
        }"#,
    )
    .expect("the options parse");
    let state = Harness::default().state(parsed);
    assert_eq!(state.ingest_name.as_deref(), Some("docs"));
    assert_eq!(
        state.ocr_config,
        Some(RAGIngestOCROptions {
            model: Some("mistral/mistral-ocr-latest".to_owned())
        })
    );
    assert_eq!(
        state
            .embedding_config
            .as_ref()
            .and_then(|config| config.model.as_deref()),
        Some("text-embedding-3-small")
    );
    assert_eq!(
        text_of(&state.vector_store_config, "custom_llm_provider"),
        "openai"
    );
    assert_eq!(
        text_of(&state.vector_store_config, "vector_store_id"),
        "vs_1"
    );
}

#[test]
fn init_defaults_the_chunking_strategy_to_auto() {
    let state = Harness::default().state(IngestionOptions::default());
    assert_eq!(state.chunking_strategy.get(), r#"{"type":"auto"}"#);
    assert!(state.vector_store_config.is_empty());
    assert!(state.ocr_config.is_none());
}

#[test]
fn init_defaults_an_empty_chunking_strategy() {
    let parsed = serde_json::from_str(r#"{"chunking_strategy": {}}"#).expect("the options parse");
    let state = Harness::default().state(parsed);
    assert_eq!(state.chunking_strategy.get(), r#"{"type":"auto"}"#);
}

#[test]
fn init_keeps_a_configured_chunking_strategy() {
    let parsed = serde_json::from_str(r#"{"chunking_strategy": {"chunk_size": 5}}"#)
        .expect("the options parse");
    let state = Harness::default().state(parsed);
    let plan = ChunkPlan::from_strategy(&state.chunking_strategy).expect("the plan");
    assert_eq!(plan.size, 5);
    assert_eq!(plan.overlap, 200);
}

#[test]
fn init_leaves_supports_existing_file_id_false() {
    assert!(!IngestionState::SUPPORTS_EXISTING_FILE_ID);
}

#[test]
fn credential_stored_values_win_over_caller_values() {
    let harness = Harness::default()
        .with_credentials(r#"{"custom_llm_provider": "bedrock", "vector_store_id": "vs_stored"}"#);
    let parsed = serde_json::from_str(
        r#"{
            "vector_store": {
                "custom_llm_provider": "openai",
                "vector_store_id": "vs_caller",
                "litellm_credential_name": "creds"
            }
        }"#,
    )
    .expect("the options parse");
    let state = harness.state(parsed);
    assert_eq!(
        text_of(&state.vector_store_config, "custom_llm_provider"),
        "bedrock"
    );
    assert_eq!(
        text_of(&state.vector_store_config, "vector_store_id"),
        "vs_stored"
    );
}

#[rstest]
#[case::api_base("api_base")]
#[case::aws_sts_endpoint("aws_sts_endpoint")]
#[case::aws_web_identity_token("aws_web_identity_token")]
#[test]
fn credential_keeps_only_the_endpoint_the_credential_defines(#[case] stored: &str) {
    let harness = Harness::default().with_credentials(&format!(r#"{{"{stored}": "kept"}}"#));
    let state = harness.state(
        serde_json::from_str(
            r#"{
                "vector_store": {
                    "api_base": "https://caller.example",
                    "aws_sts_endpoint": "https://sts.caller",
                    "aws_web_identity_token": "caller-token",
                    "ttl_days": 7,
                    "litellm_credential_name": "creds"
                }
            }"#,
        )
        .expect("the options parse"),
    );
    assert_eq!(
        state.vector_store_config.contains_key("api_base"),
        stored == "api_base"
    );
    assert_eq!(
        state.vector_store_config.contains_key("aws_sts_endpoint"),
        stored == "aws_sts_endpoint"
    );
    assert_eq!(
        state
            .vector_store_config
            .contains_key("aws_web_identity_token"),
        stored == "aws_web_identity_token"
    );
    assert_eq!(text_of(&state.vector_store_config, stored), "kept");
    assert!(state.vector_store_config.contains_key("ttl_days"));
}

#[test]
fn credential_removes_every_endpoint_field_a_credential_omits() {
    let harness = Harness::default().with_credentials(r#"{"api_key": "sk-stored"}"#);
    let parsed = serde_json::from_str(
        r#"{
            "vector_store": {
                "api_base": "https://caller.example",
                "aws_sts_endpoint": "https://sts.caller",
                "aws_web_identity_token": "caller-token",
                "litellm_credential_name": "creds"
            }
        }"#,
    )
    .expect("the options parse");
    let state = harness.state(parsed);
    assert!(state.vector_store_config.contains_key("api_key"));
    assert!(!state.vector_store_config.contains_key("api_base"));
    assert!(!state.vector_store_config.contains_key("aws_sts_endpoint"));
    assert!(
        !state
            .vector_store_config
            .contains_key("aws_web_identity_token")
    );
}

#[test]
fn credential_keeps_endpoint_fields_the_credential_defines() {
    let harness = Harness::default().with_credentials(
        r#"{
            "api_base": "https://stored.example",
            "aws_sts_endpoint": "https://sts.stored",
            "aws_web_identity_token": "stored-token"
        }"#,
    );
    let parsed = serde_json::from_str(
        r#"{
            "vector_store": {
                "api_base": "https://caller.example",
                "aws_sts_endpoint": "https://sts.caller",
                "aws_web_identity_token": "caller-token",
                "litellm_credential_name": "creds"
            }
        }"#,
    )
    .expect("the options parse");
    let state = harness.state(parsed);
    assert_eq!(
        text_of(&state.vector_store_config, "api_base"),
        "https://stored.example"
    );
    assert_eq!(
        text_of(&state.vector_store_config, "aws_sts_endpoint"),
        "https://sts.stored"
    );
    assert_eq!(
        text_of(&state.vector_store_config, "aws_web_identity_token"),
        "stored-token"
    );
}

#[test]
fn credential_absent_name_keeps_the_caller_config() {
    let harness = Harness::default().with_credentials(r#"{"api_key": "sk-stored"}"#);
    let parsed = serde_json::from_str(
        r#"{"vector_store": {"api_base": "https://caller.example", "vector_store_id": "vs_1"}}"#,
    )
    .expect("the options parse");
    let state = harness.state(parsed);
    assert!(!state.vector_store_config.contains_key("api_key"));
    assert!(state.vector_store_config.contains_key("api_base"));
    assert!(state.vector_store_config.contains_key("vector_store_id"));
}

#[test]
fn credential_null_name_keeps_the_caller_config() {
    let harness = Harness::default().with_credentials(r#"{"api_key": "sk-stored"}"#);
    let parsed = serde_json::from_str(
        r#"{"vector_store": {"api_base": "https://caller.example", "litellm_credential_name": null}}"#,
    )
    .expect("the options parse");
    let state = harness.state(parsed);
    assert!(!state.vector_store_config.contains_key("api_key"));
    assert!(state.vector_store_config.contains_key("api_base"));
}

#[test]
fn credential_unknown_name_keeps_the_caller_config() {
    let parsed = serde_json::from_str(
        r#"{"vector_store": {"api_base": "https://caller.example", "litellm_credential_name": "missing"}}"#,
    )
    .expect("the options parse");
    let state = Harness::default().state(parsed);
    assert!(state.vector_store_config.contains_key("api_base"));
}

#[test]
fn credential_empty_values_skip_the_merge_and_removal() {
    let harness = Harness::default().with_credentials("{}");
    let parsed = serde_json::from_str(
        r#"{
            "vector_store": {
                "api_base": "https://caller.example",
                "vector_store_id": "vs_1",
                "litellm_credential_name": "empty"
            }
        }"#,
    )
    .expect("the options parse");
    let state = harness.state(parsed);
    assert!(state.vector_store_config.contains_key("api_base"));
    assert!(state.vector_store_config.contains_key("vector_store_id"));
}

#[test]
fn credential_name_must_be_a_string() {
    let parsed = serde_json::from_str(r#"{"vector_store": {"litellm_credential_name": 5}}"#)
        .expect("the options parse");
    let error = IngestionState::new(
        parsed,
        None,
        IngestionPorts {
            ocr: Arc::new(FakeOcr::default()),
            uploader: Arc::new(FakeFetch::default()),
            credentials: Arc::new(FakeCredentials::default()),
        },
    )
    .err()
    .expect("the name is rejected");
    assert!(matches!(
        error,
        IngestionSetupError::InvalidField {
            field: "litellm_credential_name",
            expected: "a string"
        }
    ));
}

#[tokio::test]
async fn upload_precedence_prefers_file_data_over_url_and_id() {
    let harness = Harness::default();
    let state = harness.state(IngestionOptions::default());
    let uploaded = state
        .upload(
            Some(FileData {
                filename: "inline.txt".to_owned(),
                content: b"inline".to_vec(),
                content_type: "text/plain".to_owned(),
            }),
            Some("https://host/remote.txt"),
            Some("file_1"),
        )
        .await
        .expect("the upload");
    assert_eq!(uploaded.filename.as_deref(), Some("inline.txt"));
    assert_eq!(uploaded.file_content.as_deref(), Some(b"inline".as_slice()));
    assert_eq!(uploaded.content_type.as_deref(), Some("text/plain"));
    assert!(uploaded.existing_file_id.is_none());
    assert!(harness.fetch.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn upload_precedence_uses_the_url_without_file_data() {
    let harness = Harness::default().with_fetch(FakeFetch::default().reply(
        200,
        header("content-type", "text/plain"),
        b"remote",
    ));
    let state = harness.state(IngestionOptions::default());
    let uploaded = state
        .upload(None, Some("https://host/folder/remote.txt"), Some("file_1"))
        .await
        .expect("the upload");
    assert_eq!(uploaded.filename.as_deref(), Some("remote.txt"));
    assert_eq!(uploaded.file_content.as_deref(), Some(b"remote".as_slice()));
    assert!(uploaded.existing_file_id.is_none());
    let calls = harness.fetch.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0], "https://host/folder/remote.txt");
}

#[tokio::test]
async fn upload_precedence_falls_back_to_the_file_id() {
    let harness = Harness::default();
    let state = harness.state(IngestionOptions::default());
    let uploaded = state
        .upload(None, None, Some("file_123"))
        .await
        .expect("the upload");
    assert_eq!(uploaded.existing_file_id.as_deref(), Some("file_123"));
    assert!(uploaded.filename.is_none());
    assert!(uploaded.file_content.is_none());
    assert!(uploaded.content_type.is_none());
    assert!(harness.fetch.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn upload_precedence_rejects_an_absent_source() {
    let state = Harness::default().state(IngestionOptions::default());
    let error = state
        .upload(None, None, None)
        .await
        .err()
        .expect("the source is required");
    assert!(matches!(error, UploadError::MissingSource));
}

#[rstest]
#[case::file_in_directory("https://host/dir/report.pdf", "report.pdf")]
#[case::trailing_slash("https://host/dir/", "document")]
#[case::bare_name("report.pdf", "report.pdf")]
#[tokio::test]
async fn upload_fetch_derives_the_filename_from_the_url_path(
    #[case] url: &str,
    #[case] expected: &str,
) {
    let harness = Harness::default();
    let state = harness.state(IngestionOptions::default());
    let uploaded = state
        .upload(None, Some(url), None)
        .await
        .expect("the upload");
    assert_eq!(uploaded.filename.as_deref(), Some(expected));
}

#[test]
fn upload_fetch_takes_the_content_type_from_the_response_headers() {
    assert_eq!(
        content_type_from_headers(&header("content-type", "application/pdf; charset=binary")),
        "application/pdf; charset=binary"
    );
    assert_eq!(content_type_from_headers(&header("content-type", "")), "");
    assert_eq!(
        content_type_from_headers(&HeaderMap::new()),
        DEFAULT_CONTENT_TYPE
    );
}

#[test]
fn upload_fetch_defaults_the_filename_of_an_empty_url() {
    assert_eq!(filename_from_url(""), "document");
}

#[tokio::test]
async fn upload_fetch_reports_a_failed_status() {
    let harness = Harness::default().with_fetch(FakeFetch::default().reply(
        404,
        HeaderMap::new(),
        b"missing",
    ));
    let state = harness.state(IngestionOptions::default());
    let error = state
        .upload(None, Some("https://host/missing.txt"), None)
        .await
        .err()
        .expect("the status fails the upload");
    assert!(matches!(error, UploadError::HttpStatus { status: 404 }));
}

#[tokio::test]
async fn upload_fetch_accepts_a_status_below_400() {
    let harness = Harness::default().with_fetch(FakeFetch::default().reply(
        399,
        HeaderMap::new(),
        b"redirected",
    ));
    let state = harness.state(IngestionOptions::default());
    let uploaded = state
        .upload(None, Some("https://host/file.txt"), None)
        .await
        .expect("the upload succeeds");
    assert_eq!(
        uploaded.file_content.as_deref(),
        Some(b"redirected".as_slice())
    );
}

#[tokio::test]
async fn upload_fetch_propagates_a_port_error() {
    let harness =
        Harness::default().with_fetch(FakeFetch::default().failing("the host is blocked"));
    let state = harness.state(IngestionOptions::default());
    let error = state
        .upload(None, Some("https://host/file.txt"), None)
        .await
        .err()
        .expect("the fetch fails");
    assert!(matches!(error, UploadError::Fetch(_)));
}

fn http_fetch(allowed_hosts: &[String]) -> HttpUrlFetch<SystemResolver> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("the policy"),
    );
    let client = ProxiedClient::build(
        base_builder().http1_only(),
        &policy,
        TransportLimits::DEFAULT,
    )
    .expect("the client");
    HttpUrlFetch {
        client,
        urls: UserUrlFetcher {
            validator: UrlValidator::new(SystemResolver, allowed_hosts, &Verification::Enabled),
            validation: true,
        },
    }
}

#[tokio::test]
async fn upload_fetch_reads_the_body_and_headers_through_the_checked_client() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let url = format!("http://{address}/files/report.txt");
    let fetch = http_fetch(&["127.0.0.1".to_owned()]);
    let serving = async {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = [0_u8; 1024];
        let _read = socket.read(&mut buffer).await.unwrap();
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
            )
            .await
            .unwrap();
    };
    let (result, ()) = tokio::join!(fetch.fetch(&url), serving);
    let response = result.expect("the fetch succeeds");
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"hello");
    assert_eq!(content_type_from_headers(&response.headers), "text/plain");
}

#[tokio::test]
async fn upload_fetch_refuses_a_blocked_address() {
    let fetch = http_fetch(&[]);
    let error = fetch
        .fetch("http://127.0.0.1:9/file.txt")
        .await
        .err()
        .expect("the address is blocked");
    assert!(matches!(error, UrlFetchError::Request(_)));
}

#[tokio::test]
async fn ocr_without_a_config_returns_none() {
    let harness = Harness::default();
    let state = harness.state(IngestionOptions::default());
    let text = state
        .ocr(Some(b"content"), Some("text/plain"))
        .await
        .expect("ocr");
    assert!(text.is_none());
    assert!(harness.ocr.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn ocr_without_content_returns_none() {
    let harness = Harness::default();
    let parsed = serde_json::from_str(r#"{"ocr": {"model": "mistral/mistral-ocr-latest"}}"#)
        .expect("the options parse");
    let state = harness.state(parsed);
    assert!(
        state
            .ocr(None, Some("text/plain"))
            .await
            .expect("ocr")
            .is_none()
    );
    assert!(
        state
            .ocr(Some(b""), Some("text/plain"))
            .await
            .expect("ocr")
            .is_none()
    );
    assert!(harness.ocr.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn ocr_defaults_the_model_and_encodes_a_document_data_url() {
    let harness = Harness::default().with_ocr(FakeOcr::returning(vec!["page".to_owned()]));
    let state = harness.state(serde_json::from_str(r#"{"ocr": {}}"#).expect("the options parse"));
    let text = state
        .ocr(Some(b"hello"), Some("application/pdf"))
        .await
        .expect("ocr");
    assert_eq!(text.as_deref(), Some("page"));
    let calls = harness.ocr.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "mistral/mistral-ocr-latest");
    assert_eq!(calls[0].document.kind, OcrDocumentKind::DocumentUrl);
    assert_eq!(
        calls[0].document.data_url,
        "data:application/pdf;base64,aGVsbG8="
    );
}

#[tokio::test]
async fn ocr_uses_the_configured_model() {
    let harness = Harness::default().with_ocr(FakeOcr::returning(vec!["page".to_owned()]));
    let state = harness.state(
        serde_json::from_str(r#"{"ocr": {"model": "mistral/mistral-ocr-2505"}}"#)
            .expect("the options parse"),
    );
    state
        .ocr(Some(b"hello"), Some("text/plain"))
        .await
        .expect("ocr");
    assert_eq!(
        harness.ocr.calls.lock().unwrap()[0].model,
        "mistral/mistral-ocr-2505"
    );
}

#[tokio::test]
async fn ocr_image_content_types_use_the_image_url_key() {
    let harness = Harness::default().with_ocr(FakeOcr::returning(vec!["page".to_owned()]));
    let state = harness.state(serde_json::from_str(r#"{"ocr": {}}"#).expect("the options parse"));
    state
        .ocr(Some(b"img"), Some("image/png"))
        .await
        .expect("ocr");
    let calls = harness.ocr.calls.lock().unwrap();
    assert_eq!(calls[0].document.kind, OcrDocumentKind::ImageUrl);
    assert_eq!(calls[0].document.data_url, "data:image/png;base64,aW1n");
}

#[tokio::test]
async fn ocr_without_a_content_type_uses_an_empty_mime() {
    let harness = Harness::default().with_ocr(FakeOcr::returning(vec!["page".to_owned()]));
    let state = harness.state(serde_json::from_str(r#"{"ocr": {}}"#).expect("the options parse"));
    state.ocr(Some(b"x"), None).await.expect("ocr");
    assert_eq!(
        harness.ocr.calls.lock().unwrap()[0].document.data_url,
        "data:;base64,eA=="
    );
}

#[tokio::test]
async fn ocr_prefers_the_router_operation() {
    let harness = Harness::default();
    let routed = Arc::new(FakeOcr::returning(vec!["router".to_owned()]));
    let state = harness.routed(
        serde_json::from_str(r#"{"ocr": {}}"#).expect("the options parse"),
        Some(routed.clone()),
    );
    let text = state
        .ocr(Some(b"x"), Some("text/plain"))
        .await
        .expect("ocr");
    assert_eq!(text.as_deref(), Some("router"));
    assert_eq!(routed.calls.lock().unwrap().len(), 1);
    assert!(harness.ocr.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn ocr_joins_pages_with_a_blank_line() {
    let harness = Harness::default().with_ocr(FakeOcr::returning(vec![
        "first".to_owned(),
        "second".to_owned(),
    ]));
    let state = harness.state(serde_json::from_str(r#"{"ocr": {}}"#).expect("the options parse"));
    let text = state
        .ocr(Some(b"x"), Some("text/plain"))
        .await
        .expect("ocr");
    assert_eq!(text.as_deref(), Some("first\n\nsecond"));
}

#[tokio::test]
async fn ocr_without_pages_returns_none() {
    let harness = Harness::default().with_ocr(FakeOcr::returning(Vec::new()));
    let state = harness.state(serde_json::from_str(r#"{"ocr": {}}"#).expect("the options parse"));
    let text = state
        .ocr(Some(b"x"), Some("text/plain"))
        .await
        .expect("ocr");
    assert!(text.is_none());
}

#[tokio::test]
async fn ocr_propagates_the_operation_error() {
    let harness = Harness::default().with_ocr(FakeOcr::failing("the provider failed"));
    let state = harness.state(serde_json::from_str(r#"{"ocr": {}}"#).expect("the options parse"));
    let error = state
        .ocr(Some(b"x"), Some("text/plain"))
        .await
        .err()
        .expect("the OCR fails");
    assert_eq!(
        error,
        OcrError::Failed {
            message: "the provider failed".to_owned()
        }
    );
}

#[test]
fn chunk_splits_the_given_text() {
    let state = Harness::default().state(IngestionOptions::default());
    let chunks = state
        .chunk(Some("hello world"), None, true)
        .expect("chunks");
    assert_eq!(chunks, ["hello world"]);
}

#[test]
fn chunk_prefers_the_text_over_the_file_content() {
    let state = Harness::default().state(IngestionOptions::default());
    let chunks = state
        .chunk(Some("from ocr"), Some(b"from file"), true)
        .expect("chunks");
    assert_eq!(chunks, ["from ocr"]);
}

#[test]
fn chunk_decodes_utf8_file_content() {
    let state = Harness::default().state(IngestionOptions::default());
    let chunks = state
        .chunk(None, Some(b"from file"), false)
        .expect("chunks");
    assert_eq!(chunks, ["from file"]);
}

#[test]
fn chunk_ignores_file_content_when_ocr_was_used() {
    let state = Harness::default().state(IngestionOptions::default());
    let chunks = state.chunk(None, Some(b"from file"), true).expect("chunks");
    assert!(chunks.is_empty());
}

#[test]
fn chunk_extracts_text_from_a_binary_pdf() {
    let state = Harness::default().state(IngestionOptions::default());
    let pdf = sample_pdf();
    let chunks = state
        .chunk(None, Some(pdf.as_slice()), false)
        .expect("chunks");
    assert_eq!(chunks, ["Hello PDF"]);
}

#[test]
fn chunk_skips_binary_content_that_is_not_a_pdf() {
    let state = Harness::default().state(IngestionOptions::default());
    let chunks = state
        .chunk(None, Some(&[0xff, 0xfe, 0x00]), false)
        .expect("chunks");
    assert!(chunks.is_empty());
}

#[test]
fn chunk_skips_a_pdf_without_extractable_text() {
    let state = Harness::default().state(IngestionOptions::default());
    let chunks = state
        .chunk(None, Some(b"%PDF-\xff"), false)
        .expect("chunks");
    assert!(chunks.is_empty());
}

#[test]
fn chunk_skips_a_pdf_behind_a_binary_preamble() {
    let state = Harness::default().state(IngestionOptions::default());
    let mut content = vec![0xff, 0xfe];
    content.extend_from_slice(&sample_pdf());
    let chunks = state.chunk(None, Some(&content), false).expect("chunks");
    assert!(chunks.is_empty());
}

#[test]
fn chunk_with_no_source_returns_no_chunks() {
    let state = Harness::default().state(IngestionOptions::default());
    assert!(state.chunk(None, None, false).expect("chunks").is_empty());
    assert!(
        state
            .chunk(None, Some(b""), false)
            .expect("chunks")
            .is_empty()
    );
    assert!(
        state
            .chunk(Some(""), Some(b"from file"), false)
            .expect("chunks")
            .is_empty()
    );
}

#[test]
fn chunk_uses_the_configured_size_and_separators() {
    let state = Harness::default().state(
        serde_json::from_str(
            r#"{"chunking_strategy": {"chunk_size": 5, "chunk_overlap": 0, "separators": ["\n\n", ""]}}"#,
        )
        .expect("the options parse"),
    );
    let chunks = state
        .chunk(Some("one\n\ntwo\n\nthree"), None, false)
        .expect("chunks");
    assert_eq!(chunks, ["one", "two", "three"]);
}

#[test]
fn chunk_defaults_the_size_and_overlap() {
    let state = Harness::default().state(IngestionOptions::default());
    let chunks = state.chunk(Some("abcdefgh"), None, false).expect("chunks");
    assert_eq!(chunks, ["abcdefgh"]);
}

#[rstest]
#[case::zero(0)]
#[case::negative(-5)]
#[test]
fn chunk_config_rejects_a_non_positive_size(#[case] size: i64) {
    let parsed = serde_json::from_str(&format!(
        r#"{{"chunking_strategy": {{"chunk_size": {size}}}}}"#
    ))
    .expect("the options parse");
    let state = Harness::default().state(parsed);
    let error = state
        .chunk(Some("text"), None, false)
        .err()
        .expect("the size is rejected");
    assert!(matches!(error, ChunkError::Configuration(_)));
}

#[test]
fn chunk_config_rejects_a_negative_overlap() {
    let parsed =
        serde_json::from_str(r#"{"chunking_strategy": {"chunk_size": 10, "chunk_overlap": -1}}"#)
            .expect("the options parse");
    let state = Harness::default().state(parsed);
    let error = state
        .chunk(Some("text"), None, false)
        .err()
        .expect("the overlap is rejected");
    assert!(matches!(error, ChunkError::Configuration(_)));
}

#[rstest]
#[case::equal(10)]
#[case::greater(11)]
#[test]
fn chunk_config_rejects_an_overlap_at_or_above_the_size(#[case] overlap: i64) {
    let parsed = serde_json::from_str(&format!(
        r#"{{"chunking_strategy": {{"chunk_size": 10, "chunk_overlap": {overlap}}}}}"#
    ))
    .expect("the options parse");
    let state = Harness::default().state(parsed);
    let error = state
        .chunk(Some("text"), None, false)
        .err()
        .expect("the overlap is rejected");
    assert!(matches!(error, ChunkError::Configuration(_)));
}

#[test]
fn chunk_config_accepts_an_overlap_just_below_the_size() {
    let parsed =
        serde_json::from_str(r#"{"chunking_strategy": {"chunk_size": 10, "chunk_overlap": 9}}"#)
            .expect("the options parse");
    let state = Harness::default().state(parsed);
    let chunks = state.chunk(Some("short"), None, false).expect("chunks");
    assert_eq!(chunks, ["short"]);
}

#[test]
fn chunk_config_null_values_take_the_defaults() {
    let parsed = serde_json::from_str(
        r#"{"chunking_strategy": {"chunk_size": null, "chunk_overlap": null, "separators": null}}"#,
    )
    .expect("the options parse");
    let state = Harness::default().state(parsed);
    let plan = ChunkPlan::from_strategy(&state.chunking_strategy).expect("the plan");
    assert_eq!(plan.size, 1000);
    assert_eq!(plan.overlap, 200);
    assert_eq!(plan.separators, ["\n\n", "\n", " ", ""]);
}

#[rstest]
#[case::size_text(r#"{"chunk_size": "5"}"#)]
#[case::overlap_text(r#"{"chunk_overlap": "5"}"#)]
#[case::separators_text(r#"{"separators": "x"}"#)]
#[test]
fn chunk_config_wrong_types_are_rejected(#[case] strategy: &str) {
    let parsed = serde_json::from_str(&format!(r#"{{"chunking_strategy": {strategy}}}"#))
        .expect("the options parse");
    let state = Harness::default().state(parsed);
    let error = state
        .chunk(Some("text"), None, false)
        .err()
        .expect("the types are rejected");
    assert!(matches!(error, ChunkError::Strategy(_)));
}

#[test]
fn chunk_config_is_not_validated_without_text() {
    let parsed = serde_json::from_str(r#"{"chunking_strategy": {"chunk_size": 0}}"#)
        .expect("the options parse");
    let state = Harness::default().state(parsed);
    assert!(state.chunk(None, None, false).expect("chunks").is_empty());
}

#[derive(Debug, Error)]
enum StoreFailure {
    #[error("the store refused")]
    Refused,
}

struct FakeStore {
    chunks: Mutex<usize>,
}

#[async_trait]
impl RagStore for FakeStore {
    type Error = StoreFailure;

    async fn store(&self, request: StoreRequest<'_>) -> Result<StoreResult, Self::Error> {
        *self.chunks.lock().unwrap() = request.chunks.len();
        if request.existing_file_id == Some("fail") {
            return Err(StoreFailure::Refused);
        }
        Ok((
            request.filename.map(str::to_owned),
            request.existing_file_id.map(str::to_owned),
        ))
    }
}

#[tokio::test]
async fn store_contract_returns_the_vector_store_and_file_ids() {
    let store = FakeStore {
        chunks: Mutex::new(0),
    };
    let chunks = vec!["one".to_owned(), "two".to_owned()];
    let result = store
        .store(StoreRequest {
            file_content: Some(b"content"),
            filename: Some("doc.txt"),
            content_type: Some("text/plain"),
            chunks: &chunks,
            embeddings: None,
            existing_file_id: Some("file_1"),
        })
        .await
        .expect("the store succeeds");
    assert_eq!(
        result,
        (Some("doc.txt".to_owned()), Some("file_1".to_owned()))
    );
    assert_eq!(*store.chunks.lock().unwrap(), 2);
}

#[tokio::test]
async fn store_contract_propagates_the_provider_error() {
    let store = FakeStore {
        chunks: Mutex::new(0),
    };
    let chunks = Vec::new();
    let error = store
        .store(StoreRequest {
            file_content: None,
            filename: None,
            content_type: None,
            chunks: &chunks,
            embeddings: None,
            existing_file_id: Some("fail"),
        })
        .await
        .err()
        .expect("the store fails");
    assert!(matches!(error, StoreFailure::Refused));
}

fn sample_pdf() -> Vec<u8> {
    use base64::Engine as _;
    let encoded = "JVBERi0xLjQKJenpCjEgMCBvYmoKPDwgL1R5cGUgL0NhdGFsb2cgL1BhZ2VzIDIgMCBSID4+CmVuZG9iagoyIDAgb2JqCjw8IC9UeXBlIC9QYWdlcyAvS2lkcyBbMyAwIFJdIC9Db3VudCAxID4+CmVuZG9iagozIDAgb2JqCjw8IC9UeXBlIC9QYWdlIC9QYXJlbnQgMiAwIFIgL1Jlc291cmNlcyA8PCAvRm9udCA8PCAvRjEgNCAwIFIgPj4gPj4gL01lZGlhQm94IFswIDAgNjEyIDc5Ml0gL0NvbnRlbnRzIDUgMCBSID4+CmVuZG9iago0IDAgb2JqCjw8IC9UeXBlIC9Gb250IC9TdWJ0eXBlIC9UeXBlMSAvQmFzZUZvbnQgL0hlbHZldGljYSA+PgplbmRvYmoKNSAwIG9iago8PCAvTGVuZ3RoIDQyID4+CnN0cmVhbQpCVCAvRjEgMTIgVGYgMTAwIDcwMCBUZCAoSGVsbG8gUERGKSBUaiBFVAplbmRzdHJlYW0KZW5kb2JqCnhyZWYKMCA2CjAwMDAwMDAwMDAgNjU1MzUgZiAKMDAwMDAwMDAxMyAwMDAwMCBuIAowMDAwMDAwMDYyIDAwMDAwIG4gCjAwMDAwMDAxMTkgMDAwMDAgbiAKMDAwMDAwMDI0NSAwMDAwMCBuIAowMDAwMDAwMzE1IDAwMDAwIG4gCnRyYWlsZXIKPDwgL1NpemUgNiAvUm9vdCAxIDAgUiA+PgpzdGFydHhyZWYKNDA2CiUlRU9GCg==";
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .expect("the fixture decodes")
}
