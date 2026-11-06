use std::error::Error;
use std::ops::AsyncFnOnce;
use std::sync::Arc;
use std::time::Duration;

use tokio::time::Instant;
use tracing::{debug, warn};

use crate::client::cleanup::bounded::{CleanupError, run_bounded_cleanup};
use crate::client::cleanup::failure::{McpError, ReadTimeout, as_mcp_read_timeout};

use super::client::SessionClient;
use super::error::{ExitFailure, SessionError};
use super::execute::execute_session_operation;
use super::runtime::{McpRuntime, SessionContext};

pub const HTTP_CLOSE_TIMEOUT: Duration = Duration::from_secs(1);

const fn endpoint<Runtime>(client: &SessionClient<Runtime>) -> &str
where
    Runtime: McpRuntime,
{
    if client.auth.server_url.is_empty() {
        "stdio"
    } else {
        client.auth.server_url.as_str()
    }
}

fn read_timeout<Source>(error: &SessionError<Source>) -> Option<ReadTimeout>
where
    Source: Error + Send + Sync + 'static,
{
    match error {
        SessionError::Protocol(failure) => as_mcp_read_timeout(&McpError {
            code: failure.code,
            message: failure.message.clone(),
            context: failure.context,
        }),
        SessionError::Runtime(_)
        | SessionError::Stream(_)
        | SessionError::Cancelled
        | SessionError::CleanupTimedOut
        | SessionError::CleanupTask(_)
        | SessionError::ReadTimeout(_)
        | SessionError::Configuration(_) => None,
    }
}

pub async fn run_with_session<Runtime, Output, Operation>(
    client: &mut SessionClient<Runtime>,
    operation: Operation,
    quiet_on_error: bool,
) -> Result<Output, SessionError<Runtime::Error>>
where
    Runtime: McpRuntime,
    Operation: AsyncFnOnce(
        &mut <Runtime::Session as SessionContext>::Session,
    ) -> Result<Output, SessionError<Runtime::Error>>,
{
    client.last_initialize_instructions = None;
    let mut http_client = None;
    let session_outcome = match client.create_transport_context() {
        Ok(contexts) => {
            http_client = contexts.http_client.clone();
            execute_session_operation(client, contexts, operation).await
        }
        Err(error) => Err(error),
    };
    let outcome = log_failure(client, session_outcome, quiet_on_error);
    let close_cancelled = close_http_client(client, http_client).await;
    match outcome {
        Ok(output) if !close_cancelled && !client.caller_cancelled.is_cancelled() => Ok(output),
        Ok(_) => Err(SessionError::Cancelled),
        Err(error) => Err(error),
    }
}

fn log_failure<Runtime, Output>(
    client: &SessionClient<Runtime>,
    outcome: Result<Output, SessionError<Runtime::Error>>,
    quiet_on_error: bool,
) -> Result<Output, SessionError<Runtime::Error>>
where
    Runtime: McpRuntime,
{
    match outcome {
        Ok(output) => Ok(output),
        Err(error) => {
            if let Some(timeout) = read_timeout(&error) {
                warn!(
                    "MCP client timed out after {:?} waiting for a valid MCP response from {}",
                    client.auth.timeout,
                    endpoint(client)
                );
                return Err(SessionError::ReadTimeout(timeout));
            }
            let endpoint = endpoint(client);
            if quiet_on_error {
                debug!("MCP client run_with_session failed for {endpoint}");
            } else {
                warn!("MCP client run_with_session failed for {endpoint}");
            }
            Err(error)
        }
    }
}

async fn close_http_client<Runtime>(
    client: &SessionClient<Runtime>,
    handle: Option<Arc<Runtime::HttpClient>>,
) -> bool
where
    Runtime: McpRuntime,
{
    let Some(client_handle) = handle else {
        return false;
    };
    let deadline = Instant::now()
        .checked_add(HTTP_CLOSE_TIMEOUT)
        .unwrap_or_else(Instant::now);
    let runtime = Arc::clone(&client.runtime);
    let close = runtime.close_http_client(Arc::clone(&client_handle));
    match run_bounded_cleanup(&client.tracker, &client.caller_cancelled, deadline, close).await {
        Ok(()) => false,
        Err(
            error @ (CleanupError::Cancelled | CleanupError::Operation(ExitFailure::Cancelled)),
        ) => {
            debug!("Error during http client cleanup: {error}");
            true
        }
        Err(error) => {
            debug!("Error during http client cleanup: {error}");
            false
        }
    }
}
