#![cfg(test)]

use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::credentials::CredentialValues;
use betterlite_config::rag::document::ingest::VertexAIVectorStoreOptions;
use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_models::decisions::Extra;
use secrecy::SecretString;
use serde_json::value::RawValue;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::rag::ingestion::IngestionPorts;
use crate::rag::ingestion::IngestionState;
use crate::rag::ingestion::config::{ChunkingStrategyMap, IngestionOptions};
use crate::rag::ingestion::credentials::{CredentialLookup, CredentialLookupError};
use crate::rag::ingestion::ocr::{OcrError, OcrOperation, OcrRequest, OcrResponse};
use crate::rag::ingestion::store::{StoreRequest, StoreResult};
use crate::rag::ingestion::upload::{FetchedResponse, UrlFetch, UrlFetchError};

use super::VertexIngestion;
use super::error::VertexIngestionError;
use super::ports::{
    GetRequest, HttpReply, JsonRequest, MultipartRequest, PortError, VertexIngestionPorts,
    VertexRagHttp, VertexTokenPort,
};

const TOKEN: &str = "tok-123";
const PROJECT_OPTIONS: &str = r#"{"vertex_project":"proj"}"#;
const EXISTING_OPTIONS: &str = r#"{"vertex_project":"proj","vector_store_id":"corpora/7"}"#;
const NOT_DONE: &str = r#"{"done":false,"name":"operations/1"}"#;
const DONE: &str = r#"{"done":true,"response":{"name":"corpora/1"}}"#;
const FILE_REPLY: &str = r#"{"ragFile":{"name":"files/1"}}"#;
const CORPUS_URL: &str = "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/ragCorpora";
const UPLOAD_URL: &str =
    "https://us-central1-aiplatform.googleapis.com/upload/v1beta1/corpora/7/ragFiles:upload";

struct Unused;

#[async_trait]
impl OcrOperation for Unused {
    async fn aocr(&self, _request: OcrRequest) -> Result<OcrResponse, OcrError> {
        Err(OcrError::Failed {
            message: "unused".to_owned(),
        })
    }
}

#[async_trait]
impl UrlFetch for Unused {
    async fn fetch(&self, _url: &str) -> Result<FetchedResponse, UrlFetchError> {
        Err(UrlFetchError::Failed {
            message: "unused".to_owned(),
        })
    }
}

impl CredentialLookup for Unused {
    fn credential_values(
        &self,
        credential_name: &str,
    ) -> Result<CredentialValues, CredentialLookupError> {
        Err(CredentialLookupError::NotFound {
            credential_name: credential_name.to_owned(),
        })
    }
}

#[derive(Default)]
struct FakeToken {
    calls: Mutex<Vec<(bool, Option<String>)>>,
}

impl FakeToken {
    fn credentials_seen(&self) -> Vec<bool> {
        self.records()
            .into_iter()
            .map(|(credentials, _)| credentials)
            .collect()
    }

    fn projects_seen(&self) -> Vec<Option<String>> {
        self.records()
            .into_iter()
            .map(|(_, project)| project)
            .collect()
    }

