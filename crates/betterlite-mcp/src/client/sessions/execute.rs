use std::error::Error;
use std::future::{Future, poll_fn};
use std::ops::AsyncFnOnce;
use std::pin::pin;
use std::sync::{Arc, PoisonError};
use std::task::Poll;
use std::time::Duration;

use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::client::cleanup::bounded::{CleanupError, run_bounded_cleanup};
use crate::client::cleanup::failure::{Failure, first_non_cancelled_cause};

use super::client::SessionClient;
use super::error::{ExitFailure, SessionError, to_failure};
use super::negotiate::{InitializeOutcome as _, initialize_session};
use super::runtime::{HttpClientHandle as _, McpRuntime, SessionContext};
use super::stream::MessageHandler;
use super::transport::{TransportContext as _, TransportContexts};

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
}

impl<Output, Source> Execution<Output, Source>
where
    Source: Error + Send + Sync + 'static,
{
    const fn cancelled_early() -> Self {
        Self {
            outcome: Err(SessionError::Cancelled),
            in_flight_cancelled: true,
            shutdown: Shutdown::Clean,
            session_deadline: None,
        }
    }

    const fn failed_early(error: SessionError<Source>) -> Self {
        Self {
            outcome: Err(error),
            in_flight_cancelled: false,
            shutdown: Shutdown::Clean,
            session_deadline: None,
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

    fn record_exit(&mut self, exit: Result<(), CleanupError<ExitFailure<Source>>>) {
        match exit {
            Ok(()) => {}
            Err(CleanupError::Operation(failure)) => {
                debug!("MCP context exit failed");
                if self.outcome.is_ok() && matches!(to_failure(failure), Failure::Cancelled) {
                    self.record_shutdown(Shutdown::Cancelled);
                }
            }
            Err(CleanupError::TimedOut(_)) => {
                debug!("MCP context exit passed its deadline");
                self.record_shutdown(Shutdown::Expired);
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

async fn open_transport<Runtime>(
    client: &SessionClient<Runtime>,
    transport: &mut Runtime::Transport,
) -> Option<Result<Runtime::Streams, SessionError<Runtime::Error>>>
where
    Runtime: McpRuntime,
{
    until_cancelled(transport.enter(), &client.caller_cancelled)
        .await
        .map(|result| result.map_err(|error| SessionError::Runtime(Arc::new(error))))
}

async fn open_session<Runtime>(
    client: &SessionClient<Runtime>,
    context: &mut Runtime::Session,
) -> Option<Result<<Runtime::Session as SessionContext>::Session, SessionError<Runtime::Error>>>
where
    Runtime: McpRuntime,
{
    until_cancelled(context.enter(), &client.caller_cancelled)
        .await
        .map(|result| result.map_err(|error| SessionError::Runtime(Arc::new(error))))
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
    let exit = close_transport(client, transport, http_client.as_deref(), deadline).await;
    let recorded = http_client.map_or_else(Vec::new, |handle| handle.finish_cleanup());
    resolve_transport_exit(client, execution, exit, &recorded)
}

async fn run_session_layer<Runtime, Output, Operation>(
    client: &mut SessionClient<Runtime>,
    transport: &mut Runtime::Transport,
    http_client: Option<&Runtime::HttpClient>,
    operation: Operation,
) -> Execution<Output, Runtime::Error>
where
    Runtime: McpRuntime,
    Operation: AsyncFnOnce(
        &mut <Runtime::Session as SessionContext>::Session,
    ) -> Result<Output, SessionError<Runtime::Error>>,
{
    let streams = match open_transport(client, transport).await {
        None => return Execution::cancelled_early(),
        Some(Err(error)) => return Execution::failed_early(error),
        Some(Ok(streams)) => streams,
    };
    let handler = MessageHandler::default();
    let mut context = client
        .runtime
        .session_context(streams, client.session_options(handler.clone()));
    let mut session = match open_session(client, &mut context).await {
        None => return Execution::cancelled_early(),
        Some(Err(error)) => return Execution::failed_early(error),
        Some(Ok(session)) => session,
    };
    let outcome = run_operation(client, &mut session, &handler, operation).await;
    drop(session);
    close_session(client, context, outcome, http_client).await
}

async fn run_operation<Runtime, Output, Operation>(
    client: &mut SessionClient<Runtime>,
    session: &mut <Runtime::Session as SessionContext>::Session,
    handler: &MessageHandler,
    operation: Operation,
) -> Result<Output, SessionError<Runtime::Error>>
where
    Runtime: McpRuntime,
    Operation: AsyncFnOnce(
        &mut <Runtime::Session as SessionContext>::Session,
    ) -> Result<Output, SessionError<Runtime::Error>>,
{
    let cancellation = client.caller_cancelled.clone();
    let result = until_cancelled(
        initialize_and_operate(client, session, operation),
        &cancellation,
    )
    .await
    .unwrap_or_else(|| Err(SessionError::Cancelled));
    match result {
        Ok(output) => Ok(output),
        Err(SessionError::Protocol(failure)) => {
            let recorded = handler
                .recorded
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            recorded.map_or_else(
                || Err(SessionError::Protocol(failure)),
                |stream| Err(SessionError::Stream(stream)),
            )
        }
        Err(error) => Err(error),
    }
}

async fn initialize_and_operate<Runtime, Output, Operation>(
    client: &mut SessionClient<Runtime>,
    session: &mut <Runtime::Session as SessionContext>::Session,
    operation: Operation,
) -> Result<Output, SessionError<Runtime::Error>>
where
    Runtime: McpRuntime,
    Operation: AsyncFnOnce(
        &mut <Runtime::Session as SessionContext>::Session,
    ) -> Result<Output, SessionError<Runtime::Error>>,
{
    let result = initialize_session(session, &client.settings).await?;
    client.last_initialize_instructions = result
        .instructions()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned);
    operation(session).await
}

async fn close_session<Runtime, Output>(
    client: &SessionClient<Runtime>,
    context: Runtime::Session,
    outcome: Result<Output, SessionError<Runtime::Error>>,
    http_client: Option<&Runtime::HttpClient>,
) -> Execution<Output, Runtime::Error>
where
    Runtime: McpRuntime,
{
    let deadline = cleanup_deadline();
    if let Some(handle) = http_client {
        handle.begin_cleanup(client.cleanup_scope(deadline));
    }
    let exit = run_bounded_cleanup(
        &client.tracker,
        &client.caller_cancelled,
        deadline,
        context.exit(),
    )
    .await;
    let mut execution = Execution {
        in_flight_cancelled: matches!(&outcome, Err(SessionError::Cancelled)),
        outcome,
        shutdown: Shutdown::Clean,
        session_deadline: Some(deadline),
    };
    execution.record_exit(exit);
    execution
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
        handle.begin_cleanup(client.cleanup_scope(deadline));
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
