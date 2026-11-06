use serde::Deserialize;
use serde_json::value::RawValue;
use thiserror::Error;

use super::verdict::{Action, Failure, ScanVerdict};

const LOGGED_BODY_CHARS: usize = 500;
const PROFILE_ERROR_PHRASES: [&str; 3] =
    ["profile not found", "profile required", "invalid profile"];

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("timeout error: {0}")]
    Timeout(#[source] reqwest::Error),
    #[error("network/request error: {0}")]
    Network(#[source] reqwest::Error),
    #[error("unexpected error: {0}")]
    Unexpected(#[source] reqwest::Error),
    #[error("the scan response is larger than {limit} bytes")]
    TooLarge { limit: usize },
    #[error("the scan response is not JSON: {0}")]
    NotJson(#[source] serde_json::Error),
    #[error("HTTP {status} from the scan API")]
    Status { status: u16, body: String },
}

#[derive(Debug, Deserialize)]
struct ToolEventDiagnostics {
    metadata: Option<Box<RawValue>>,
    input: Option<Box<RawValue>>,
}

// Maps a failed scan call to the block verdict LiteLLM returns for it.
#[must_use]
pub fn classify(error: &TransportError, tool_event: Option<&RawValue>) -> ScanVerdict {
    match error {
        TransportError::Status { status, body } => classify_status(*status, body, tool_event),
        TransportError::Timeout(source) => {
            tracing::error!("PANW Prisma AIRS: Timeout error: {source}");
            ScanVerdict::synthetic(Action::Block, "timeout_error", Some(Failure::Transient))
        }
        TransportError::Network(source) => {
            tracing::error!("PANW Prisma AIRS: Network/request error: {source}");
            ScanVerdict::synthetic(Action::Block, "network_error", Some(Failure::Transient))
        }
        TransportError::Unexpected(_)
        | TransportError::TooLarge { .. }
        | TransportError::NotJson(_) => {
            tracing::error!("PANW Prisma AIRS: Unexpected error: {error}");
            ScanVerdict::synthetic(Action::Block, "api_error", Some(Failure::Transient))
        }
    }
}

fn classify_status(status: u16, body: &str, tool_event: Option<&RawValue>) -> ScanVerdict {
    let snippet: String = body.chars().take(LOGGED_BODY_CHARS).collect();
    if status == 400 {
        log_bad_request(&snippet, tool_event);
    }
    let lowered = body.to_lowercase();
    let is_profile_error = PROFILE_ERROR_PHRASES
        .iter()
        .any(|phrase| lowered.contains(phrase));
    if matches!(status, 401 | 403) || is_profile_error {
        tracing::error!(
            "PANW Prisma AIRS: Authentication/config error (HTTP {status}). Check API key and profile configuration."
        );
        return ScanVerdict::synthetic(Action::Block, "config_error", Some(Failure::Permanent));
    }
    let category = format!("http_{status}_error");
    if status == 429 || status >= 500 {
        tracing::error!("PANW Prisma AIRS: API error (HTTP {status}): {snippet}");
        return ScanVerdict::synthetic(Action::Block, &category, Some(Failure::Transient));
    }
    // A 400 was logged with diagnostics above; the other client errors must not bypass scanning.
    if status != 400 {
        tracing::error!("PANW Prisma AIRS: API error (HTTP {status}): {snippet}");
    }
    ScanVerdict::synthetic(Action::Block, &category, Some(Failure::Permanent))
}

fn log_bad_request(snippet: &str, tool_event: Option<&RawValue>) {
    let mut parts = vec!["PANW Prisma AIRS: HTTP 400 from AIRS API.".to_owned()];
    if let Some(event) = tool_event {
        let diagnostics: Option<ToolEventDiagnostics> = serde_json::from_str(event.get()).ok();
        let metadata = diagnostics
            .as_ref()
            .and_then(|found| found.metadata.as_deref())
            .map_or("None", RawValue::get);
        let input = diagnostics
            .as_ref()
            .and_then(|found| found.input.as_deref());
        parts.push(format!("tool_event.metadata={metadata}"));
        parts.push(format!(
            "input present={}, len={}",
            input.is_some(),
            input.map_or(0, |raw| raw.get().len())
        ));
    }
    parts.push(format!("response body: {snippet}"));
    tracing::error!("{}", parts.join(" | "));
}
