use std::error::Error;
use std::future::{Future, poll_fn};
use std::ops::AsyncFnOnce;
use std::pin::pin;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::client::cleanup::bounded::{CleanupError, run_bounded_cleanup};
use crate::client::cleanup::failure::{Failure, first_non_cancelled_cause};
use crate::client::cleanup::http_client::CleanupScope;

use super::client::SessionClient;
use super::error::{ExitFailure, SessionError, to_failure};
use super::runtime::{HttpClientHandle as _, McpRuntime, SessionContext};
use super::transport::{TransportContext as _, TransportContexts};

mod session;
use session::run_session_layer;

pub const SESSION_CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Shutdown {
    #[default]
    Clean,
    Expired,
    Cancelled,
}

struct Execution<Output, Source>
where
    Source: Error + Send + Sync + 'static,
{
    outcome: Result<Output, SessionError<Source>>,
    in_flight_cancelled: bool,
    shutdown: Shutdown,
    session_deadline: Option<Instant>,
    transport_open: bool,
}

impl<Output, Source> Execution<Output, Source>
where
    Source: Error + Send + Sync + 'static,
{
    const fn failed(error: SessionError<Source>) -> Self {
        Self {
            in_flight_cancelled: matches!(&error, SessionError::Cancelled),
            outcome: Err(error),
            shutdown: Shutdown::Clean,
            session_deadline: None,
            transport_open: false,
        }
    }

    fn finish(self, caller_cancelled: bool) -> Result<Output, SessionError<Source>> {
        if caller_cancelled || (self.shutdown == Shutdown::Cancelled && self.outcome.is_ok()) {
            return Err(SessionError::Cancelled);
        }
        if self.shutdown == Shutdown::Expired && self.outcome.is_ok() {
            return Err(SessionError::CleanupTimedOut);
        }
        self.outcome
    }

    fn record_shutdown(&mut self, shutdown: Shutdown) {
        if shutdown == Shutdown::Cancelled || self.shutdown == Shutdown::Clean {
            self.shutdown = shutdown;
        }
    }

    fn record_exit(&mut self, exit: Result<(), CleanupError<ExitFailure<Source>>>) {
        match exit {
            Ok(()) => {}
            Err(CleanupError::Operation(failure)) => {
                debug!("MCP context exit failed");
                if self.outcome.is_ok() && matches!(to_failure(failure), Failure::Cancelled) {
                    self.record_shutdown(Shutdown::Cancelled);
                    self.in_flight_cancelled = true;
                }
            }
            Err(CleanupError::TimedOut(_)) => {
                debug!("MCP context exit passed its deadline");
                self.record_shutdown(Shutdown::Expired);
                if self.outcome.is_ok() {
                    self.in_flight_cancelled = true;
                }
            }
            Err(CleanupError::Cancelled) => {
                debug!("MCP context exit was cancelled");
                self.record_shutdown(Shutdown::Cancelled);
            }
            Err(CleanupError::Task(error)) => {
                debug!("MCP context exit task failed");
                if self.outcome.is_ok() {
                    self.outcome = Err(SessionError::CleanupTask(error));
                }
            }
        }
    }

    // A transport failure after the caller cancelled wins only when it carries the real cause and not a recorded cleanup error
    fn apply_transport_failure(&mut self, failure: &Failure<Source>, recorded: &[Arc<Source>]) {
        if self.in_flight_cancelled {
            let Some(cause) = first_non_cancelled_cause(failure, recorded) else {
                return;
            };
            self.outcome = Err(SessionError::Runtime(cause));
            self.in_flight_cancelled = false;
            return;
        }
        if self.outcome.is_ok() && matches!(failure, Failure::Cancelled) {
            self.record_shutdown(Shutdown::Cancelled);
        }
    }
}

fn cleanup_deadline() -> Instant {
    Instant::now()
        .checked_add(SESSION_CLEANUP_TIMEOUT)
        .unwrap_or_else(Instant::now)
}

