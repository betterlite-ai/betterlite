use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_openai::text_completion_transport::logging::SecretHeaders;
use betterlite_openai::text_completion_transport::sdk::SdkError;
use betterlite_streaming::provider_chunk_decode::error::ProviderChunkError;
use thiserror::Error;

// Python's getattr: Absent means the attribute does not exist, Present(None) means it is None
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reported<T> {
    Absent,
    Present(T),
}

// One exception as the streaming paths see it, with the attributes the async except block reads
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Raised {
    pub class: String,
    pub status: Reported<Option<u16>>,
    pub message: Reported<Option<String>>,
    pub headers: Option<SecretHeaders>,
    // a truthy error.response and the headers the async fallback would read off it
    pub response_headers: Option<SecretHeaders>,
    pub body: Option<Json>,
    pub text: String,
}

impl Raised {
    // AzureOpenAIError(status_code=status, message=message)
    #[must_use]
    pub fn azure(status: u16, message: String) -> Self {
        Self {
            class: "AzureOpenAIError".to_owned(),
            status: Reported::Present(Some(status)),
            message: Reported::Present(Some(message.clone())),
            headers: None,
            response_headers: None,
            body: None,
            text: message,
        }
    }

    // an exception an SDK client or our transport raised: class plus whichever attributes it carries
    #[must_use]
    pub fn raised(
        class: String,
        status: Reported<Option<u16>>,
        message: Reported<Option<String>>,
        text: String,
    ) -> Self {
        Self {
            class,
            status,
            message,
            headers: None,
            response_headers: None,
            body: None,
            text,
        }
    }

    // ValueError(message): Python's ValueError carries no status_code, message or body attribute
    #[must_use]
    pub fn value_error(message: String) -> Self {
        Self {
            class: "ValueError".to_owned(),
            status: Reported::Absent,
            message: Reported::Absent,
            headers: None,
            response_headers: None,
            body: None,
            text: message,
        }
    }

    #[must_use]
    pub fn key_error(name: &str) -> Self {
        Self {
            class: "KeyError".to_owned(),
            status: Reported::Absent,
            message: Reported::Absent,
            headers: None,
            response_headers: None,
            body: None,
            text: format!("'{name}'"),
        }
    }

    #[must_use]
    pub fn plain(text: String) -> Self {
        Self {
            class: "Exception".to_owned(),
            status: Reported::Absent,
            message: Reported::Absent,
            headers: None,
            response_headers: None,
            body: None,
            text,
        }
    }

    // A byte stream that failed while it was read, after the client call returned
    #[must_use]
    pub fn sdk(error: &SdkError) -> Self {
        match error {
            SdkError::Status(status) => Self {
                class: "APIStatusError".to_owned(),
                status: Reported::Present(status.status),
                message: Reported::Present(Some(status.message.clone())),
                headers: Some(SecretHeaders::from_response(&status.headers)),
                response_headers: None,
                body: None,
                text: status.message.clone(),
            },
            SdkError::Decode(_) | SdkError::Sse(_) | SdkError::Other { .. } => Self {
                class: "APIError".to_owned(),
                status: Reported::Absent,
                message: Reported::Absent,
                headers: None,
                response_headers: None,
                body: None,
                text: error.to_string(),
            },
        }
    }
}

// One failure as betterlite reports it after the sync raise or the async except block
#[derive(Debug, PartialEq, Eq)]
pub struct AzureStreamFailure {
    pub class: String,
    pub status: Reported<Option<u16>>,
    pub message: Reported<Option<String>>,
    pub headers: Option<SecretHeaders>,
    pub body: Option<Json>,
    pub text: String,
}

impl AzureStreamFailure {
    // the sync path raises the exception as the client made it
    #[must_use]
    pub fn raised(raised: Raised) -> Self {
        Self {
            class: raised.class,
            status: raised.status,
            message: raised.message,
            headers: raised.headers,
            body: raised.body,
            text: raised.text,
        }
    }

    // status_code=getattr(e, "status_code", 500), message=getattr(e, "message", str(e)),
    // headers=getattr(e, "headers", None), body=getattr(e, "body", None), then the response fallback
    #[must_use]
    pub fn wrapped(raised: Raised) -> Self {
        let status = match raised.status {
            Reported::Absent => Reported::Present(Some(500)),
            Reported::Present(value) => Reported::Present(value),
        };
        let message = match raised.message {
            Reported::Absent => Reported::Present(Some(raised.text.clone())),
            Reported::Present(value) => Reported::Present(value),
        };
        let headers = raised.headers.or(raised.response_headers);
        Self {
            class: "AzureOpenAIError".to_owned(),
            status,
            message,
            headers,
            body: raised.body,
            text: raised.text,
        }
    }

    #[must_use]
    pub fn message(&self) -> &str {
        match &self.message {
            Reported::Absent => &self.text,
            Reported::Present(Some(message)) => message,
            Reported::Present(None) => "None",
        }
    }
}

#[derive(Debug, Error)]
pub enum StreamingError {
    #[error("{}", .0.message())]
    Azure(AzureStreamFailure),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error(transparent)]
    Chunk(#[from] ProviderChunkError),
    #[error(transparent)]
    Usage(#[from] serde_json::Error),
}

impl From<Raised> for StreamingError {
    fn from(raised: Raised) -> Self {
        Self::Azure(AzureStreamFailure::raised(raised))
    }
}
