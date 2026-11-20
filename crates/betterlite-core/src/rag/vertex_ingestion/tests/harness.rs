use std::collections::VecDeque;
use std::io;
use std::str::from_utf8;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::credentials::CredentialValues;
use betterlite_config::rag::document::ingest::VertexAIVectorStoreOptions;
use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_models::decisions::Extra;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::RawValue;
use thiserror::Error;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::rag::ingestion::IngestionPorts;
use crate::rag::ingestion::IngestionState;
use crate::rag::ingestion::config::{ChunkingStrategyMap, IngestionOptions};
use crate::rag::ingestion::credentials::{CredentialLookup, CredentialLookupError};
use crate::rag::ingestion::ocr::{OcrError, OcrOperation, OcrRequest, OcrResponse};
use crate::rag::ingestion::store::{RagStore as _, StoreRequest, StoreResult};
use crate::rag::ingestion::upload::{FetchedResponse, UrlFetch, UrlFetchError};

use super::super::VertexIngestion;
use super::super::error::VertexIngestionError;
use super::super::ports::{
    GetRequest, HttpReply, JsonRequest, MultipartRequest, PortError, VertexIngestionPorts,
    VertexRagHttp, VertexTokenPort,
};
use super::TOKEN;

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
pub(super) struct FakeToken {
    calls: Mutex<Vec<(bool, Option<String>)>>,
}

impl FakeToken {
    pub(super) fn credentials_seen(&self) -> Vec<bool> {
        self.records()
            .into_iter()
            .map(|(credentials, _)| credentials)
            .collect()
    }