    fn records(&self) -> Vec<(bool, Option<String>)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl VertexTokenPort for FakeToken {
    async fn access_token(
        &self,
        credentials: Option<&ConfiguredCredentials>,
        project: Option<&str>,
    ) -> Result<SecretString, PortError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((credentials.is_some(), project.map(str::to_owned)));
        Ok(SecretString::from(TOKEN.to_owned()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Json,
    Get,
    Multipart,
}

#[derive(Debug, Clone)]
struct Call {
    kind: Kind,
    url: String,
    body: String,
    filename: String,
    file: Vec<u8>,
    content_type: String,
    timeout: Duration,
}

struct ScriptedHttp {
    replies: Mutex<VecDeque<(u16, &'static str)>>,
    calls: Mutex<Vec<Call>>,
}

impl ScriptedHttp {
    fn new(replies: &[(u16, &'static str)]) -> Self {
        Self {
            replies: Mutex::new(replies.iter().copied().collect()),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn reply(&self) -> HttpReply {
        let mut replies = self.replies.lock().unwrap_or_else(PoisonError::into_inner);
        let (status, body) = replies.pop_front().expect("a scripted reply");
        HttpReply {
            status,
            body: body.as_bytes().to_vec(),
        }
    }

    fn record(&self, call: Call) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }

    fn calls(&self) -> Vec<Call> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn text(bytes: &[u8]) -> String {
    std::str::from_utf8(bytes)
        .expect("a UTF-8 request part")
        .to_owned()
}

#[async_trait]
impl VertexRagHttp for ScriptedHttp {
    async fn post_json(&self, request: JsonRequest<'_>) -> Result<HttpReply, PortError> {
        self.record(Call {
            kind: Kind::Json,
            url: request.url.to_owned(),
            body: text(request.body),
            filename: String::new(),
            file: Vec::new(),
            content_type: String::new(),
            timeout: request.timeout,
        });
        Ok(self.reply())
    }

    async fn get(&self, request: GetRequest<'_>) -> Result<HttpReply, PortError> {
        self.record(Call {
            kind: Kind::Get,
            url: request.url.to_owned(),
            body: String::new(),
            filename: String::new(),
            file: Vec::new(),
            content_type: String::new(),
            timeout: request.timeout,
        });
        Ok(self.reply())
    }

    async fn post_multipart(&self, request: MultipartRequest<'_>) -> Result<HttpReply, PortError> {
        self.record(Call {
            kind: Kind::Multipart,
            url: request.url.to_owned(),
            body: text(request.metadata),
            filename: request.filename.to_owned(),
            file: request.file.to_vec(),
            content_type: request.file_content_type.to_owned(),
            timeout: request.timeout,
        });
        Ok(self.reply())
    }
}

struct Harness {
    ingestion: VertexIngestion,
    token: Arc<FakeToken>,
    http: Arc<ScriptedHttp>,
}

fn raw(source: &str) -> Box<RawValue> {
    serde_json::from_str(source).expect("a raw JSON value")
}

fn chunking(entries: &[(&str, &str)]) -> ChunkingStrategyMap {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), raw(value)))
        .collect()
}

fn state(name: Option<&str>, strategy: Option<ChunkingStrategyMap>) -> IngestionState {
    IngestionState::new(
        IngestionOptions {
            name: name.map(str::to_owned),
            chunking_strategy: strategy,
            ..IngestionOptions::default()
        },
        None,
        IngestionPorts {
            ocr: Arc::new(Unused),
            uploader: Arc::new(Unused),
            credentials: Arc::new(Unused),
        },
    )
    .expect("an ingestion state")
}

fn build(
    options: &str,
    settings: &VertexSettings,
    name: Option<&str>,
    strategy: Option<ChunkingStrategyMap>,
    replies: &[(u16, &'static str)],
) -> Harness {
    let http = Arc::new(ScriptedHttp::new(replies));
    let token = Arc::new(FakeToken::default());
    let ports = VertexIngestionPorts {
        token: Arc::<FakeToken>::clone(&token),
        http: Arc::<ScriptedHttp>::clone(&http),
    };
    let options: VertexAIVectorStoreOptions =
        serde_json::from_str(options).expect("vector store options");
    Harness {
        ingestion: VertexIngestion::new(state(name, strategy), options, settings, ports),
        token,
        http,
    }
}

#[derive(Default, Clone, Copy)]
struct File<'call> {
    content: Option<&'call [u8]>,
    filename: Option<&'call str>,
    content_type: Option<&'call str>,
    existing_file_id: Option<&'call str>,
}

async fn store(harness: &Harness, file: File<'_>) -> Result<StoreResult, VertexIngestionError> {
    harness
        .ingestion
        .store(StoreRequest {
            file_content: file.content,
            filename: file.filename,
            content_type: file.content_type,
            chunks: &[],
            embeddings: None,
            existing_file_id: file.existing_file_id,
        })
        .await
}

fn json_body(call: &Call) -> Extra {
    serde_json::from_str(&call.body).expect("a JSON request body")
}

fn expected(source: &str) -> Extra {
    serde_json::from_str(source).expect("an expected JSON body")
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        let bytes = self.0.lock().expect("the capture lock").clone();
        String::from_utf8(bytes).expect("the captured log is UTF-8")
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}

#[test]
fn flags_supports_existing_file_id_matches_the_shared_state() {
    assert!(!VertexIngestion::SUPPORTS_EXISTING_FILE_ID);
    assert_eq!(
        VertexIngestion::SUPPORTS_EXISTING_FILE_ID,
        IngestionState::SUPPORTS_EXISTING_FILE_ID
    );
}

#[tokio::test]
async fn flags_embed_returns_no_embeddings() {
    let harness = build(PROJECT_OPTIONS, &VertexSettings::default(), None, None, &[]);
    let chunks = ["one chunk".to_owned()];
    assert_eq!(harness.ingestion.embed(&chunks).await, None);
}

#[tokio::test]
async fn flags_existing_file_id_is_ignored() {
    let harness = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, FILE_REPLY)],
    );
    let file = File {
        content: Some(b"data".as_slice()),
        filename: Some("n.txt"),
        existing_file_id: Some("legacy"),
        ..File::default()
    };
    let (_, file_id) = store(&harness, file).await.expect("a stored file");
    assert_eq!(file_id.as_deref(), Some("files/1"));
    let idle = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[],
    );
    let file = File {
        existing_file_id: Some("legacy"),
        ..File::default()
    };
    let (_, file_id) = store(&idle, file).await.expect("a skipped upload");
    assert_eq!(file_id, None);
}

#[tokio::test]
async fn project_missing_project_fails_without_any_port_call() {
    let harness = build("{}", &VertexSettings::default(), None, None, &[]);
    let error = store(&harness, File::default())
        .await
        .expect_err("a missing project");
    assert_eq!(
        error.to_string(),
        "vertex_project is required for Vertex AI RAG ingestion. Set it in vector_store config."
    );
    assert!(harness.token.credentials_seen().is_empty());
    assert!(harness.http.calls().is_empty());
}

#[tokio::test]
async fn project_empty_project_option_is_missing() {
    let settings = VertexSettings {
        project: Some("settings-proj".to_owned()),
        ..VertexSettings::default()
    };
    let harness = build(r#"{"vertex_project":""}"#, &settings, None, None, &[]);
    let error = store(&harness, File::default())
        .await
        .expect_err("an empty project");
    assert_eq!(
        error.to_string(),
        "vertex_project is required for Vertex AI RAG ingestion. Set it in vector_store config."
    );
    assert!(harness.token.credentials_seen().is_empty());
}

#[tokio::test]
async fn project_settings_project_is_used_when_the_options_omit_one() {
    let settings = VertexSettings {
        project: Some("settings-proj".to_owned()),
        ..VertexSettings::default()
    };
    let harness = build("{}", &settings, None, None, &[(200, DONE)]);
    store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(
        harness.http.calls()[0].url,
        "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/settings-proj/locations/us-central1/ragCorpora"
    );
}

#[tokio::test]
async fn project_vertex_project_wins_over_the_vertex_ai_alias() {
    let settings = VertexSettings {
        project: Some("settings-proj".to_owned()),
        ..VertexSettings::default()
    };
    let options = r#"{"vertex_project":"primary","vertex_ai_project":"alias"}"#;
    let harness = build(options, &settings, None, None, &[(200, DONE)]);
    store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert!(
        harness.http.calls()[0]
            .url
            .contains("/projects/primary/locations/")
    );
}

#[tokio::test]
async fn project_location_defaults_to_us_central1() {
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, DONE)],
    );
    store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(harness.http.calls()[0].url, CORPUS_URL);
}

