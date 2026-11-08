use std::error::Error;

use tracing::{debug, error, info, warn};

use crate::client::sessions::client::SessionClient;
use crate::client::sessions::error::SessionError;
use crate::client::sessions::run::run_with_session;
use crate::client::sessions::runtime::{McpRuntime, SessionContext};
use crate::client::sessions::transport::TransportType;
use crate::tools::tool_calls::CallRequest;

use super::progress::{ProgressForwarder, ProgressHost};
use super::result::{
    ToolCallResult, describe_failure, error_text_result, is_cancellation, names_broken_connection,
};
use super::session::ToolCallSession;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CallToolOptions {
    pub raise_on_error: bool,
    pub allow_input_required: bool,
}

#[must_use]
pub fn error_tool_result<Source>(error: &SessionError<Source>) -> ToolCallResult
where
    Source: Error + Send + Sync + 'static,
{
    let failure = describe_failure(error);
    let message = failure.message;
    error_text_result(failure.error_type, &message)
}

pub async fn call_tool<Runtime, Host>(
    client: &mut SessionClient<Runtime>,
    request: &CallRequest,
    host: Option<&Host>,
    options: CallToolOptions,
) -> Result<ToolCallResult, SessionError<Runtime::Error>>
where
    Runtime: McpRuntime,
    Host: ProgressHost + Sync,
    <Runtime::Session as SessionContext>::Session:
        ToolCallSession<Error = SessionError<Runtime::Error>>,
{
    let endpoint = if client.auth.server_url.is_empty() {
        "stdio".to_owned()
    } else {
        client.auth.server_url.clone()
    };
    info!("MCP client calling tool '{}'", request.name);
    let progress = ProgressForwarder {
        tool: &request.name,
        host,
    };
    let outcome = run_with_session(
        client,
        async |session| {
            debug!("MCP client sending tool call to session");
            session
                .call_tool(request, &progress, options.allow_input_required)
                .await
        },
        options.raise_on_error,
    )
    .await;
    finish_call(outcome, client, &request.name, &endpoint, options)
}

fn finish_call<Runtime>(
    outcome: Result<ToolCallResult, SessionError<Runtime::Error>>,
    client: &SessionClient<Runtime>,
    tool: &str,
    endpoint: &str,
    options: CallToolOptions,
) -> Result<ToolCallResult, SessionError<Runtime::Error>>
where
    Runtime: McpRuntime,
{
    match outcome {
        Ok(result) => {
            info!("MCP client tool call '{tool}' completed successfully");
            Ok(result)
        }
        Err(candidate) if is_cancellation(&candidate) => {
            warn!(
                "MCP client tool call timed out after {}s for {endpoint}",
                client.auth.timeout.as_secs_f64()
            );
            Err(candidate)
        }
        Err(candidate) => {
            debug!("MCP client tool call traceback: {candidate:?}");
            log_call_failure(
                &candidate,
                tool,
                endpoint,
                client.settings.transport_type,
                options.raise_on_error,
            );
            if options.raise_on_error {
                return Err(candidate);
            }
            Ok(error_tool_result(&candidate))
        }
    }
}

fn log_call_failure<Source>(
    error: &SessionError<Source>,
    tool: &str,
    endpoint: &str,
    transport: TransportType,
    raise_on_error: bool,
) where
    Source: Error + Send + Sync + 'static,
{
    let error_type = describe_failure(error).error_type;
    let broken = names_broken_connection(error);
    if raise_on_error {
        debug!(
            "MCP client call_tool failed - error type: {error_type}, tool: {tool}, server: {endpoint}, transport: {transport:?}"
        );
        if broken {
            debug!(
                "MCP client detected broken connection/stream - the MCP server may have crashed, disconnected, or timed out."
            );
        }
    } else {
        error!(
            "MCP client call_tool failed - error type: {error_type}, tool: {tool}, server: {endpoint}, transport: {transport:?}"
        );
        if broken {
            error!(
                "MCP client detected broken connection/stream - the MCP server may have crashed, disconnected, or timed out."
            );
        }
    }
}