    pub(super) fn projects_seen(&self) -> Vec<Option<String>> {
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
    ) -> Result<(SecretString, String), PortError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((credentials.is_some(), project.map(str::to_owned)));
        Ok((
            SecretString::from(TOKEN.to_owned()),
            project.map(str::to_owned).unwrap_or_default(),
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Json,
    Get,
    Multipart,
}

#[derive(Debug, Clone)]
pub(super) struct Call {
    pub(super) kind: Kind,
    pub(super) url: String,
    pub(super) token: String,
    pub(super) headers: Vec<String>,
    pub(super) body: String,
    pub(super) filename: String,
    pub(super) file: Vec<u8>,
    pub(super) content_type: String,
    pub(super) timeout: Duration,
}

pub(super) struct ScriptedHttp {
    replies: Mutex<VecDeque<(u16, Vec<u8>)>>,
    calls: Mutex<Vec<Call>>,
}

impl ScriptedHttp {
    pub(super) fn new(replies: &[(u16, &'static str)]) -> Self {
        Self {
            replies: Mutex::new(
                replies
                    .iter()
                    .map(|(status, body)| (*status, body.as_bytes().to_vec()))
                    .collect(),
            ),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn push_reply(&self, status: u16, body: &[u8]) {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back((status, body.to_vec()));
    }

    pub(super) fn calls(&self) -> Vec<Call> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn reply(&self) -> Result<HttpReply, PortError> {
        let popped = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let Some((status, body)) = popped else {
            return Err(PortError::new(ExhaustedScript));
        };
        Ok(HttpReply { status, body })
    }

    fn record(&self, call: Call) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }
}

fn text(bytes: &[u8]) -> String {
    from_utf8(bytes).expect("a UTF-8 request part").to_owned()
}

enum Part<'request> {
    Json(&'request JsonRequest<'request>),
    Get(&'request GetRequest<'request>),
    Multipart(&'request MultipartRequest<'request>),
}

impl Part<'_> {
    fn into_call(self) -> Call {
        match self {
            Part::Json(request) => Call {
                kind: Kind::Json,
                url: request.url.to_owned(),
                token: request.token.expose_secret().to_owned(),
                headers: header_names(&["Authorization", "Content-Type"]),
                body: text(request.body),
                filename: String::new(),
                file: Vec::new(),
                content_type: String::new(),
                timeout: request.timeout,
            },
            Part::Get(request) => Call {
                kind: Kind::Get,
                url: request.url.to_owned(),
                token: request.token.expose_secret().to_owned(),
                headers: header_names(&["Authorization"]),
                body: String::new(),
                filename: String::new(),
                file: Vec::new(),
                content_type: String::new(),
                timeout: request.timeout,
            },
            Part::Multipart(request) => Call {
                kind: Kind::Multipart,
                url: request.url.to_owned(),
                token: request.token.expose_secret().to_owned(),
                headers: header_names(&["Authorization", "X-Goog-Upload-Protocol"]),
                body: text(request.metadata),
                filename: request.filename.to_owned(),
                file: request.file.to_vec(),
                content_type: request.file_content_type.to_owned(),
                timeout: request.timeout,
            },
        }
    }
}

fn header_names(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[async_trait]
impl VertexRagHttp for ScriptedHttp {
    async fn post_json(&self, request: JsonRequest<'_>) -> Result<HttpReply, PortError> {
        self.record(Part::Json(&request).into_call());
        self.reply()
    }

    async fn get(&self, request: GetRequest<'_>) -> Result<HttpReply, PortError> {
        self.record(Part::Get(&request).into_call());
        self.reply()
    }

    async fn post_multipart(&self, request: MultipartRequest<'_>) -> Result<HttpReply, PortError> {
        self.record(Part::Multipart(&request).into_call());
        self.reply()
    }
}

#[derive(Debug, Error)]
#[error("the reply script is exhausted")]
struct ExhaustedScript;

pub(super) struct Harness {
    pub(super) ingestion: VertexIngestion,
    pub(super) token: Arc<FakeToken>,
    pub(super) http: Arc<ScriptedHttp>,
}

fn raw(source: &str) -> Box<RawValue> {
    serde_json::from_str(source).expect("a raw JSON value")
}

pub(super) fn chunking(entries: &[(&str, &str)]) -> ChunkingStrategyMap {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), raw(value)))
        .collect()
}

fn state(name: Option<&str>, strategy: Option<ChunkingStrategyMap>) -> IngestionState {
    IngestionState::new(
        IngestionOptions {
            name: name.map(str::to_owned),
            ocr: None,
            chunking_strategy: strategy,
            embedding: None,
            vector_store: None,
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

pub(super) fn settings(project: Option<&str>, location: Option<&str>) -> VertexSettings {
    let defaults = VertexSettings::default();
    VertexSettings {
        project: project.map(str::to_owned),
        location: location.map(str::to_owned),
        credentials: defaults.credentials,
        adc: defaults.adc,
        default_video_duration_seconds: defaults.default_video_duration_seconds,
    }
}

pub(super) fn build(
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
    let parsed: VertexAIVectorStoreOptions =
        serde_json::from_str(options).expect("vector store options");
    Harness {
        ingestion: VertexIngestion::new(state(name, strategy), parsed, settings, ports),
        token,
        http,
    }
}

#[derive(Default, Clone, Copy)]
pub(super) struct File<'call> {
    pub(super) content: Option<&'call [u8]>,
    pub(super) filename: Option<&'call str>,
    pub(super) content_type: Option<&'call str>,
    pub(super) existing_file_id: Option<&'call str>,
}

impl<'call> File<'call> {
    pub(super) fn new(content: &'call [u8], filename: &'call str) -> Self {
        Self {
            content: Some(content),
            filename: Some(filename),
            content_type: None,
            existing_file_id: None,
        }
    }
}

pub(super) async fn store(
    harness: &Harness,
    file: File<'_>,
) -> Result<StoreResult, VertexIngestionError> {
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

pub(super) fn json_body(call: &Call) -> Extra {
    serde_json::from_str(&call.body).expect("a JSON request body")
}

pub(super) fn expected(source: &str) -> Extra {
    serde_json::from_str(source).expect("an expected JSON body")
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn text(&self) -> String {
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

pub(super) fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}