#[tokio::test]
async fn project_location_uses_the_settings_value() {
    let settings = VertexSettings {
        location: Some("europe-west4".to_owned()),
        ..VertexSettings::default()
    };
    let harness = build(PROJECT_OPTIONS, &settings, None, None, &[(200, DONE)]);
    store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(
        harness.http.calls()[0].url,
        "https://europe-west4-aiplatform.googleapis.com/v1beta1/projects/proj/locations/europe-west4/ragCorpora"
    );
}

#[tokio::test]
async fn project_location_aliases_fall_through_empties() {
    let options =
        r#"{"vertex_project":"proj","vertex_location":"","vertex_ai_location":"asia-east1"}"#;
    let harness = build(
        options,
        &VertexSettings::default(),
        None,
        None,
        &[(200, DONE)],
    );
    store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(
        harness.http.calls()[0].url,
        "https://asia-east1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/asia-east1/ragCorpora"
    );
}

#[tokio::test]
async fn project_location_uses_the_global_host() {
    let options = r#"{"vertex_project":"proj","vertex_location":"global"}"#;
    let harness = build(
        options,
        &VertexSettings::default(),
        None,
        None,
        &[(200, DONE)],
    );
    store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(
        harness.http.calls()[0].url,
        "https://aiplatform.googleapis.com/v1beta1/projects/proj/locations/global/ragCorpora"
    );
}

