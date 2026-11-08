use std::error::Error;
use std::time::Duration;

use tracing::{debug, error, info, warn};

use crate::client::cleanup::failure::ReadTimeout;
use crate::client::sessions::client::SessionClient;
use crate::client::sessions::error::SessionError;
use crate::client::sessions::run::run_with_session;
use crate::client::sessions::runtime::{McpRuntime, SessionContext};
use crate::client::sessions::transport::TransportType;
use crate::tools::pagination::{McpSession, Timeouts, ToolError, list_tools_with_pagination};
use crate::tools::schemas::McpTool;

use super::result::{describe_failure, is_cancellation, names_broken_connection};

pub async fn list_tools<Runtime>(
    client: &mut SessionClient<Runtime>,
    listing_timeout: Duration,
    raise_on_error: bool,
) -> Result<Vec<McpTool>, SessionError<Runtime::Error>>
where
    Runtime: McpRuntime,
    <Runtime::Session as SessionContext>::Session: McpSession<Error = SessionError<Runtime::Error>>,
{
    let endpoint = if client.auth.server_url.is_empty() {
        "stdio".to_owned()
    } else {
        client.auth.server_url.clone()
    };
    debug!("MCP client listing tools from {endpoint}");
    let timeouts = Timeouts {
        client: client.auth.timeout,
        listing: listing_timeout,
        listing_deadline: Some(client.auth.timeout.max(listing_timeout)),
    };
    let outcome = run_with_session(
        client,
        async |session| {
            list_tools_with_pagination(session, timeouts)
                .await
                .map_err(listing_failure)
        },
        raise_on_error,
    )
    .await;
    finish_listing(
        outcome,
        &endpoint,
        client.settings.transport_type,
        raise_on_error,
    )
}

fn finish_listing<Source>(
    outcome: Result<Vec<McpTool>, SessionError<Source>>,
    endpoint: &str,
    transport: TransportType,
    raise_on_error: bool,
) -> Result<Vec<McpTool>, SessionError<Source>>
where
    Source: Error + Send + Sync + 'static,
{
    match outcome {
        Ok(tools) => {
            let names = tools.iter().map(|tool| tool.name.as_str());
            info!(
                "MCP client listed {} tools from {endpoint}: {:?}",
                tools.len(),
                names.collect::<Vec<_>>()
            );
            Ok(tools)
        }
        Err(candidate) if is_cancellation(&candidate) => {
            warn!("MCP client list_tools was cancelled");
            Err(candidate)
        }
        Err(candidate) => {
            log_list_failure(&candidate, endpoint, transport, raise_on_error);
            if raise_on_error {
                return Err(candidate);
            }
            Ok(Vec::new())
        }
    }
}

fn listing_failure<Source>(error: ToolError<SessionError<Source>>) -> SessionError<Source>
where
    Source: Error + Send + Sync + 'static,
{
    match error {
        ToolError::Session(source) => source,
        ToolError::Timeout(elapsed) => SessionError::ReadTimeout(ReadTimeout {
            message: elapsed.to_string(),
        }),
    }
}

fn log_list_failure<Source>(
    error: &SessionError<Source>,
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
            "MCP client list_tools failed - error type: {error_type}, server: {endpoint}, transport: {transport:?}"
        );
        if broken {
            debug!(
                "MCP client detected broken connection/stream during list_tools - the MCP server may have crashed, disconnected, or timed out"
            );
        }
    } else {
        error!(
            "MCP client list_tools failed - error type: {error_type}, server: {endpoint}, transport: {transport:?}"
        );
        if broken {
            error!(
                "MCP client detected broken connection/stream during list_tools - the MCP server may have crashed, disconnected, or timed out"
            );
        }
    }
}
