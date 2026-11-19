use serde_json::Value;

use crate::case::UpstreamAnswer;

mod tests;

const FIXTURE_PREFIX: &str = "/__mcp_fixture/";
const MARKER: &str = "$mcp_request_id";
const JSONRPC_VERSION: &str = "2.0";

#[derive(Debug, Clone)]
pub(crate) enum McpCorrelation {
    Outside,
    // the path stays with the id so a marked answer without one can name the request it answers
    Fixture { path: String, id: Option<Value> },
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum McpReplyError {
    #[error(
        "scripted answer on {path} uses the $mcp_request_id marker, but the request has no string or integer id"
    )]
    MarkerWithoutId { path: String },
}

pub(crate) fn correlation(path: &str, body: &Value) -> McpCorrelation {
    if !path.starts_with(FIXTURE_PREFIX) {
        return McpCorrelation::Outside;
    }
    let id = body
        .as_object()
        .and_then(|fields| fields.get("id"))
        .filter(|id| id.is_string() || id.is_i64() || id.is_u64())
        .cloned();
    McpCorrelation::Fixture {
        path: path.to_owned(),
        id,
    }
}

pub(crate) fn correlate(
    answer: &mut UpstreamAnswer,
    correlation: &McpCorrelation,
) -> Result<(), McpReplyError> {
    let McpCorrelation::Fixture { path, id } = correlation else {
        return Ok(());
    };
    let Some(body) = answer.body.as_mut() else {
        return Ok(());
    };
    let Some(fields) = body.as_object_mut() else {
        return Ok(());
    };
    if fields.get("jsonrpc").and_then(Value::as_str) != Some(JSONRPC_VERSION) {
        return Ok(());
    }
    let Some(Value::String(marker)) = fields.get("id") else {
        return Ok(());
    };
    if marker.as_str() != MARKER {
        return Ok(());
    }
    let Some(request_id) = id else {
        return Err(McpReplyError::MarkerWithoutId { path: path.clone() });
    };
    fields.insert("id".to_owned(), request_id.clone());
    Ok(())
}
