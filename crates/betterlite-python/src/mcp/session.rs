use super::client::ClientState;
use super::convert::{self, McpErrorKind, NativeMcpErrorPayload, session_payload, tool_failure};
use betterlite_mcp::client::sdk_runtime::session::SdkSession;
use betterlite_mcp::client::sdk_transport::error::SdkError;
use betterlite_mcp::client::sessions::error::SessionError;
use betterlite_mcp::client::sessions::run::run_with_session;
use betterlite_mcp::client::tools::progress::{ProgressForwarder, ProgressHost};
use betterlite_mcp::client::tools::session::ToolCallSession;
use betterlite_mcp::tools::pagination::{self, McpSession, Timeouts, ToolFormat};
use betterlite_mcp::tools::tool_calls::{self, CallRequest, OpenAiToolCall};
use serde::Serialize;
use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
mod tests;
pub(crate) type Reply = oneshot::Sender<Result<String, NativeMcpErrorPayload>>;
// The one thing open() waits on: the session started, or the payload that stopped it
type Startup = Result<(), NativeMcpErrorPayload>;
// LiteLLM's call_tool takes no progress callback, so every notification the server sends is dropped
pub(crate) struct NoProgress;
impl ProgressHost for NoProgress {
    type Error = Infallible;

    fn report(
        &self,
        _progress: f64,
        _total: Option<f64>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        ready(Ok(()))
    }
}
pub(crate) enum Command {
    ListPage(Option<String>),
    CallTool(CallRequest, bool),
    LoadTools(ToolFormat, Option<Duration>),
    CallOpenAiTool(OpenAiToolCall),
}
// One open MCP session, served by a single task that owns the transport until close or drop
pub(crate) struct OpenSession {
    commands: mpsc::Sender<(Command, Reply)>,
    stop: CancellationToken,
    closed: Mutex<bool>,
    join: Mutex<Option<JoinHandle<()>>>,
}
impl OpenSession {
    pub(crate) async fn open(
        state: &Arc<ClientState>,
        quiet_on_error: bool,
    ) -> Result<Self, NativeMcpErrorPayload> {
        let (ready, startup) = oneshot::channel();
        let (commands, incoming) = mpsc::channel(64);
        let stop = CancellationToken::new();
        let join = state.tracker.spawn(drive(
            Arc::clone(state),
            incoming,
            stop.clone(),
            ready,
            quiet_on_error,
        ));
        match startup.await {
            Ok(Ok(())) => Ok(Self {
                commands,
                stop,
                closed: Mutex::new(false),
                join: Mutex::new(Some(join)),
            }),
            Ok(Err(payload)) => Err(payload),
            Err(_gone) => Err(closed_payload()),
        }
    }

    pub(crate) async fn exchange(&self, command: Command) -> Result<String, NativeMcpErrorPayload> {
        self.ensure_open()?;
        let (reply, answer) = oneshot::channel();
        self.commands
            .send((command, reply))
            .await
            .map_err(|_gone| closed_payload())?;
        answer.await.unwrap_or_else(|_gone| Err(closed_payload()))
    }

    pub(crate) async fn close(&self) {
        *self.closed.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.stop.cancel();
        let join = self
            .join
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(handle) = join {
            let _joined = handle.await;
        }
    }

    fn ensure_open(&self) -> Result<(), NativeMcpErrorPayload> {
        let closed = *self.closed.lock().unwrap_or_else(PoisonError::into_inner);
        if closed {
            return Err(closed_payload());
        }
        Ok(())
    }
}
impl Drop for OpenSession {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
// The session task: B4744 opens and closes the session, and every command is answered on its own oneshot
async fn drive(
    state: Arc<ClientState>,
    mut incoming: mpsc::Receiver<(Command, Reply)>,
    stop: CancellationToken,
    ready_tx: oneshot::Sender<Startup>,
    quiet_on_error: bool,
) {
    let timeouts = Timeouts {
        client: state.timeout,
        listing: state.listing_timeout,
        listing_deadline: None,
    };
    let mut client = state.session_client_with(TaskTracker::new(), stop.clone());
    let mut ready = Some(ready_tx);
    let outcome = run_with_session(
        &mut client,
        async |session| {
            announce(&mut ready);
            serve(session, &mut incoming, &stop, timeouts).await
        },
        quiet_on_error,
    )
    .await;
    state.remember_instructions(&client);
    let payload = startup_payload(&outcome);
    if let Some(sender) = ready.take() {
        let _answer = sender.send(Err(payload));
    }
}
// open_session answers as soon as initialize and the handshake are through, not after the first command
fn announce(ready: &mut Option<oneshot::Sender<Startup>>) {
    if let Some(sender) = ready.take() {
        let _answer = sender.send(Ok(()));
    }
}
async fn serve(
    session: &mut SdkSession,
    incoming: &mut mpsc::Receiver<(Command, Reply)>,
    stop: &CancellationToken,
    timeouts: Timeouts,
) -> Result<(), SessionError<SdkError>> {
    loop {
        let next = tokio::select! {
            () = stop.cancelled() => return Ok(()),
            next = incoming.recv() => next,
        };
        let Some((command, reply)) = next else {
            return Ok(());
        };
        let answer = tokio::select! {
            () = stop.cancelled() => return Ok(()),
            answer = answer_command(session, command, timeouts) => answer,
        };
        let _sent = reply.send(answer);
    }
}
async fn answer_command(
    session: &mut SdkSession,
    command: Command,
    timeouts: Timeouts,
) -> Result<String, NativeMcpErrorPayload> {
    match command {
        Command::ListPage(cursor) => {
            let page = McpSession::list_tools(session, cursor.as_deref());
            finish(page.await, session_payload)
        }
        Command::CallTool(request, allow_input_required) => {
            let forwarder = ProgressForwarder::<NoProgress> {
                tool: request.name.as_str(),
                host: None,
            };
            let called =
                ToolCallSession::call_tool(session, &request, &forwarder, allow_input_required);
            finish(called.await, session_payload)
        }
        Command::LoadTools(tool_format, listing_deadline) => {
            let settings = Timeouts {
                client: timeouts.client,
                listing: timeouts.listing,
                listing_deadline,
            };
            let loaded = pagination::load_mcp_tools(session, tool_format, settings);
            finish(loaded.await, tool_failure)
        }
        Command::CallOpenAiTool(call) => {
            let request = tool_calls::openai_tool_call_to_mcp(&call);
            let converted = tool_calls::call_mcp_tool(session, request, timeouts);
            finish(converted.await, tool_failure)
        }
    }
}
fn startup_payload(outcome: &Result<(), SessionError<SdkError>>) -> NativeMcpErrorPayload {
    match outcome {
        Ok(()) => NativeMcpErrorPayload::plain(
            McpErrorKind::RuntimeError,
            "the MCP session ended before it opened".to_owned(),
        ),
        Err(error) => session_payload(error),
    }
}
fn closed_payload() -> NativeMcpErrorPayload {
    let message = "the MCP session is closed".to_owned();
    let kind = McpErrorKind::RuntimeError;
    NativeMcpErrorPayload::plain(kind, message)
}
fn finish<Value, Failure>(
    outcome: Result<Value, Failure>,
    to_payload: fn(&Failure) -> NativeMcpErrorPayload,
) -> Result<String, NativeMcpErrorPayload>
where
    Value: Serialize,
{
    let value = outcome.map_err(|error| to_payload(&error))?;
    let text = convert::encode(&value)?;
    Ok(text)
}
