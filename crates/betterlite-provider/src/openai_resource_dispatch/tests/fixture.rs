use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::openai_credentials::Settings;
use betterlite_http::client_kind::ClientKind;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::batches::error::SdkError as BatchSdkError;
use betterlite_openai::batches::records::{BatchCreate, BatchPage, BatchQuery};
use betterlite_openai::batches::sdk::{
    BatchesSdk, ClientConstructor, ClientSettings, SdkArguments,
};
use betterlite_openai::containers::OpenAiContainerConfig;
use betterlite_openai::containers::costs::ContainerCosts;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::evals::openai::OpenAiEvalsConfig;
use betterlite_openai::files::FilesClient;
use betterlite_openai::files::transport::{FileTransport, StreamReply};
use betterlite_openai::fine_tuning::client::{
    BuildError, ClientArgs, ClientBuilder, JobsApi, SdkError as JobSdkError,
};
use betterlite_openai::fine_tuning::job::FineTuneCreate;
use betterlite_openai::openai_base_config::Object;
use futures_util::StreamExt as _;
use futures_util::stream::iter;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::de::DeserializeOwned;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Part {
    pub(crate) name: String,
    pub(crate) file_name: Option<String>,
    pub(crate) content_type: Option<String>,
    pub(crate) data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WireBody {
    Empty,
    Bytes(Vec<u8>),
    Multipart(Vec<Part>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Wire {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) query: Vec<(String, String)>,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: WireBody,
}

impl Wire {
    #[must_use]
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header == name)
            .map(|(_, value)| value.as_str())
    }
}

pub(crate) fn project(request: &OutboundRequest) -> Wire {
    let body = match &request.body {
        RequestBody::Empty => WireBody::Empty,
        RequestBody::Json(bytes) => WireBody::Bytes(bytes.clone()),
        RequestBody::Bytes { data, .. } => WireBody::Bytes(data.clone()),
        RequestBody::Multipart { parts, .. } => WireBody::Multipart(
            parts
                .iter()
                .map(|part| Part {
                    name: part.name.clone(),
                    file_name: part.file_name.clone(),
                    content_type: part.content_type.clone(),
                    data: part.data.clone(),
                })
                .collect(),
        ),
    };
    Wire {
        method: request.method.as_str().to_owned(),
        path: request.url.path().to_owned(),
        query: request.query.clone(),
        headers: request
            .headers
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_owned(),
                    value.to_str().unwrap_or_default().to_owned(),
                )
            })
            .collect(),
        body,
    }
}

pub(crate) struct Fixture {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    chunks: Vec<Vec<u8>>,
    seen: Mutex<Vec<Wire>>,
}

