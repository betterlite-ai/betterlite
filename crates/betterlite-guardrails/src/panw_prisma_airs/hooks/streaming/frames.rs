use indexmap::IndexMap;
use serde::Serialize;
use serde_json::value::RawValue;

use crate::guardrail_message_scope::StreamItem;
use crate::panw_prisma_airs::error_detail::{ErrorType, GuardrailHttpError};
use crate::panw_prisma_airs::verdict::Action;

use super::StreamError;

const GENERIC_MESSAGE: &str = "Security scan failed - streaming response blocked for safety";
// json.dumps cannot fail on LiteLLM's plain dict; a serializer failure still yields the safety message.
const FALLBACK_FRAME: &str = "data: {\"error\":{\"message\":\"Security scan failed - streaming response blocked for safety\",\"type\":\"guardrail_scan_error\",\"code\":500}}\n\n";

// LiteLLM replaces the detail's string code with the HTTP status and re-wraps the body under "error".
#[derive(Serialize)]
struct ErrorFrame<'frame> {
    error: ErrorFrameBody<'frame>,
}

#[derive(Serialize)]
struct ErrorFrameBody<'frame> {
    #[serde(skip_serializing_if = "Option::is_none")]
    action: Option<Action>,
    #[serde(flatten)]
    scan_fields: &'frame IndexMap<String, Box<RawValue>>,
    message: &'frame str,
    #[serde(rename = "type")]
    kind: ErrorType,
    code: u16,
    guardrail: &'frame str,
    category: &'frame str,
}

#[derive(Serialize)]
struct GenericFrame<'frame> {
    error: GenericFrameBody<'frame>,
}

#[derive(Serialize)]
struct GenericFrameBody<'frame> {
    message: &'frame str,
    #[serde(rename = "type")]
    kind: ErrorType,
    code: u16,
    guardrail: &'frame str,
}

#[must_use]
pub fn error_item(guardrail_name: &str, error: StreamError) -> StreamItem {
    match error {
        StreamError::Http(http) => http_item(&http),
        other @ (StreamError::Data { .. }
        | StreamError::Assembly { .. }
        | StreamError::Masking(_)) => generic_item(guardrail_name, &other),
    }
}

fn http_item(error: &GuardrailHttpError) -> StreamItem {
    let body = &error.detail.error;
    let frame = ErrorFrame {
        error: ErrorFrameBody {
            action: body.action,
            scan_fields: &body.scan_fields,
            message: &body.message,
            kind: body.kind,
            code: error.status,
            guardrail: &body.guardrail,
            category: &body.category,
        },
    };
    item(serde_json::to_string(&frame))
}

fn generic_item(guardrail_name: &str, error: &StreamError) -> StreamItem {
    tracing::error!("PANW Prisma AIRS streaming error: {error}");
    let frame = GenericFrame {
        error: GenericFrameBody {
            message: GENERIC_MESSAGE,
            kind: ErrorType::GuardrailScanError,
            code: 500,
            guardrail: guardrail_name,
        },
    };
    item(serde_json::to_string(&frame))
}

fn item(json: Result<String, serde_json::Error>) -> StreamItem {
    match json {
        Ok(body) => StreamItem::Bytes(format!("data: {body}\n\n").into_bytes()),
        Err(error) => {
            tracing::error!("PANW Prisma AIRS streaming error frame could not be built: {error}");
            StreamItem::Bytes(FALLBACK_FRAME.as_bytes().to_vec())
        }
    }
}
