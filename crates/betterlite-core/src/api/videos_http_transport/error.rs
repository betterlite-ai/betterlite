use std::error::Error as StdError;
use std::time::Duration;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::error::ProviderError;
use thiserror::Error;

use super::client::{ClientError, ClientKind, HttpVerb, Sent};
use super::video_config::{Headers, VideoHttpConfig};

// A value the declared type cannot hold, named so the caller knows which field to fix
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("{field} must be {expected}")]
pub struct BadField {
    pub field: &'static str,
    pub expected: &'static str,
}

// What a send raised: httpx's post raises on a failing status and turns its own timeout into litellm.Timeout
#[derive(Debug)]
pub(crate) enum SendFailure {
    Status {
        status: u16,
        headers: Headers,
        text: String,
    },
    Timeout {
        message: String,
    },
    Other {
        message: String,
    },
}

#[derive(Debug, Error)]
pub enum VideoTransportError<ConfigError: StdError> {
    #[error("{}", .0.message)]
    Provider(Box<ProviderError>),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error(transparent)]
    Field(#[from] BadField),
}

pub(crate) fn failed_send(error: &TransportError, sent: Sent, elapsed: Duration) -> SendFailure {
    if sent.verb == HttpVerb::Post && is_timeout(error) {
        return SendFailure::Timeout {
            message: timeout_message(sent, elapsed),
        };
    }
    SendFailure::Other {
        message: error.to_string(),
    }
}

const fn is_timeout(error: &TransportError) -> bool {
    matches!(
        error,
        TransportError::ConnectTimeout(_) | TransportError::Deadline(_)
    )
}

fn timeout_message(sent: Sent, elapsed: Duration) -> String {
    let timeout = format_timeout(sent.timeout);
    match sent.mode {
        ClientKind::Sync => {
            format!("litellm.Timeout: Connection timed out after {timeout} seconds.")
        }
        ClientKind::Asynchronous => {
            let taken = elapsed_text(elapsed);
            format!(
                "litellm.Timeout: Connection timed out. Timeout passed={timeout}, time taken={taken} seconds"
            )
        }
    }
}

// round(elapsed, 3) printed as a Python float: three decimals at most, and a .0 when none are left
pub(crate) fn elapsed_text(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs_f64();
    let text = format!("{seconds:.3}");
    let trimmed = text.trim_end_matches('0');
    if trimmed.ends_with('.') {
        format!("{trimmed}0")
    } else {
        trimmed.to_owned()
    }
}

fn format_timeout(timeout: Option<Duration>) -> String {
    match timeout {
        None => "None".to_owned(),
        Some(value) if value.subsec_nanos() == 0 => value.as_secs().to_string(),
        Some(value) => value.as_secs_f64().to_string(),
    }
}

// _handle_error: a status keeps its code and headers, a timeout keeps 408, and anything else becomes a synthesized 500
#[must_use]
pub(crate) fn convert<C>(config: &C, failure: SendFailure) -> ProviderError
where
    C: VideoHttpConfig,
{
    let (message, status_code, headers, synthesized) = match failure {
        SendFailure::Status {
            status,
            headers,
            text,
        } => (text, status, headers, false),
        SendFailure::Timeout { message } => (message, 408_u16, Headers::new(), false),
        SendFailure::Other { message } => (message, 500_u16, Headers::new(), true),
    };
    let mut error = config.get_error_class(&message, status_code, &headers);
    error.status_code_is_synthesized = synthesized;
    error
}