#[tokio::test]
async fn project_location_uses_the_multiregion_host() {
    let options = r#"{"vertex_project":"proj","vertex_location":"eu"}"#;
    let harness = build(
        options,
        &VertexSettings::default(),
        None,
        None,
        &[(200, DONE)],
    );
    store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(
        harness.http.calls()[0].url,
        "https://aiplatform.eu.rep.googleapis.com/v1beta1/projects/proj/locations/eu/ragCorpora"
    );
}

#[tokio::test]
async fn project_invalid_location_fails_after_the_token_call() {
    let options = r#"{"vertex_project":"proj","vertex_location":"Bad_Location"}"#;
    let harness = build(options, &VertexSettings::default(), None, None, &[]);
    let error = store(&harness, File::default())
        .await
        .expect_err("an invalid location");
    assert_eq!(error.to_string(), "invalid vertex_location format");
    assert_eq!(harness.token.credentials_seen().len(), 1);
    assert!(harness.http.calls().is_empty());
}

#[tokio::test]
async fn corpus_create_sends_the_camel_case_body() {
    let options =
        r#"{"vertex_project":"proj","description":"notes","vector_db_config":{"index":"x"}}"#;
    let harness = build(
        options,
        &VertexSettings::default(),
        Some("ingest-name"),
        None,
        &[(200, DONE)],
    );
    let (corpus, file_id) = store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(corpus.as_deref(), Some("corpora/1"));
    assert_eq!(file_id, None);
    let calls = harness.http.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].kind, Kind::Json);
    assert_eq!(calls[0].url, CORPUS_URL);
    assert_eq!(calls[0].timeout, Duration::from_secs(60));
    assert_eq!(
        json_body(&calls[0]),
        expected(
            r#"{"displayName":"ingest-name","description":"notes","vectorDbConfig":{"index":"x"}}"#
        )
    );
    assert_eq!(harness.token.credentials_seen().len(), 1);
}

