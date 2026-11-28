use std::mem;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use betterlite_azure::assistants::error::AssistantsRunError;
use betterlite_azure::assistants::operations::{
    AssistantsRunsApi, RunCall, RunDispatch, RunInvocation,
};
use betterlite_azure::azure_assistant_definitions::client::AzureAssistantsClient;
use betterlite_azure::azure_assistant_definitions::operations::ClientRequest;
use betterlite_azure::endpoints::projection::SessionKind;
use betterlite_config::sections::azure_endpoints::Settings;
use betterlite_http::client_kind::ClientKind;
use betterlite_openai::thread_runs::records::RunReply;
use betterlite_openai::thread_runs::stream::{RunEventHandler, RunStream};
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::helper::HelperResult;

use super::fixture::{NoSources, Recorder, RecordingHandler};
use super::reports;
use super::{Input, Step};

const DEFAULT_TIMEOUT: f64 = 600.0;
const SUPPLIED_BASE_URL: &str = "https://supplied.test/openai";
const SUPPLIED_API_VERSION: &str = "2024-01-01";
const SUPPLIED_KEY: &str = "supplied-key";
const SUPPLIED_DEADLINE: Duration = Duration::from_secs(42);

pub(super) fn run(input: &Input) -> HelperResult {
    let runtime = Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(|error| reports::invalid_input(error.to_string()))?;
    let steps = [
        input.run_poll.as_deref(),
        input.run_stream.as_deref(),
        input.run_instructions_tools.as_deref(),
        input.run_failure.as_deref(),
        input.run_dispatch.as_deref(),
        input.instructions_tools_overrides.as_deref(),
        input.stream_event_handler.as_deref(),
        input.assistants_beta_default.as_deref(),
    ]
    .into_iter()
    .flatten()
    .next()
    .ok_or_else(|| reports::invalid_input("expected one azure assistants variant".to_owned()))?;
    let outcomes = steps
        .iter()
        .map(|step| one(&runtime, step))
        .collect::<Result<Vec<Value>, _>>()?;
    Ok(json!({"steps": outcomes}))
}

// The run facade, called with the keywords LiteLLM's run_thread reads, and the run the case names.
fn one(runtime: &Runtime, step: &Step) -> HelperResult {
    let harness = Harness::new(step);
    let request = harness.request(step);
    let run = step.run.request();
    let recording = step.handler.unwrap_or(false).then(RecordingHandler::start);
    let dispatch = RunDispatch {
        arun_thread: flag(step.arun_thread.as_ref()),
        stream: flag(step.stream.as_ref()),
    };
    let supplied = step
        .supplied
        .map(|chosen| supplied_client(chosen.kind, &harness.recorder));
    let attached = recording
        .as_ref()
        .map(|recorded| -> Arc<dyn RunEventHandler> { Arc::<RecordingHandler>::clone(recorded) });
    match runtime.block_on(harness.runs().run(
        &request,
        supplied,
        RunCall {
            dispatch,
            run: &run,
            handler: attached,
        },
    )) {
        Ok(RunInvocation::Polled(kind, reply)) => Ok(polled(&harness, kind, &reply)),
        Ok(RunInvocation::Streaming(manager)) => {
            consumed(runtime, &harness, *manager, recording.as_ref())
        }
        Err(error) => failed(&error, &harness),
    }
}

fn polled(harness: &Harness, kind: ClientKind, reply: &RunReply) -> Value {
    json!({
        "requests": mem::take(&mut *harness.recorder.seen.lock().unwrap_or_else(PoisonError::into_inner)),
        "calls": Vec::<String>::new(),
        "kind": kind.label(),
        "result": reply,
    })
}

fn failed(error: &AssistantsRunError, harness: &Harness) -> HelperResult {
    Ok(json!({
        "requests": mem::take(&mut *harness.recorder.seen.lock().unwrap_or_else(PoisonError::into_inner)),
        "calls": Vec::<String>::new(),
        "error": reports::failure(error)?,
    }))
}

// The manager is returned unconsumed, so the case's requests start when it does.
fn consumed(
    runtime: &Runtime,
    harness: &Harness,
    manager: RunStream<Recorder>,
    recording: Option<&Arc<RecordingHandler>>,
) -> HelperResult {
    let outcome = runtime.block_on(manager.consume());
    let calls = recording.map_or_else(Vec::new, |handler| handler.calls());
    let requests = mem::take(
        &mut *harness
            .recorder
            .seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
    );
    match outcome {
        Ok(events) => Ok(json!({"requests": requests, "calls": calls, "events": events})),
        Err(error) => Ok(json!({
            "requests": requests,
            "calls": calls,
            "error": reports::failure(&AssistantsRunError::Runs(error))?,
        })),
    }
}

struct Harness {
    recorder: Arc<Recorder>,
    settings: Settings,
    sources: NoSources,
}

impl Harness {
    fn new(step: &Step) -> Self {
        Self {
            recorder: Recorder::start(&step.replies),
            settings: Settings::default(),
            sources: NoSources,
        }
    }

    fn runs(&self) -> AssistantsRunsApi<Recorder> {
        AssistantsRunsApi {
            transport: Arc::clone(&self.recorder),
        }
    }

    fn request<'data>(&'data self, step: &'data Step) -> ClientRequest<'data> {
        ClientRequest {
            params: step.params.clone(),
            provider: None,
            api_key: step.api_key.clone(),
            api_base: step.api_base.clone(),
            api_version: step.api_version.clone(),
            azure_ad_token: step.azure_ad_token.clone(),
            timeout: step.timeout.unwrap_or(DEFAULT_TIMEOUT),
            max_retries: step.max_retries,
            refresh_enabled: false,
            session: SessionKind::Dedicated,
            settings: &self.settings,
            environment: &step.environment,
            sources: &self.sources,
        }
    }
}

// A client the caller supplied, matching the one the Python adapter builds for the same step.
fn supplied_client(
    kind: ClientKind,
    recorder: &Arc<Recorder>,
) -> AzureAssistantsClient<'static, Recorder> {
    AzureAssistantsClient {
        kind,
        inputs: None,
        transport: Arc::clone(recorder),
        base_url: String::from(SUPPLIED_BASE_URL),
        api_version: String::from(SUPPLIED_API_VERSION),
        key: Some(SecretString::from(SUPPLIED_KEY)),
        token: None,
        deadline: SUPPLIED_DEADLINE,
    }
}

// LiteLLM tests `is True`, so an absent, false or non-boolean truthy switch takes the false branch.
const fn flag(value: Option<&Value>) -> Option<bool> {
    match value {
        Some(Value::Bool(state)) => Some(*state),
        Some(_) => Some(false),
        None => None,
    }
}
