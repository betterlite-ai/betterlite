use std::str;
use std::time::Duration;

use serde::Deserialize;
use serde_json::value::RawValue;

use super::classification::{TransportError, classify};
use super::construct::PanwPrismaAirs;
use super::payload::{ScanPayload, ScanRequest, build_payload};
use super::verdict::{Action, Failure, ScanVerdict, VerdictError};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const SCAN_PATH: &str = "/v1/scan/sync/request";

#[derive(Debug, Deserialize)]
struct ToolEventEcosystem {
    metadata: Option<EcosystemMetadata>,
}

#[derive(Debug, Deserialize)]
struct EcosystemMetadata {
    ecosystem: Option<String>,
}

// Mirrors LiteLLM's fn _call_panw_api.
pub async fn call_panw_api(guardrail: &PanwPrismaAirs, request: ScanRequest<'_>) -> ScanVerdict {
    if request.tool_event.is_none() && request.content.trim().is_empty() {
        return ScanVerdict::synthetic(Action::Allow, "empty", None);
    }
    // tr_id is optional in the AIRS API, so only MCP tool_events may go without a call id; every other path fails closed.
    let call_id = request.call_id.filter(|id| !id.is_empty());
    if call_id.is_none() && !is_mcp_tool_event(request.tool_event) {
        return ScanVerdict::synthetic(Action::Block, "missing_call_id", Some(Failure::Permanent));
    }
    let scan = ScanRequest { call_id, ..request };
    let payload = build_payload(&guardrail.settings, &scan);
    match post_scan(guardrail, &payload).await {
        Ok(verdict) => verdict,
        Err(error) => classify(&error, scan.tool_event),
    }
}

async fn post_scan(
    guardrail: &PanwPrismaAirs,
    payload: &ScanPayload<'_>,
) -> Result<ScanVerdict, TransportError> {
    let timeout =
        Duration::try_from_secs_f64(guardrail.settings.timeout).unwrap_or(DEFAULT_TIMEOUT);
    let url = format!("{}{SCAN_PATH}", guardrail.settings.api_base);
    let response = guardrail
        .client
        .post(url)
        .headers(guardrail.headers.clone())
        .json(payload)
        .timeout(timeout)
        .send()
        .await
        .map_err(send_error)?;
    let status = response.status();
    let body = match read_body(response).await {
        // The status stays authoritative when an error body is too large to read.
        Err(TransportError::TooLarge { .. }) if !status.is_success() => Vec::new(),
        result => result?,
    };
    if !status.is_success() {
        // The lint table refuses bytes that are not UTF-8, so an undecodable body carries no profile phrase; the status still classifies it.
        let text = str::from_utf8(&body).unwrap_or_default().to_owned();
        return Err(TransportError::Status {
            status: status.as_u16(),
            body: text,
        });
    }
    match ScanVerdict::from_body(&body) {
        Ok(verdict) => {
            log_verdict(&verdict);
            Ok(verdict)
        }
        Err(VerdictError::NotJson(source)) => Err(TransportError::NotJson(source)),
        Err(VerdictError::Shape(source)) => {
            tracing::error!("PANW Prisma AIRS: Invalid API response format: {source}");
            Ok(ScanVerdict::synthetic(Action::Block, "api_error", None))
        }
    }
}

fn is_mcp_tool_event(tool_event: Option<&RawValue>) -> bool {
    let Some(event) = tool_event else {
        return false;
    };
    serde_json::from_str::<ToolEventEcosystem>(event.get())
        .ok()
        .and_then(|parsed| parsed.metadata)
        .and_then(|metadata| metadata.ecosystem)
        .is_some_and(|ecosystem| ecosystem == "mcp")
}

fn send_error(source: reqwest::Error) -> TransportError {
    if source.is_timeout() {
        TransportError::Timeout(source)
    } else if source.is_builder() {
        TransportError::Unexpected(source)
    } else {
        TransportError::Network(source)
    }
}

async fn read_body(mut response: reqwest::Response) -> Result<Vec<u8>, TransportError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(send_error)? {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(TransportError::TooLarge {
                limit: MAX_RESPONSE_BYTES,
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn log_verdict(verdict: &ScanVerdict) {
    if verdict.action == Action::Block
        && let Some(error) = verdict.error.as_deref()
    {
        let lowered = error.to_lowercase();
        if lowered.contains("profile")
            && ["not found", "required", "invalid"]
                .iter()
                .any(|phrase| lowered.contains(phrase))
        {
            tracing::error!(
                "PANW Prisma AIRS: Profile configuration error. Ensure your API key has a linked profile in Strata Cloud Manager, or provide 'profile_name' or 'profile_id' in config/metadata. PANW API response error: {error}"
            );
        }
    }
    tracing::debug!(
        action = ?verdict.action,
        category = verdict.category.as_deref().unwrap_or("unknown"),
        "PANW Prisma AIRS: Scan result"
    );
}