#[tokio::test]
async fn corpus_create_defaults_the_display_name() {
    for name in [None, Some("")] {
        let harness = build(
            PROJECT_OPTIONS,
            &VertexSettings::default(),
            name,
            None,
            &[(200, DONE)],
        );
        store(&harness, File::default())
            .await
            .expect("a created corpus");
        assert_eq!(
            json_body(&harness.http.calls()[0]),
            expected(r#"{"displayName":"litellm-rag-corpus"}"#)
        );
    }
}

#[tokio::test]
async fn corpus_create_drops_an_empty_description() {
    let options = r#"{"vertex_project":"proj","description":""}"#;
    let harness = build(
        options,
        &VertexSettings::default(),
        Some("named"),
        None,
        &[(200, DONE)],
    );
    store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(
        json_body(&harness.http.calls()[0]),
        expected(r#"{"displayName":"named"}"#)
    );
}

#[tokio::test]
async fn corpus_create_merges_the_embedding_model() {
    let options = r#"{"vertex_project":"proj","embedding_model":"text-embedding-005","vector_db_config":{"a":1,"ragEmbeddingModelConfig":{"keep":true}}}"#;
    let harness = build(
        options,
        &VertexSettings::default(),
        None,
        None,
        &[(200, DONE)],
    );
    store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(
        json_body(&harness.http.calls()[0]),
        expected(
            r#"{"displayName":"litellm-rag-corpus","vectorDbConfig":{"a":1,"ragEmbeddingModelConfig":{"vertexPredictionEndpoint":{"endpoint":"text-embedding-005"}}}}"#
        )
    );
}

#[tokio::test]
async fn corpus_create_sends_vector_db_config_alone() {
    let options = r#"{"vertex_project":"proj","vector_db_config":{"a":1}}"#;
    let harness = build(
        options,
        &VertexSettings::default(),
        None,
        None,
        &[(200, DONE)],
    );
    store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(
        json_body(&harness.http.calls()[0]),
        expected(r#"{"displayName":"litellm-rag-corpus","vectorDbConfig":{"a":1}}"#)
    );
}

#[tokio::test]
async fn corpus_create_drops_an_empty_vector_db_config() {
    let options = r#"{"vertex_project":"proj","vector_db_config":{}}"#;
    let harness = build(
        options,
        &VertexSettings::default(),
        None,
        None,
        &[(200, DONE)],
    );
    store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(
        json_body(&harness.http.calls()[0]),
        expected(r#"{"displayName":"litellm-rag-corpus"}"#)
    );
}

#[tokio::test]
async fn corpus_create_rejects_a_non_object_vector_db_config() {
    let options = r#"{"vertex_project":"proj","vector_db_config":"text"}"#;
    let harness = build(options, &VertexSettings::default(), None, None, &[]);
    let error = store(&harness, File::default())
        .await
        .expect_err("a vector db config error");
    assert_eq!(error.to_string(), "`vector_db_config` must be an object");
    assert!(harness.http.calls().is_empty());
}

#[tokio::test]
async fn corpus_create_done_operation_returns_the_corpus_name() {
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(201, DONE)],
    );
    let (corpus, file_id) = store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(corpus.as_deref(), Some("corpora/1"));
    assert_eq!(file_id, None);
    assert_eq!(harness.http.calls().len(), 1);
}

#[tokio::test]
async fn corpus_create_done_without_a_name_returns_an_empty_corpus() {
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, r#"{"done":true}"#)],
    );
    let file = File {
        content: Some(b"data".as_slice()),
        filename: Some("n.txt"),
        ..File::default()
    };
    let (corpus, file_id) = store(&harness, file).await.expect("an empty corpus");
    assert_eq!(corpus.as_deref(), Some(""));
    assert_eq!(file_id, None);
    assert_eq!(harness.http.calls().len(), 1);
}

#[tokio::test]
async fn corpus_create_existing_corpus_skips_the_create() {
    let harness = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[],
    );
    let (corpus, file_id) = store(&harness, File::default())
        .await
        .expect("an existing corpus");
    assert_eq!(corpus.as_deref(), Some("corpora/7"));
    assert_eq!(file_id, None);
    assert!(harness.http.calls().is_empty());
    assert!(harness.token.credentials_seen().is_empty());
}

#[tokio::test(start_paused = true)]
async fn poll_pending_operation_polls_until_done() {
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, NOT_DONE), (200, DONE)],
    );
    let started = tokio::time::Instant::now();
    let (corpus, _) = store(&harness, File::default())
        .await
        .expect("a polled corpus");
    assert_eq!(
        tokio::time::Instant::now() - started,
        Duration::from_secs(2)
    );
    assert_eq!(corpus.as_deref(), Some("corpora/1"));
    let calls = harness.http.calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[1].kind, Kind::Get);
    assert_eq!(
        calls[1].url,
        "https://us-central1-aiplatform.googleapis.com/v1beta1/operations/1"
    );
    assert_eq!(calls[1].timeout, Duration::from_secs(60));
    assert_eq!(calls[2].kind, Kind::Get);
    assert_eq!(harness.token.credentials_seen().len(), 1);
    assert_eq!(harness.token.projects_seen(), [Some("proj".to_owned())]);
}

#[tokio::test(start_paused = true)]
async fn poll_operation_error_fails() {
    let replies = [
        (200, NOT_DONE),
        (200, r#"{"done":true,"error":{"code":5,"message":"boom"}}"#),
    ];
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &replies,
    );
    let error = store(&harness, File::default())
        .await
        .expect_err("a failed operation");
    assert_eq!(
        error.to_string(),
        r#"Operation failed: {"code":5,"message":"boom"}"#
    );
}

