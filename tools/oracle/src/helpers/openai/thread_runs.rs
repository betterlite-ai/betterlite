use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_openai::assistants::client::{ClientKind, ClientOptions};
use betterlite_openai::thread_runs::error::ThreadRunsError;
use betterlite_openai::thread_runs::operations::{RunCall, RunDispatch, RunInvocation, RunsApi};
use betterlite_openai::thread_runs::records::RunRequest;
use betterlite_openai::thread_runs::stream::{RunEventHandler, RunStream};
use input::{Fields, Round, ThreadRunsCall};
use secrecy::SecretString;
use serde_json::{Value, json};
use wire::{RecordingHandler, Wire};

pub(super) fn invalid_input(message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
fn client_options(fields: &Fields) -> Result<ClientOptions, Box<crate::helpers::HelperError>> {
    let options = &fields.options;
    let timeout = options
        .timeout
        .map(Duration::try_from_secs_f64)
        .transpose()
        .map_err(|error| invalid_input(error.to_string()))?;
    Ok(ClientOptions {
        api_key: options.api_key.clone().map(SecretString::from),
        api_base: options.api_base.clone(),
        timeout,
        max_retries: options.max_retries,
        organization: options.organization.clone(),
    })
}
const fn selected(kind: ClientKind, streaming: bool) -> &'static str {
    match (kind, streaming) {
        (ClientKind::Sync, false) => "sync_run",
        (ClientKind::Async, false) => "async_run",
        (ClientKind::Sync, true) => "sync_stream",
        (ClientKind::Async, true) => "async_stream",
    }
}
fn failure(error: &ThreadRunsError) -> Result<Value, Box<crate::helpers::HelperError>> {
    match error {
        ThreadRunsError::Status(status, _) => Ok(json!({"kind": "status", "status": status})),
        ThreadRunsError::MissingApiKey => Ok(json!({"kind": "configuration"})),
        ThreadRunsError::EmptyThreadId
        | ThreadRunsError::EmptyRunId
        | ThreadRunsError::DotSegment(_) => Ok(json!({"kind": "invalid_thread_id"})),
        ThreadRunsError::Shape(_) => Ok(json!({"kind": "shape"})),
        ThreadRunsError::Malformed(_) | ThreadRunsError::Truncated(_) => {
            Ok(json!({"kind": "malformed"}))
        }
        ThreadRunsError::ProviderError { body } => {
            let value = serde_json::from_slice::<Value>(body).unwrap_or(Value::Null);
            Ok(
                json!({"kind": "stream_error", "type": value.get("type"), "code": value.get("code")}),
            )
        }
        ThreadRunsError::BaseUrl(_)
        | ThreadRunsError::Header(_)
        | ThreadRunsError::Encode(_)
        | ThreadRunsError::Transport(_)
        | ThreadRunsError::PollAfterHeader(_)
        | ThreadRunsError::PollAfter { .. }
        | ThreadRunsError::StreamTooLarge
        | ThreadRunsError::Timeout => Err(invalid_input(error.to_string())),
    }
}
async fn stream_outcome(
    wire: &Wire,
    stream: RunStream<Wire>,
    calls: Arc<Mutex<Vec<String>>>,
    recorded: bool,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let kind = selected(stream.kind, true);
    let events = stream.consume().await;
    let logged = if recorded {
        calls.lock().unwrap_or_else(PoisonError::into_inner).clone()
    } else {
        Vec::new()
    };
    match events {
        Ok(decoded) => Ok(json!({
            "selected": kind,
            "requests": wire.requests(),
            "events": decoded,
            "calls": logged,
        })),
        Err(error) => Ok(json!({
            "selected": kind,
            "requests": wire.requests(),
            "calls": logged,
            "error": failure(&error)?,
        })),
    }
}
async fn round_outcome(
    fields: &Fields,
    round: &Round,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let wire = Wire::new(&round.replies)?;
    let api = RunsApi {
        transport: Arc::clone(&wire),
    };
    let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let handler: Option<Arc<dyn RunEventHandler>> =
        round.handler.then(|| -> Arc<dyn RunEventHandler> {
            Arc::new(RecordingHandler {
                calls: Arc::clone(&calls),
            })
        });
    let call = RunCall {
        options: client_options(fields)?,
        client: None,
        // LiteLLM tests `is True`, so an absent, false or non-boolean truthy switch takes the false branch.
        dispatch: RunDispatch {
            arun_thread: matches!(round.arun_thread.as_ref(), Some(Value::Bool(true))),
            stream: matches!(round.stream.as_ref(), Some(Value::Bool(true))),
        },
        handler,
    };
    let request: &RunRequest = &round.request;
    match api.run_thread(call, request).await {
        Ok(RunInvocation::Polled { kind, run }) => Ok(json!({
            "selected": selected(kind, false),
            "requests": wire.requests(),
            "result": run,
        })),
        Ok(RunInvocation::Streaming(stream)) => {
            stream_outcome(&wire, *stream, calls, round.handler).await
        }
        Err(error) => Ok(json!({
            "requests": wire.requests(),
            "error": failure(&error)?,
        })),
    }
}
async fn rounds(fields: &Fields) -> Result<Value, Box<crate::helpers::HelperError>> {
    let mut outcomes = Vec::new();
    for round in &fields.rounds {
        outcomes.push(round_outcome(fields, round).await?);
    }
    Ok(json!({ "rounds": outcomes }))
}
fn invoke(call: &ThreadRunsCall) -> crate::helpers::HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| invalid_input(error.to_string()))?;
    let fields = match call {
        ThreadRunsCall::RunPollCompletion(fields)
        | ThreadRunsCall::RequiresAction(fields)
        | ThreadRunsCall::FailedRun(fields)
        | ThreadRunsCall::StreamIncrementalEvents(fields)
        | ThreadRunsCall::InstructionsToolsOverrides(fields)
        | ThreadRunsCall::RunDispatch(fields)
        | ThreadRunsCall::StreamEventHandler(fields)
        | ThreadRunsCall::AssistantsBetaDefault(fields) => fields,
    };
    runtime.block_on(rounds(fields))
}
pub(crate) struct Op;
impl crate::helpers::Helper for Op {
    type Input = ThreadRunsCall;
    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        invoke(input)
    }
}
mod input;
mod tests;
mod wire;
