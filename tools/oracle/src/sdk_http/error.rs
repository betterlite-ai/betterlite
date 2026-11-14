use std::io;
use std::process::ExitStatus;
use std::str::Utf8Error;
use std::string::FromUtf8Error;

use betterlite_config::loader::ConfigError;
use betterlite_core::api::messages_stream_logging::ChunkError;
use betterlite_env::EnvError;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::worker::WorkerError;
use betterlite_openai::chat::RequestError;
use betterlite_openai::chat::deployment::DeploymentError;
use betterlite_openai::chat::execution::ChatError;
use betterlite_provider::api::vector_store_contracts::error::VectorStoreEmbeddingError;
use betterlite_redact::error::RedactionError;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SdkHttpError {
    #[error("sdk_http case is {bytes} bytes, past the {limit} byte limit")]
    CaseTooLarge { bytes: usize, limit: usize },
    #[error("sdk_http result is {bytes} bytes, past the {limit} byte limit")]
    ResultTooLarge { bytes: usize, limit: usize },
    #[error("sdk_http input is not valid UTF-8: {0}")]
    Utf8(#[source] Utf8Error),
    #[error("cannot parse sdk_http case: {0}")]
    CaseParse(#[source] Box<serde_saphyr::Error>),
    #[error("cannot parse sdk_http result: {0}")]
    ResultParse(#[source] serde_json::Error),
    #[error("invalid sdk_http case: {0}")]
    Invalid(String),
    #[error("the SDK sent {count} requests, past the limit of {limit}")]
    TooManyRequests { count: usize, limit: usize },
    #[error("request {index} carries a body of {bytes} bytes, past the {limit} byte limit")]
    BodyTooLarge {
        index: usize,
        bytes: usize,
        limit: usize,
    },
    #[error("the execution deadline passed and the work was stopped")]
    Deadline,
    #[error("the execution was cancelled and the work was stopped")]
    Cancelled,
    #[error("cannot start the SDK process: {0}")]
    Spawn(#[source] io::Error),
    #[error("SDK process I/O failed: {0}")]
    Io(#[source] io::Error),
    #[error("SDK process pipe thread panicked")]
    Join,
    #[error("SDK process exited with {status}: {stderr}")]
    Exited { status: ExitStatus, stderr: String },
    #[error("SDK process wrote more than {limit} bytes")]
    OutputTooLarge { limit: usize },
    #[error("cannot serialize an SDK input: {0}")]
    Serialize(#[source] serde_json::Error),
    #[error("cannot load the native service config: {0}")]
    ConfigLoad(#[source] Box<ConfigError>),
    #[error("cannot resolve the native deployment: {0}")]
    Deployment(#[source] Box<DeploymentError>),
    #[error("cannot resolve the native proxy policy: {0}")]
    ProxyPolicy(#[source] EnvError),
    #[error("cannot build the native HTTP client: {0}")]
    Client(#[source] Box<HttpError>),
    #[error("the native service rejected the chat request: {0}")]
    ChatParse(#[source] Box<RequestError>),
    #[error("the native service failed outside a provider answer: {0}")]
    Chat(#[source] Box<ChatError>),
    #[error("the vector store embedding executor failed: {source}")]
    Embedding {
        #[source]
        source: Box<VectorStoreEmbeddingError>,
    },
    #[error("the provider error body is not UTF-8: {0}")]
    ErrorBodyUtf8(#[source] FromUtf8Error),
    #[error("the provider error body does not decode: {0}")]
    ErrorBody(#[source] serde_json::Error),
    #[error("the native error class {0} has no LiteLLM SDK projection yet")]
    Unprojected(String),
    #[error("the provider request URL is not valid: {0}")]
    Url(#[source] Box<url::ParseError>),
    #[error("the provider request failed: {0}")]
    Transport(#[source] Box<TransportError>),
    #[error("the provider response stream failed: {0}")]
    Stream(#[source] Box<HttpError>),
    #[error("the provider response header {0} is not text")]
    HeaderValue(String),
    #[error("the provider streamed more than {limit} bytes")]
    StreamTooLarge { limit: usize },
    #[error("the stream frame is not UTF-8: {0}")]
    FrameUtf8(#[source] FromUtf8Error),
    #[error("cannot start the stream logging worker: {0}")]
    Worker(#[source] WorkerError),
    #[error("cannot convert the stream chunk: {0}")]
    Chunk(#[source] ChunkError),
    #[error("cannot build the fixture redactor: {0}")]
    Redactor(#[source] RedactionError),
    #[error("cannot build the fixture logging state: {0}")]
    LoggingState(#[source] LoggingStateError),
}
