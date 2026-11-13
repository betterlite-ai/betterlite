use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use std::io::Error as IoError;
use std::path::PathBuf;
use thiserror::Error;
use tokio::task::JoinError;
#[derive(Debug, Error)]
pub enum OcrError {
    #[error("the OCR provider answered with status {status}: {body}")]
    Provider {
        status: u16,
        body: String,
        headers: Vec<(String, String)>,
    },
    #[error("the OCR file is empty or could not be read")]
    EmptyFile,
    #[error("could not read the OCR file {path}")]
    FileRead {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("the OCR document preparation task failed")]
    DocumentTask(#[source] JoinError),
    #[error("invalid OCR MIME type: {0}")]
    InvalidMimeType(String),
    #[error("Cohere parse accepts only image_url documents, not document_url or PDF input")]
    CohereImageOnly,
    #[error("invalid req_format, expected \"native\" or \"litellm\"")]
    RequestFormat,
    #[error("invalid OCR request field: {path}")]
    RequestField { path: String },
    #[error("missing required field: {0}")]
    MissingField(&'static str),
    #[error("a document_url or image_url document needs its URL")]
    MissingDocumentUrl,
    #[error("the OCR document is not a valid data URI")]
    InvalidDataUri,
    #[error("Reducto needs a reducto:// id or a data URI, and plain HTTP URLs are not accepted")]
    ReductoSource,
    #[error("the inline OCR document exceeds the size limit")]
    InlineDocumentTooLarge,
    #[error("the OCR document URL is blocked by the network policy")]
    BlockedDocumentUrl,
    #[error("OCR document downloads are disabled")]
    DownloadDisabled,
    #[error("the OCR document download exceeds the size limit")]
    DownloadTooLarge,
    #[error("the OCR document download exceeded the redirect limit")]
    TooManyRedirects,
    #[error("invalid OCR pages: {0}")]
    Pages(String),
    #[error("invalid OCR features")]
    Features,
    #[error("an OCR model cannot be a dot path segment")]
    DotModel,
    #[error("the OCR response exceeds the size limit of {limit} bytes")]
    TooLarge { limit: usize },
    #[error("invalid OCR response field: {path}")]
    ResponseField { path: String },
    #[error("the OCR response has no content")]
    EmptyContent,
    #[error("the OCR redirect has no location")]
    MissingRedirectLocation,
    #[error("the OCR redirect location is invalid")]
    InvalidRedirect,
    #[error("the OCR operation ended with status {0}")]
    OperationStatus(String),
    #[error("an OCR response number is out of range: {0}")]
    NumericRange(&'static str),
    #[error("the accepted OCR response has no valid operation-location")]
    PollLocation,
    #[error("the operation-location must share the submission origin and carry no credentials")]
    PollOrigin,
    #[error("OCR polling timed out")]
    PollTimeout,
    #[error("unsupported by the OCR path: {0}")]
    Unsupported(&'static str),
    #[error("invalid OCR provider: {0}")]
    InvalidProvider(String),
    #[error("invalid model: {provider} has no model {model:?} - use one of: {}", supported.join(", "))]
    InvalidModel {
        provider: &'static str,
        model: String,
        supported: &'static [&'static str],
    },
    #[error("invalid OCR request: {0}")]
    InvalidRequest(String),
    #[error("invalid OCR response: {0}")]
    InvalidResponse(String),
    #[error("missing Azure AI credentials: set AZURE_AI_API_KEY or configure Entra ID")]
    MissingAzureAiCredentials,
    #[error(
        "missing Azure Document Intelligence credentials: set AZURE_DOCUMENT_INTELLIGENCE_API_KEY or configure Entra ID"
    )]
    MissingDocIntelCredentials,
    #[error("missing REDUCTO_API_KEY: set it in the environment or pass api_key")]
    MissingReductoApiKey,
    #[error("secret resolution failed")]
    Secret(#[source] OcrSecretError),
    #[error("OCR authentication failed")]
    Auth(#[source] OcrAuthError),
    #[error(transparent)]
    Transport(OcrTransportError),
    #[error(transparent)]
    Params(OcrParamsError),
    #[error(transparent)]
    Headers(OcrHeaderError),
    #[error(transparent)]
    Http(HttpError),
}
#[derive(Debug, Error)]
pub enum OcrTransportError {
    #[error("the provider answered with status {status}: {body}")]
    Http { status: u16, body: String },
    #[error("the OCR request timed out")]
    Timeout,
    #[error(transparent)]
    Failed(TransportError),
}
#[derive(Debug, Error)]
pub enum OcrSecretError {
    #[error("could not resolve the secret {name}: {detail}")]
    Resolution { name: String, detail: String },
}
#[derive(Debug, Error)]
pub enum OcrAuthError {
    #[error(
        "missing {provider} API key: set the {environment_variable} environment variable or pass api_key"
    )]
    MissingApiKey {
        provider: &'static str,
        environment_variable: &'static str,
    },
    #[error("missing {provider} API base: {guidance}")]
    MissingApiBase {
        provider: &'static str,
        guidance: &'static str,
    },
    #[error("invalid authentication configuration: {detail}")]
    InvalidConfiguration { detail: String },
}
#[derive(Debug, Error)]
pub enum OcrParamsError {
    #[error("invalid request: extra_body must be an object")]
    ExtraBody,
    #[error("invalid request: the provider body must be a JSON object")]
    Body,
}
#[derive(Debug, Error)]
pub enum OcrHeaderError {
    #[error("the header name {name:?} is not valid")]
    Name {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the {name} header value is not valid")]
    Value {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OcrErrorFields {
    pub status: u16,
    pub error_type: &'static str,
    pub param: Option<&'static str>,
    pub code: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcrErrorKind {
    Request,
    Response,
    Other,
}
impl OcrError {
    #[must_use]
    pub const fn http_status_code(&self) -> Option<u16> {
        if let Self::Provider { status, .. } = self {
            return Some(*status);
        }
        if let Self::Transport(OcrTransportError::Http { status, .. }) = self {
            return Some(*status);
        }
        if matches!(self, Self::Transport(OcrTransportError::Timeout)) {
            return Some(408);
        }
        match self.kind() {
            OcrErrorKind::Request => Some(400),
            OcrErrorKind::Response | OcrErrorKind::Other => None,
        }
    }
    #[must_use]
    #[rustfmt::skip]
    pub const fn kind(&self) -> OcrErrorKind {
        match self {
            Self::EmptyFile | Self::InvalidMimeType(_) | Self::CohereImageOnly | Self::RequestFormat
            | Self::RequestField { .. } | Self::MissingField(_) | Self::MissingDocumentUrl
            | Self::InvalidDataUri | Self::ReductoSource | Self::InlineDocumentTooLarge
            | Self::BlockedDocumentUrl | Self::DownloadDisabled | Self::DownloadTooLarge
            | Self::TooManyRedirects | Self::Pages(_) | Self::Features | Self::DotModel
            | Self::InvalidRequest(_) | Self::InvalidProvider(_) | Self::InvalidModel { .. }
            | Self::Params(_) | Self::Headers(_) | Self::Http(_) => OcrErrorKind::Request,
            Self::TooLarge { .. } | Self::ResponseField { .. } | Self::EmptyContent
            | Self::MissingRedirectLocation | Self::InvalidRedirect | Self::OperationStatus(_)
            | Self::NumericRange(_) | Self::PollLocation | Self::PollOrigin | Self::PollTimeout
            | Self::InvalidResponse(_) => OcrErrorKind::Response,
            Self::Provider { .. } | Self::FileRead { .. } | Self::DocumentTask(_)
            | Self::Unsupported(_) | Self::MissingAzureAiCredentials | Self::MissingDocIntelCredentials
            | Self::MissingReductoApiKey | Self::Secret(_) | Self::Auth(_) | Self::Transport(_) => {
                OcrErrorKind::Other
            }
        }
    }
    #[must_use]
    pub fn fields(&self) -> OcrErrorFields {
        let status = self.http_status_code().unwrap_or(500);
        OcrErrorFields {
            status,
            error_type: openai_error_type(status),
            param: None,
            code: status.to_string(),
        }
    }
}
// the proxy's fallback when an exception carries no type of its own
const fn openai_error_type(status: u16) -> &'static str {
    match status {
        401 => "authentication_error",
        403 => "permission_error",
        429 => "rate_limit_error",
        below_500 if below_500 < 500 => "invalid_request_error",
        _ => "internal_server_error",
    }
}
mod tests;
