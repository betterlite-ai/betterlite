use std::any::type_name_of_val;
use std::error::Error;

use betterlite_mcp::client::sessions::error::SessionError;
use tracing::{debug, error, warn};

#[derive(Clone, Copy, Debug)]
pub(crate) enum FailureMode {
    List { raise_on_error: bool },
    Detail,
}

pub(crate) fn finish_failure<Output, Source>(
    error: SessionError<Source>,
    mode: FailureMode,
    operation: &str,
    server: &str,
) -> Result<Output, SessionError<Source>>
where
    Source: Error + Send + Sync + 'static,
    Output: Default,
{
    if matches!(&error, SessionError::Cancelled) {
        warn!("MCP client {operation} was cancelled");
        return Err(error);
    }
    match mode {
        FailureMode::List {
            raise_on_error: true,
        } => Err(error),
        FailureMode::List {
            raise_on_error: false,
        } => {
            log_failure(&error, operation, server);
            Ok(Output::default())
        }
        FailureMode::Detail => {
            let mut traceback = error.to_string();
            let mut source = error.source();
            while let Some(cause) = source {
                traceback.push_str(": ");
                traceback.push_str(&cause.to_string());
                source = cause.source();
            }
            debug!("MCP client {operation} traceback:\n{traceback}");
            log_failure(&error, operation, server);
            Err(error)
        }
    }
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
