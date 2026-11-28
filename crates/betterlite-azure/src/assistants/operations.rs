use std::sync::Arc;
use std::time::Duration;

use betterlite_http::client_kind::ClientKind;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::thread_runs::error::ThreadRunsError;
use betterlite_openai::thread_runs::records::{RunReply, RunRequest};
use betterlite_openai::thread_runs::stream::{RunEventHandler, RunStream, RunStreamTransport};
use http::HeaderMap;
use http::Method;
use tokio::time::sleep;

use crate::assistants::error::AssistantsRunError;
use crate::assistants::request;
use crate::azure_assistant_definitions::client::AzureAssistantsClient;
use crate::azure_assistant_definitions::operations::{AssistantsApi, ClientRequest};

const DEFAULT_POLL: Duration = Duration::from_secs(1);
const STREAM_DEADLINE: Duration = Duration::from_mins(100);
const TERMINAL_STATES: [&str; 6] = [
    "requires_action",
    "cancelled",
    "completed",
    "failed",
    "expired",
    "incomplete",
];

pub struct AssistantsRunsApi<T> {
    pub transport: Arc<T>,
}

// The facade switches LiteLLM tests with `is True`, so only an explicit true selects async or streaming.
#[derive(Debug, Clone, Copy, Default)]
pub struct RunDispatch {
    pub arun_thread: Option<bool>,
    pub stream: Option<bool>,
}

pub struct RunCall<'call> {
    pub dispatch: RunDispatch,
    pub run: &'call RunRequest,
    pub handler: Option<Arc<dyn RunEventHandler>>,
}

// The buffered and streaming results stay distinct, and each names the SDK client that serves it.
pub enum RunInvocation<T> {
    Polled(ClientKind, Box<RunReply>),
    Streaming(Box<RunStream<T>>),
}

impl<T> AssistantsRunsApi<T>
where
    T: AssistantsTransport + RunStreamTransport,
{
    // The SDK polls right after the create, so the create's own status never ends the wait; each request carries the caller's timeout.
    pub async fn poll(
        &self,
        client: &AzureAssistantsClient<'_, T>,
        run: &RunRequest,
    ) -> Result<RunReply, AssistantsRunError> {
        let created = request::fetch(client, create_request(client, run, false)?).await?;
        let mut delay: Option<Duration> = None;
        loop {
            let poll = poll_request(client, run, &created.reply.id)?;
            let fetched = request::fetch(client, poll).await?;
            if TERMINAL_STATES.contains(&fetched.reply.status.as_str()) {
                return Ok(fetched.reply);
            }
            let wait = match delay {
                Some(wait) => wait,
                None => next_poll(&fetched.headers)?,
            };
            delay = Some(wait);
            sleep(wait).await;
        }
    }

    // The manager is built, not opened: the request goes out when the caller starts reading the stream.
    pub fn stream(
        &self,
        client: &AzureAssistantsClient<'_, T>,
        run: &RunRequest,
        handler: Option<Arc<dyn RunEventHandler>>,
    ) -> Result<RunStream<T>, AssistantsRunError> {
        Ok(RunStream {
            kind: client.kind,
            handler,
            transport: Arc::clone(&client.transport),
            request: create_request(client, run, true)?,
            read_timeout: client.deadline,
        })
    }

    // Serves the run facade: the literal-true switches pick the client kind and the streaming helper.
    pub async fn run<'data>(
        &self,
        request: &ClientRequest<'data>,
        supplied: Option<AzureAssistantsClient<'data, T>>,
        call: RunCall<'_>,
    ) -> Result<RunInvocation<T>, AssistantsRunError> {
        let api = AssistantsApi {
            transport: Arc::clone(&self.transport),
        };
        let client = api.azure_client(request, supplied, client_kind(call.dispatch.arun_thread))?;
        if call.dispatch.stream == Some(true) {
            let stream = self.stream(&client, call.run, call.handler)?;
            return Ok(RunInvocation::Streaming(Box::new(stream)));
        }
        let reply = self.poll(&client, call.run).await?;
        Ok(RunInvocation::Polled(client.kind, Box::new(reply)))
    }
}

const fn client_kind(arun_thread: Option<bool>) -> ClientKind {
    match arun_thread {
        Some(true) => ClientKind::Asynchronous,
        Some(false) | None => ClientKind::Sync,
    }
}

fn create_request<T>(
    client: &AzureAssistantsClient<'_, T>,
    run: &RunRequest,
    stream: bool,
) -> Result<OutboundRequest, AssistantsRunError> {
    let deadline = if stream {
        STREAM_DEADLINE
    } else {
        client.deadline
    };
    let body = request::create_body(run, stream)?;
    let tail = request::run_tail(&run.thread_id)?;
    request::outbound(
        client,
        Method::POST,
        &tail,
        RequestBody::Json(body),
        deadline,
    )
}

fn poll_request<T>(
    client: &AzureAssistantsClient<'_, T>,
    run: &RunRequest,
    run_id: &str,
) -> Result<OutboundRequest, AssistantsRunError> {
    let tail = request::poll_tail(&run.thread_id, run_id)?;
    request::outbound(
        client,
        Method::GET,
        &tail,
        RequestBody::Empty,
        client.deadline,
    )
}

// The SDK reads openai-poll-after-ms once, from the first reply that is not yet terminal, and then reuses it.
fn next_poll(headers: &HeaderMap) -> Result<Duration, AssistantsRunError> {
    let Some(value) = headers.get("openai-poll-after-ms") else {
        return Ok(DEFAULT_POLL);
    };
    let text = value
        .to_str()
        .map_err(|error| AssistantsRunError::Runs(ThreadRunsError::PollAfter(error.to_string())))?;
    let millis = text.trim().parse::<u64>().map_err(|_invalid| {
        AssistantsRunError::Runs(ThreadRunsError::PollAfter(text.to_owned()))
    })?;
    Ok(Duration::from_millis(millis))
}