#[tokio::test(start_paused = true)]
async fn poll_null_operation_error_fails() {
    let replies = [(200, NOT_DONE), (200, r#"{"done":true,"error":null}"#)];
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &replies,
    );
    let error = store(&harness, File::default())
        .await
        .expect_err("a failed operation");
    assert_eq!(error.to_string(), "Operation failed: null");
}

#[tokio::test(start_paused = true)]
async fn poll_empty_corpus_name_fails() {
    let replies = [
        (200, NOT_DONE),
        (200, r#"{"done":true,"response":{"name":""}}"#),
    ];
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &replies,
    );
    let error = store(&harness, File::default())
        .await
        .expect_err("a nameless operation");
    assert_eq!(
        error.to_string(),
        r#"No corpus name in operation response: {"done":true,"response":{"name":""}}"#
    );
}

#[tokio::test(start_paused = true)]
async fn poll_non_200_status_fails() {
    let replies = [(200, NOT_DONE), (500, "unavailable")];
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &replies,
    );
    let error = store(&harness, File::default())
        .await
        .expect_err("a failed poll");
    assert_eq!(error.to_string(), "Failed to poll operation: unavailable");
}

#[tokio::test(start_paused = true)]
async fn poll_gives_up_after_thirty_attempts() {
    let replies = [(200_u16, NOT_DONE); 30];
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &replies,
    );
    let started = tokio::time::Instant::now();
    let error = store(&harness, File::default())
        .await
        .expect_err("a timeout");
    assert_eq!(error.to_string(), "Operation timed out after 30 attempts");
    assert_eq!(
        tokio::time::Instant::now() - started,
        Duration::from_secs(60)
    );
    assert_eq!(harness.http.calls().len(), 31);
    assert_eq!(harness.token.credentials_seen().len(), 1);
}

#[tokio::test]
async fn upload_posts_the_multipart_form() {
    let harness = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, FILE_REPLY)],
    );
    let file = File {
        content: Some(b"data".as_slice()),
        filename: Some("notes.txt"),
        content_type: Some("text/plain"),
        existing_file_id: None,
    };
    let (corpus, file_id) = store(&harness, file).await.expect("an uploaded file");
    assert_eq!(corpus.as_deref(), Some("corpora/7"));
    assert_eq!(file_id.as_deref(), Some("files/1"));
    let calls = harness.http.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].kind, Kind::Multipart);
    assert_eq!(calls[0].url, UPLOAD_URL);
    assert_eq!(calls[0].filename, "notes.txt");
    assert_eq!(calls[0].file, b"data".to_vec());
    assert_eq!(calls[0].content_type, "text/plain");
    assert_eq!(calls[0].timeout, Duration::from_secs(300));
    assert_eq!(harness.token.projects_seen(), [Some("proj".to_owned())]);
    assert_eq!(
        json_body(&calls[0]),
        expected(r#"{"rag_file":{"display_name":"notes.txt"}}"#)
    );
}

#[tokio::test]
async fn upload_description_comes_from_file_description() {
    let options =
        r#"{"vertex_project":"proj","vector_store_id":"corpora/7","file_description":"described"}"#;
    let harness = build(
        options,
        &VertexSettings::default(),
        None,
        None,
        &[(200, FILE_REPLY)],
    );
    let file = File {
        content: Some(b"data".as_slice()),
        filename: Some("notes.txt"),
        ..File::default()
    };
    store(&harness, file).await.expect("an uploaded file");
    assert_eq!(
        json_body(&harness.http.calls()[0]),
        expected(r#"{"rag_file":{"display_name":"notes.txt","description":"described"}}"#)
    );
}

#[tokio::test]
async fn upload_chunking_config_carries_only_nonzero_sizes() {
    let strategy = chunking(&[("chunk_size", "500"), ("chunk_overlap", "0")]);
    let harness = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        Some(strategy),
        &[(200, FILE_REPLY)],
    );
    let file = File {
        content: Some(b"data".as_slice()),
        filename: Some("notes.txt"),
        ..File::default()
    };
    store(&harness, file).await.expect("an uploaded file");
    assert_eq!(
        json_body(&harness.http.calls()[0]),
        expected(
            r#"{"rag_file":{"display_name":"notes.txt"},"upload_rag_file_config":{"rag_file_transformation_config":{"rag_file_chunking_config":{"fixed_length_chunking":{"chunk_size":500}}}}}"#
        )
    );
}

