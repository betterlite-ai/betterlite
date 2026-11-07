use std::any::type_name_of_val;
use std::error::Error;

use betterlite_mcp::client::sessions::error::SessionError;
use tracing::{debug, error, warn};

pub(crate) fn list_failure<Source, Item>(
    error: SessionError<Source>,
    raise_on_error: bool,
    operation: &str,
    server: &str,
) -> Result<Vec<Item>, SessionError<Source>>
where
    Source: Error + Send + Sync + 'static,
{
    if was_cancelled(&error, operation) {
        return Err(error);
    }
    if raise_on_error {
        return Err(error);
    }
    log_failure(&error, operation, server);
    Ok(Vec::new())
}

pub(crate) fn detail_failure<Output, Source>(
    error: SessionError<Source>,
    operation: &str,
    server: &str,
) -> Result<Output, SessionError<Source>>
where
    Source: Error + Send + Sync + 'static,
{
    if was_cancelled(&error, operation) {
        return Err(error);
    }
    debug!("MCP client {operation} traceback:\n{}", error_chain(&error));
    log_failure(&error, operation, server);
    Err(error)
}

fn was_cancelled<Source>(error: &SessionError<Source>, operation: &str) -> bool
where
    Source: Error + Send + Sync + 'static,
{
    if !matches!(error, SessionError::Cancelled) {
        return false;
    }
    warn!("MCP client {operation} was cancelled");
    true
}

fn log_failure<Source>(error: &SessionError<Source>, operation: &str, server: &str)
where
    Source: Error + Send + Sync + 'static,
{
    let type_name = type_name_of_val(error);
    let error_type = type_name.rsplit("::").next().unwrap_or(type_name);
    error!(
        "MCP client {operation} failed - Error Type: {error_type}, Error: {error}, Server: {server}"
    );
    if is_broken_stream(error) {
        error!(
            "MCP client detected broken connection/stream during {operation} - the MCP server may have crashed, disconnected, or timed out"
        );
    }
}

pub(crate) fn is_broken_stream<Source>(error: &SessionError<Source>) -> bool
where
    Source: Error + Send + Sync + 'static,
{
    if matches!(error, SessionError::Stream(_)) {
        return true;
    }
    let name = type_name_of_val(error);
    ["BrokenResourceError", "Broken"]
        .iter()
        .any(|pattern| name.contains(pattern))
}

fn error_chain<Source>(error: &Source) -> String
where
    Source: Error,
{
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}