fn transport_deadline<Output, Source>(execution: &Execution<Output, Source>) -> Instant
where
    Source: Error + Send + Sync + 'static,
{
    let extended = cleanup_deadline();
    execution
        .session_deadline
        .map_or(extended, |deadline| deadline.min(extended))
}

async fn until_cancelled<Pending>(
    pending: Pending,
    cancelled: &CancellationToken,
) -> Option<Pending::Output>
where
    Pending: Future,
{
    let mut running = pin!(pending);
    let mut cancellation = pin!(cancelled.cancelled());
    poll_fn(|context| {
        if cancellation.as_mut().poll(context).is_ready() {
            return Poll::Ready(None);
        }
        running.as_mut().poll(context).map(Some)
    })
    .await
}

async fn prepare<Pending, Output, Source>(
    pending: Pending,
    cancelled: &CancellationToken,
) -> Result<Output, SessionError<Source>>
where
    Pending: Future<Output = Result<Output, Source>>,
    Source: Error + Send + Sync + 'static,
{
    match until_cancelled(pending, cancelled).await {
        Some(Ok(output)) => Ok(output),
        Some(Err(error)) => Err(SessionError::Runtime(Arc::new(error))),
        None => Err(SessionError::Cancelled),
    }
}

pub async fn execute_session_operation<Runtime, Output, Operation>(
    client: &mut SessionClient<Runtime>,
    contexts: TransportContexts<Runtime>,
    operation: Operation,
) -> Result<Output, SessionError<Runtime::Error>>
where
    Runtime: McpRuntime,
    Operation: AsyncFnOnce(
        &mut <Runtime::Session as SessionContext>::Session,
    ) -> Result<Output, SessionError<Runtime::Error>>,
{
    let TransportContexts {
        mut transport,
        http_client,
    } = contexts;
    let execution =
        run_session_layer(client, &mut transport, http_client.as_deref(), operation).await;
    let deadline = transport_deadline(&execution);
    let exit = if execution.transport_open {
        close_transport(client, transport, http_client.as_deref(), deadline).await
    } else {
        Ok(())
    };
    let recorded = http_client.map_or_else(Vec::new, |handle| handle.finish_cleanup());
    resolve_transport_exit(client, execution, exit, &recorded)
}

async fn close_transport<Runtime>(
    client: &SessionClient<Runtime>,
    transport: Runtime::Transport,
    http_client: Option<&Runtime::HttpClient>,
    deadline: Instant,
) -> Result<(), CleanupError<ExitFailure<Runtime::Error>>>
where
    Runtime: McpRuntime,
{
    if let Some(handle) = http_client {
        handle.begin_cleanup(CleanupScope {
            shield: true,
            deadline,
            caller_cancelled: client.caller_cancelled.clone(),
        });
    }
    run_bounded_cleanup(
        &client.tracker,
        &client.caller_cancelled,
        deadline,
        transport.exit(),
    )
    .await
}

fn resolve_transport_exit<Runtime, Output>(
    client: &SessionClient<Runtime>,
    mut execution: Execution<Output, Runtime::Error>,
    exit: Result<(), CleanupError<ExitFailure<Runtime::Error>>>,
    recorded: &[Arc<Runtime::Error>],
) -> Result<Output, SessionError<Runtime::Error>>
where
    Runtime: McpRuntime,
{
    match exit {
        Ok(()) => {}
        Err(CleanupError::Operation(failure)) => {
            debug!("MCP transport context exit failed");
            execution.apply_transport_failure(&to_failure(failure), recorded);
        }
        Err(CleanupError::TimedOut(_)) => {
            debug!("MCP transport context exit passed its deadline");
            execution.record_shutdown(Shutdown::Expired);
        }
        Err(CleanupError::Cancelled) => {
            debug!("MCP transport context exit was cancelled");
            execution.record_shutdown(Shutdown::Cancelled);
        }
        Err(CleanupError::Task(error)) => {
            debug!("MCP transport context exit task failed");
            if execution.outcome.is_ok() {
                execution.outcome = Err(SessionError::CleanupTask(error));
            }
        }
    }
    execution.finish(client.caller_cancelled.is_cancelled())
}