#[tokio::test]
async fn upload_omits_the_chunking_config_for_zero_sizes() {
    let strategy = chunking(&[("chunk_size", "0"), ("chunk_overlap", "0")]);
    let harness = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        Some(strategy),
        &[(200, FILE_REPLY)],
    );
    let file = File {
        content: Some(b"data".as_slice()),
        filename: Some("notes.txt"),
        ..File::default()
    };
    store(&harness, file).await.expect("an uploaded file");
    assert_eq!(
        json_body(&harness.http.calls()[0]),
        expected(r#"{"rag_file":{"display_name":"notes.txt"}}"#)
    );
}

#[tokio::test]
async fn upload_skips_empty_content() {
    let harness = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[],
    );
    let file = File {
        content: Some(b"".as_slice()),
        filename: Some("notes.txt"),
        ..File::default()
    };
    let (corpus, file_id) = store(&harness, file).await.expect("a skipped upload");
    assert_eq!(corpus.as_deref(), Some("corpora/7"));
    assert_eq!(file_id, None);
    assert!(harness.http.calls().is_empty());
    assert!(harness.token.credentials_seen().is_empty());
}

#[tokio::test]
async fn upload_skips_an_empty_filename() {
    let harness = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[],
    );
    let file = File {
        content: Some(b"data".as_slice()),
        filename: Some(""),
        ..File::default()
    };
    let (_, file_id) = store(&harness, file).await.expect("a skipped upload");
    assert_eq!(file_id, None);
    assert!(harness.http.calls().is_empty());
}

#[tokio::test]
async fn upload_skips_an_empty_corpus_id() {
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, r#"{"done":true}"#)],
    );
    let file = File {
        content: Some(b"data".as_slice()),
        filename: Some("notes.txt"),
        ..File::default()
    };
    let (corpus, file_id) = store(&harness, file).await.expect("an empty corpus");
    assert_eq!(corpus.as_deref(), Some(""));
    assert_eq!(file_id, None);
    assert_eq!(harness.http.calls()[0].kind, Kind::Json);
    assert_eq!(harness.http.calls().len(), 1);
}

#[tokio::test]
async fn upload_file_id_prefers_rag_file_name() {
    let reply = r#"{"ragFile":{"name":"files/1"},"name":"top"}"#;
    let harness = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, reply)],
    );
    let file = File {
        content: Some(b"data".as_slice()),
        filename: Some("notes.txt"),
        ..File::default()
    };
    let (_, file_id) = store(&harness, file).await.expect("an uploaded file");
    assert_eq!(file_id.as_deref(), Some("files/1"));
}

#[tokio::test]
async fn upload_file_id_falls_back_to_the_top_level_name() {
    let harness = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(201, r#"{"name":"top"}"#)],
    );
    let file = File {
        content: Some(b"data".as_slice()),
        filename: Some("notes.txt"),
        ..File::default()
    };
    let (_, file_id) = store(&harness, file).await.expect("an uploaded file");
    assert_eq!(file_id.as_deref(), Some("top"));
}

#[tokio::test]
async fn upload_file_id_is_empty_when_absent() {
    let harness = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, "{}")],
    );
    let file = File {
        content: Some(b"data".as_slice()),
        filename: Some("notes.txt"),
        ..File::default()
    };
    let (_, file_id) = store(&harness, file).await.expect("an uploaded file");
    assert_eq!(file_id.as_deref(), Some(""));
}

#[tokio::test]
async fn upload_unparseable_response_returns_uploaded() {
    let harness = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, "not json")],
    );
    let file = File {
        content: Some(b"data".as_slice()),
        filename: Some("notes.txt"),
        ..File::default()
    };
    let (_, file_id) = store(&harness, file).await.expect("an uploaded file");
    assert_eq!(file_id.as_deref(), Some("uploaded"));
}

