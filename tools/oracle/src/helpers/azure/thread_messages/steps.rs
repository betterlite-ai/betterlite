use std::sync::Arc;
use std::time::Duration;

use betterlite_azure::azure_assistant_definitions::client::AzureAssistantsClient;
use betterlite_azure::azure_assistant_definitions::operations::ClientRequest;
use betterlite_azure::azure_thread_messages::operations::{ThreadMessagesApi, ThreadOperation};
use betterlite_azure::endpoints::projection::SessionKind;
use betterlite_config::sections::azure_endpoints::Settings;
use betterlite_http::client_kind::ClientKind;
use betterlite_openai::threads::records::ThreadCreate;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::helper::HelperResult;

use super::fixture::{NoSources, Recorder, THREAD};
use super::reports;
use super::{Input, Step};

const DEFAULT_TIMEOUT: f64 = 600.0;
const SUPPLIED_BASE_URL: &str = "https://supplied.test/openai";
const SUPPLIED_API_VERSION: &str = "2024-01-01";
const SUPPLIED_KEY: &str = "supplied-key";
const SUPPLIED_DEADLINE: Duration = Duration::from_secs(42);

// Which facade the step exercises: a bare create, a bare retrieve, or whichever the step's fields name.
#[derive(Clone, Copy)]
enum Only {
    Create,
    Retrieve,
    Mixed,
}

pub(super) fn run(input: &Input) -> HelperResult {
    let runtime = Builder::new_current_thread()
        .build()
        .map_err(|error| reports::invalid_input(error.to_string()))?;
    let steps: Vec<Value> = match input {
        Input::ThreadCreate(steps) => steps
            .iter()
            .map(|step| thread_call(&runtime, step, Only::Create))
            .collect::<Result<_, _>>()?,
        Input::ThreadRetrieve(steps) => steps
            .iter()
            .map(|step| thread_call(&runtime, step, Only::Retrieve))
            .collect::<Result<_, _>>()?,
        Input::AssistantsBetaDefault(steps) | Input::AzureThreadAsyncFlags(steps) => steps
            .iter()
            .map(|step| thread_call(&runtime, step, Only::Mixed))
            .collect::<Result<_, _>>()?,
    };
    Ok(json!({"steps": steps}))
}

// A bare create refuses a thread id and a bare retrieve needs one; the mixed variants pick by the field the step carries.
fn thread_call(runtime: &Runtime, step: &Step, only: Only) -> HelperResult {
    if matches!(only, Only::Create) && step.thread_id.is_some() {
        return Err(reports::invalid_input(String::from(
            "a thread_create step carries a thread_id",
        )));
    }
    if matches!(only, Only::Retrieve) && step.thread_id.is_none() {
        return Err(reports::invalid_input(String::from(
            "a thread_retrieve step carries no thread_id",
        )));
    }
    let Some(thread_id) = step.thread_id.as_deref() else {
        let data = ThreadCreate {
            messages: step.messages.clone(),
            metadata: step.metadata.clone(),
        };
        return thread_step(runtime, step, ThreadOperation::Create(&data));
    };
    thread_step(runtime, step, ThreadOperation::Retrieve(thread_id))
}

fn thread_step(runtime: &Runtime, step: &Step, operation: ThreadOperation<'_>) -> HelperResult {
    let harness = Harness::new(step);
    let request = harness.request(step);
    let supplied = step
        .supplied
        .map(|supplied| supplied_client(supplied.kind, &harness.recorder));
    let outcome = runtime.block_on(harness.api().call(
        &request,
        supplied,
        flag(step.flag.as_ref()),
        operation,
    ));
    reports::operation(&harness.recorder, outcome)
}

struct Harness {
    recorder: Arc<Recorder>,
    settings: Settings,
    sources: NoSources,
}

impl Harness {
    fn new(step: &Step) -> Self {
        Self {
            recorder: Recorder::replying(step.reply.as_ref(), THREAD),
            settings: Settings::default(),
            sources: NoSources,
        }
    }

    fn api(&self) -> ThreadMessagesApi<Recorder> {
        ThreadMessagesApi {
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

// Only the literal true flag selects the async client; any other facade value stays sync.
const fn flag(value: Option<&Value>) -> Option<bool> {
    match value {
        Some(Value::Bool(state)) => Some(*state),
        Some(_) => Some(false),
        None => None,
    }
}
