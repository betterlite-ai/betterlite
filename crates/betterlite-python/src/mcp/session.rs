use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_mcp::client::sessions::run::run_with_session;
use betterlite_mcp::client::tools::progress::{ProgressForwarder, ProgressHost};
use betterlite_mcp::client::tools::session::ToolCallSession;
use betterlite_mcp::tools::pagination::{self, McpSession, Timeouts, ToolFormat};
use betterlite_mcp::tools::tool_calls::{self, CallRequest, OpenAiToolCall};
use serde::Serialize;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::client::ClientState;
use super::convert::{self, NativeMcpErrorPayload, session_payload, tool_failure};

mod tests;

pub(crate) type Reply = oneshot::Sender<Result<String, NativeMcpErrorPayload>>;

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
        let timeouts = Timeouts {
            client: state.timeout,
            listing: state.listing_timeout,
            listing_deadline: None,
        };
        let (ready_tx, ready_rx) = oneshot::channel();
        let (commands, mut incoming) = mpsc::channel(64);
        let stop = CancellationToken::new();
        let task_stop = stop.clone();
        let task_state = Arc::clone(state);
        let mut ready = Some(ready_tx);
        let join = state.tracker.spawn(async move {
            let mut client = task_state.session_client_with(TaskTracker::new(), task_stop.clone());
            let outcome = run_with_session(
                &mut client,
                async |session| {
                    if let Some(sender) = ready.take() {
                        let _answer = sender.send(Ok(()));
                    }
                    loop {
                        let (command, reply) = tokio::select! {
                            () = task_stop.cancelled() => return Ok(()),
                            command = incoming.recv() => match command {
                                Some(command) => command,
                                None => return Ok(()),
                            },
                        };
                        let answer = tokio::select! {
                            () = task_stop.cancelled() => return Ok(()),
                            answer = async {
                                match command {
                                    Command::ListPage(cursor) => {
                                        let work = McpSession::list_tools(session, cursor.as_deref());
                                        finish(work.await, session_payload)
                                    }
                                    Command::CallTool(request, allow_input_required) => {
                                        let forwarder = ProgressForwarder::<NoProgress> {
                                            tool: request.name.as_str(),
                                            host: None,
                                        };
                                        let work = ToolCallSession::call_tool(
                                            session,
                                            &request,
                                            &forwarder,
                                            allow_input_required,
                                        );
                                        finish(work.await, session_payload)
                                    }
                                    Command::LoadTools(tool_format, listing_deadline) => {
                                        let settings = Timeouts {
                                            client: timeouts.client,
                                            listing: timeouts.listing,
                                            listing_deadline,
                                        };
                                        let work = pagination::load_mcp_tools(session, tool_format, settings);
                                        finish(work.await, tool_failure)
                                    }
                                    Command::CallOpenAiTool(call) => {
                                        let request = tool_calls::openai_tool_call_to_mcp(&call);
                                        let work = tool_calls::call_mcp_tool(session, request, timeouts);
                                        finish(work.await, tool_failure)
                                    }
                                }
                            } => answer,
                        };
                        let _sent = reply.send(answer);
                    }
                },
                quiet_on_error,
            )
            .await;
            task_state.remember_instructions(&client);
            if let Some(sender) = ready.take() {
                let payload = match &outcome {
                    Ok(()) => NativeMcpErrorPayload::runtime(
                        "the MCP session ended before it opened".to_owned(),
                    ),
                    Err(error) => session_payload(error),
                };
                let _answer = sender.send(Err(payload));
            }
        });
        match ready_rx.await {
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
        receive(answer).await
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

fn closed_payload() -> NativeMcpErrorPayload {
    NativeMcpErrorPayload::runtime("the MCP session is closed".to_owned())
}

async fn receive(
    answer: oneshot::Receiver<Result<String, NativeMcpErrorPayload>>,
) -> Result<String, NativeMcpErrorPayload> {
    answer.await.unwrap_or_else(|_gone| Err(closed_payload()))
}

fn finish<Value, Failure>(
    outcome: Result<Value, Failure>,
    to_payload: fn(&Failure) -> NativeMcpErrorPayload,
) -> Result<String, NativeMcpErrorPayload>
where
    Value: Serialize,
{
    let value = outcome.map_err(|error| to_payload(&error))?;
    convert::encode(&value).map_err(NativeMcpErrorPayload::from)
}