#[tokio::test]
async fn upload_non_object_rag_file_returns_uploaded() {
    let reply = r#"{"ragFile":"text","name":"top"}"#;
    let harness = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, reply)],
    );
    let file = File {
        content: Some(b"data".as_slice()),
        filename: Some("notes.txt"),
        ..File::default()
    };
    let (_, file_id) = store(&harness, file).await.expect("an uploaded file");
    assert_eq!(file_id.as_deref(), Some("uploaded"));
}

#[tokio::test]
async fn error_create_status_failure_carries_the_body() {
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(400, "bad request")],
    );
    let error = store(&harness, File::default())
        .await
        .expect_err("a create failure");
    assert_eq!(
        error.to_string(),
        "Failed to create RAG corpus: bad request"
    );
    assert_eq!(harness.http.calls().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn error_poll_status_failure_carries_the_body() {
    let replies = [(200, NOT_DONE), (503, "unavailable")];
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &replies,
    );
    let error = store(&harness, File::default())
        .await
        .expect_err("a poll failure");
    assert_eq!(error.to_string(), "Failed to poll operation: unavailable");
    assert_eq!(harness.http.calls().len(), 2);
}

#[tokio::test]
async fn error_upload_status_failure_keeps_the_created_corpus() {
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, DONE), (500, "boom")],
    );
    let file = File {
        content: Some(b"data".as_slice()),
        filename: Some("notes.txt"),
        ..File::default()
    };
    let error = store(&harness, file).await.expect_err("an upload failure");
    assert_eq!(error.to_string(), "Failed to upload file: boom");
    let calls = harness.http.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].kind, Kind::Json);
    assert_eq!(
        json_body(&calls[0]),
        expected(r#"{"displayName":"litellm-rag-corpus"}"#)
    );
    assert_eq!(calls[1].kind, Kind::Multipart);
}

#[tokio::test]
async fn error_create_non_json_response_is_a_decode_error() {
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, "noise")],
    );
    let error = store(&harness, File::default())
        .await
        .expect_err("a decode failure");
    assert_eq!(
        error.to_string(),
        "the create corpus response is not valid JSON"
    );
}

#[tokio::test(start_paused = true)]
async fn error_poll_non_object_response_is_a_shape_error() {
    let replies = [(200, NOT_DONE), (200, r#""not an object""#)];
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &replies,
    );
    let error = store(&harness, File::default())
        .await
        .expect_err("a shape failure");
    assert_eq!(error.to_string(), "the poll response is not a JSON object");
}

#[tokio::test]
async fn error_non_boolean_done_is_a_field_error() {
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, r#"{"done":"yes"}"#)],
    );
    let error = store(&harness, File::default())
        .await
        .expect_err("a field failure");
    assert_eq!(error.to_string(), "`done` must be a boolean");
}

#[tokio::test]
async fn secret_token_never_reaches_the_logs() {
    let (logs, _guard) = capture();
    let harness = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, FILE_REPLY)],
    );
    let file = File {
        content: Some(b"data".as_slice()),
        filename: Some("notes.txt"),
        ..File::default()
    };
    store(&harness, file).await.expect("an uploaded file");
    let text = logs.text();
    assert!(text.contains("Uploading file to RAG corpus"));
    assert!(text.contains("Upload complete. File ID: files/1"));
    assert!(!text.contains(TOKEN));
}

#[tokio::test]
async fn secret_credentials_stay_out_of_logs_and_errors() {
    const SECRET: &str = "private-key-material";
    let (logs, _guard) = capture();
    let credentials = format!(
        r#"{{"type":"service_account","client_email":"sa@example.iam","private_key":"{SECRET}"}}"#
    );
    let options = format!(
        r#"{{"vertex_project":"proj","vertex_credentials":{}}}"#,
        serde_json::to_string(&credentials).expect("a JSON string")
    );
    let harness = build(
        &options,
        &VertexSettings::default(),
        None,
        None,
        &[(200, DONE), (500, "boom")],
    );
    let file = File {
        content: Some(b"data".as_slice()),
        filename: Some("notes.txt"),
        ..File::default()
    };
    let error = store(&harness, file).await.expect_err("an upload failure");
    assert_eq!(harness.token.credentials_seen(), vec![true]);
    assert!(!logs.text().contains(SECRET));
    assert!(!error.to_string().contains(SECRET));
    assert!(!format!("{error:?}").contains(SECRET));
}
