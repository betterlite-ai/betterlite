use std::ops::AsyncFnOnce;
use std::sync::PoisonError;

use crate::client::cleanup::bounded::run_bounded_cleanup;
use crate::client::cleanup::http_client::CleanupScope;

use super::super::client::SessionClient;
use super::super::error::SessionError;
use super::super::negotiate::{InitializeOutcome as _, initialize_session};
use super::super::runtime::{HttpClientHandle as _, McpRuntime, SessionContext, SessionOptions};
use super::super::stream::MessageHandler;
use super::super::transport::TransportContext as _;
use super::{Execution, Shutdown, cleanup_deadline, prepare, until_cancelled};

pub(super) async fn run_session_layer<Runtime, Output, Operation>(
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
    let streams = match prepare(transport.enter(), &client.caller_cancelled).await {
        Ok(streams) => streams,
        Err(error) => return Execution::failed(error),
    };
    let handler = MessageHandler::default();
    let mut context = client.runtime.session_context(
        streams,
        SessionOptions {
            read_timeout: client.auth.timeout,
            callbacks: client.settings.callbacks.clone(),
            message_handler: handler.clone(),
        },
    );
    let mut session = match prepare(context.enter(), &client.caller_cancelled).await {
        Ok(session) => session,
        Err(error) => {
            let mut execution = Execution::failed(error);
            execution.transport_open = true;
            return execution;
        }
    };
    let outcome = run_operation(client, &mut session, &handler, operation).await;
    drop(session);
    close_session(client, context, outcome, http_client).await
}

async fn run_operation<Runtime, Output, Operation>(
    client: &mut SessionClient<Runtime>,
    session: &mut <Runtime::Session as SessionContext>::Session,
    handler: &MessageHandler<Runtime::Error>,
    operation: Operation,
) -> Result<Output, SessionError<Runtime::Error>>
where
    Runtime: McpRuntime,
    Operation: AsyncFnOnce(
        &mut <Runtime::Session as SessionContext>::Session,
    ) -> Result<Output, SessionError<Runtime::Error>>,
{
    let cancellation = client.caller_cancelled.clone();
    let result = {
        let run = async {
            let result = initialize_session(session, &client.settings).await?;
            client.last_initialize_instructions = result
                .instructions()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_owned);
            operation(session).await
        };
        until_cancelled(run, &cancellation)
            .await
            .unwrap_or_else(|| Err(SessionError::Cancelled))
    };
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
        handle.begin_cleanup(CleanupScope {
            shield: true,
            deadline,
            caller_cancelled: client.caller_cancelled.clone(),
        });
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
        transport_open: true,
    };
    execution.record_exit(exit);
    execution
}