impl Fixture {
    pub(crate) fn replied(status: u16, body: &[u8]) -> Self {
        Self {
            status,
            headers: vec![(
                String::from("content-type"),
                String::from("application/json"),
            )],
            body: body.to_vec(),
            chunks: Vec::new(),
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn streamed(chunks: Vec<Vec<u8>>) -> Self {
        Self {
            status: 200,
            headers: vec![
                (
                    String::from("content-type"),
                    String::from("application/binary"),
                ),
                (String::from("x-request-id"), String::from("req-stream")),
            ],
            body: Vec::new(),
            chunks,
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn seen(&self) -> Vec<Wire> {
        self.seen.lock().unwrap().clone()
    }

    fn answer(&self, request: &OutboundRequest) -> UpstreamResponse {
        self.seen.lock().unwrap().push(project(request));
        UpstreamResponse {
            status: self.status,
            headers: header_map(&self.headers),
            body: self.body.clone(),
        }
    }
}

fn header_map(pairs: &[(String, String)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        headers.insert(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    headers
}

impl FileTransport for Fixture {
    fn send_buffered(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(Ok(self.answer(&request)))
    }

    fn send_stream(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<StreamReply, TransportError>> + Send {
        let reply = self.answer(&request);
        let chunks: Vec<Result<Vec<u8>, HttpError>> = self.chunks.iter().cloned().map(Ok).collect();
        ready(Ok(StreamReply {
            status: reply.status,
            headers: reply.headers,
            body: iter(chunks).boxed(),
        }))
    }
}

pub(crate) fn files_client() -> FilesClient {
    FilesClient {
        api_key: SecretString::from("sk-files"),
        base_url: Url::parse("https://api.files.test/v1/").unwrap(),
        organization: Some(String::from("org-files")),
        timeout: Duration::from_secs(600),
        max_retries: 2,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BatchCall {
    Create(BatchCreate),
    Retrieve(BatchQuery),
    Cancel(BatchQuery),
    List {
        after: Option<String>,
        limit: Option<i64>,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum BatchAnswer {
    Body(Vec<u8>),
    Failure,
}

pub(crate) struct BatchSdk {
    pub(crate) calls: Arc<Mutex<Vec<BatchCall>>>,
    pub(crate) answer: BatchAnswer,
}

impl BatchSdk {
    fn record(&self, call: BatchCall) {
        self.calls.lock().unwrap().push(call);
    }

    fn reply(&self) -> Result<Vec<u8>, BatchSdkError> {
        match &self.answer {
            BatchAnswer::Body(bytes) => Ok(bytes.clone()),
            BatchAnswer::Failure => Err(BatchSdkError::Status {
                status: 400,
                error_type: Some(String::from("invalid_request_error")),
                param: Some(String::from("input_file_id")),
                code: Some(String::from("invalid_file")),
                message: String::from("the input file is not a batch file"),
            }),
        }
    }
}

impl BatchesSdk for BatchSdk {
    type Page = BatchPage;

    fn kind(&self) -> ClientKind {
        ClientKind::Asynchronous
    }

    fn create(
        &self,
        request: &BatchCreate,
    ) -> impl Future<Output = Result<Vec<u8>, BatchSdkError>> + Send {
        self.record(BatchCall::Create(request.clone()));
        ready(self.reply())
    }

    fn retrieve(
        &self,
        request: &BatchQuery,
    ) -> impl Future<Output = Result<Vec<u8>, BatchSdkError>> + Send {
        self.record(BatchCall::Retrieve(request.clone()));
        ready(self.reply())
    }

    fn cancel(
        &self,
        request: &BatchQuery,
    ) -> impl Future<Output = Result<Vec<u8>, BatchSdkError>> + Send {
        self.record(BatchCall::Cancel(request.clone()));
        ready(self.reply())
    }

    fn list(
        &self,
        after: Option<&str>,
        limit: Option<i64>,
    ) -> impl Future<Output = Result<BatchPage, BatchSdkError>> + Send {
        self.record(BatchCall::List {
            after: after.map(str::to_owned),
            limit,
        });
        ready(match &self.answer {
            BatchAnswer::Body(_) => Ok(batch_page()),
            BatchAnswer::Failure => Err(BatchSdkError::Status {
                status: 429,
                error_type: Some(String::from("rate_limit_error")),
                param: None,
                code: Some(String::from("rate_limited")),
                message: String::from("slow down"),
            }),
        })
    }
}

pub(crate) struct BatchConstructor {
    pub(crate) calls: Arc<Mutex<Vec<BatchCall>>>,
    pub(crate) answer: BatchAnswer,
}

impl ClientConstructor for BatchConstructor {
    type Client = BatchSdk;

    fn construct(
        &self,
        _arguments: &SdkArguments,
        _kind: ClientKind,
    ) -> Result<Option<BatchSdk>, BatchSdkError> {
        Ok(Some(BatchSdk {
            calls: Arc::clone(&self.calls),
            answer: self.answer.clone(),
        }))
    }
}

pub(crate) fn batch_page() -> BatchPage {
    serde_json::from_str(
        r#"{"data":[{"id":"batch_listed","status":"completed"}],"has_more":false,"object":"list","first_id":"batch_listed","last_id":"batch_listed"}"#,
    )
    .unwrap()
}

pub(crate) fn batch_body(id: &str) -> Vec<u8> {
    format!(
        r#"{{"id":"{id}","object":"batch","endpoint":"/v1/embeddings","input_file_id":"file-1","completion_window":"24h","status":"completed","created_at":1700000000,"metadata":{{"team":"core"}}}}"#
    )
    .into_bytes()
}

pub(crate) fn batch_settings() -> ClientSettings {
    ClientSettings::default()
}

pub(crate) fn object(json: &str) -> Object {
    serde_json::from_str(json).unwrap()
}

pub(crate) fn parse<T>(json: &str) -> T
where
    T: DeserializeOwned,
{
    serde_json::from_str(json).unwrap()
}

pub(crate) fn assert_json<T>(actual: &T, json: &str)
where
    T: DeserializeOwned + PartialEq + Debug,
{
    assert_eq!(*actual, parse::<T>(json));
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum JobCall {
    Create {
        model: String,
        training_file: String,
        suffix: Option<String>,
    },
    List {
        after: Option<String>,
        limit: Option<i64>,
    },
    Retrieve(String),
    Cancel(String),
}

pub(crate) struct JobSdk {
    pub(crate) calls: Arc<Mutex<Vec<JobCall>>>,
    pub(crate) answer: Result<Vec<u8>, String>,
}

impl JobSdk {
    fn record(&self, call: JobCall) -> Result<Vec<u8>, JobSdkError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        self.answer.clone().map_err(|message| JobSdkError {
            status: 400,
            error_type: Some(String::from("invalid_request_error")),
            param: Some(String::from("training_file")),
            code: None,
            message,
        })
    }
}

impl JobsApi for JobSdk {
    fn is_async(&self) -> bool {
        false
    }

    fn create(&self, data: &FineTuneCreate) -> Result<Vec<u8>, JobSdkError> {
        self.record(JobCall::Create {
            model: data.model.clone(),
            training_file: data.training_file.clone(),
            suffix: data.suffix.clone(),
        })
    }

    fn list(&self, after: Option<&str>, limit: Option<i64>) -> Result<Vec<u8>, JobSdkError> {
        self.record(JobCall::List {
            after: after.map(str::to_owned),
            limit,
        })
    }

    fn retrieve(&self, job_id: &str) -> Result<Vec<u8>, JobSdkError> {
        self.record(JobCall::Retrieve(job_id.to_owned()))
    }

    fn cancel(&self, job_id: &str) -> Result<Vec<u8>, JobSdkError> {
        self.record(JobCall::Cancel(job_id.to_owned()))
    }
}

pub(crate) struct NoBuilder;

impl ClientBuilder for NoBuilder {
    fn build(
        &self,
        _is_async: bool,
        _args: &ClientArgs,
    ) -> Result<Option<Box<dyn JobsApi>>, BuildError> {
        Ok(None)
    }
}

pub(crate) fn job_body(id: &str) -> Vec<u8> {
    format!(
        r#"{{"id":"{id}","object":"fine_tuning.job","created_at":1700000000,"model":"gpt-test","organization_id":"org-1","result_files":[],"status":"queued","training_file":"file-1","hyperparameters":{{}}}}"#
    )
    .into_bytes()
}

#[derive(Debug)]
pub(crate) struct ZeroCosts;

impl ContainerCosts for ZeroCosts {
    fn code_interpreter_cost(&self, _sessions: u32, _provider: &str) -> Decimal {
        Decimal::ZERO
    }
}

pub(crate) fn container_config() -> OpenAiContainerConfig {
    OpenAiContainerConfig {
        settings: Settings::default(),
        globals: LiteLlmGlobals {
            api_base: None,
            api_key: Some(SecretString::from("sk-container")),
            openai_key: None,
            organization: None,
        },
        env_api_key: None,
        costs: Arc::new(ZeroCosts),
    }
}

pub(crate) fn evals_config() -> OpenAiEvalsConfig {
    OpenAiEvalsConfig {
        api_key: Some(SecretString::from("sk-evals")),
        openai_key: None,
        env_api_key: None,
    }
}
