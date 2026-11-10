use std::str::Utf8Error;

use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_http::error::HttpError;
use betterlite_openai::bytez_wire::error::BytezError;
use betterlite_openai::chat::provider_error::rendered;
use betterlite_redact::secret::Redactor;
use betterlite_streaming::provider_chunk_decode::error::ProviderChunkError;
use betterlite_streaming::stream_chunk_shaping::StreamShapeError;
use thiserror::Error;
use url::ParseError;

use super::transport::ClientError;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct BytezChunkError {
    pub status_code: u16,
    pub error_type: String,
    pub param: Option<String>,
    pub message: String,
}

#[derive(Debug, Error)]
pub enum BytezStreamError {
    #[error(transparent)]
    Api(#[from] BytezError),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error("the Bytez stream body failed")]
    Body {
        #[source]
        source: HttpError,
    },
    #[error("the Bytez stream body is larger than {limit} bytes")]
    TooLarge { limit: usize },
    #[error("the Bytez stream is not UTF-8")]
    InvalidUtf8 {
        #[source]
        source: Utf8Error,
    },
    #[error("the Bytez request body cannot be written as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Bytez request URL is not valid")]
    Url {
        #[source]
        source: ParseError,
    },
    #[error("the Bytez server sent nothing for 300 seconds")]
    Timeout,
    #[error("the Bytez stream is finished")]
    StopIteration,
    #[error(transparent)]
    Chunk(Box<BytezChunkError>),
}

impl BytezStreamError {
    // BytezCustomStreamWrapper.chunk_creator re-raises StopIteration and maps every other error through exception_type
    #[must_use]
    pub fn from_chunk(
        source: &ProviderChunkError,
        model: &str,
        provider: &str,
        redactor: &Redactor,
    ) -> Self {
        if matches!(source, ProviderChunkError::StopIteration) {
            return Self::StopIteration;
        }
        let class = match source {
            ProviderChunkError::Shape {
                source: StreamShapeError::Repetition { .. },
            } => internal_server(source),
            ProviderChunkError::Shape { .. }
            | ProviderChunkError::StopIteration
            | ProviderChunkError::ParseResponse { .. }
            | ProviderChunkError::RejectedResponse { .. }
            | ProviderChunkError::ProviderReported { .. }
            | ProviderChunkError::UnknownStream
            | ProviderChunkError::VertexBlocked
            | ProviderChunkError::FinishReasonError { .. }
            | ProviderChunkError::NotAString { .. }
            | ProviderChunkError::UnexpectedChunk { .. }
            | ProviderChunkError::MissingField { .. }
            | ProviderChunkError::InvalidField { .. }
            | ProviderChunkError::InvalidUtf8 { .. }
            | ProviderChunkError::InvalidChunk { .. } => dispatch(
                &DispatchInput::Raw(Box::new(raw_exception(source, model, provider))),
                redactor,
            ),
        };
        let failure = rendered(&class);
        Self::Chunk(Box::new(BytezChunkError {
            status_code: failure.status,
            error_type: failure.error_type,
            param: failure.param,
            message: source.to_string(),
        }))
    }
}

// raise_on_model_repetition raises an already-mapped litellm.InternalServerError, which exception_type hands back as-is
fn internal_server(source: &ProviderChunkError) -> ErrorClass {
    ErrorClass::InternalServer(Box::new(ErrorContext {
        message: source.to_string(),
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }))
}

fn raw_exception(source: &ProviderChunkError, model: &str, provider: &str) -> RawException {
    RawException {
        model: model.to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: source.to_string(),
        status_code: None,
        status_code_is_synthesized: false,
        message: Some(source.to_string()),
        body: None,
        exception_type: source.class().to_owned(),
    }
}
