use std::sync::Arc;
use std::time::Duration;

use betterlite_azure::azure_assistant_definitions::client::AzureAssistantsClient;
use betterlite_azure::azure_assistant_definitions::operations::{AssistantsApi, ClientRequest};
use betterlite_azure::azure_message_attachments::operations::{MessageCall, MessagesApi};
use betterlite_azure::endpoints::projection::SessionKind;
use betterlite_config::sections::azure_endpoints::Settings;
use betterlite_http::client_kind::ClientKind;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::helper::HelperResult;

use super::fixture::{MESSAGE, NoSources, PAGE, Wire};
use super::reports;
use super::{Input, Step};

const DEFAULT_TIMEOUT: f64 = 600.0;
const THREAD_ID: &str = "thread_1";
const SUPPLIED_BASE_URL: &str = "https://supplied.test/openai";
const SUPPLIED_API_VERSION: &str = "2023-05-01";
const SUPPLIED_KEY: &str = "supplied-key";
const SUPPLIED_DEADLINE: Duration = Duration::from_secs(42);

// Which facade the step exercises; the canned reply is the SDK body the case did not override.
#[derive(Clone, Copy)]
enum Requested {
    Append,
    List,
}

struct Harness {
    wire: Arc<Wire>,
    settings: Settings,
    sources: NoSources,
}

impl Harness {
    fn new(step: &Step, canned: &str) -> Self {
        Self {
            wire: Wire::replying(step.reply.as_ref(), canned),
            settings: Settings::default(),
            sources: NoSources,
        }
    }

    fn api(&self) -> MessagesApi<Wire> {
        MessagesApi {
            assistants: AssistantsApi {
                transport: Arc::clone(&self.wire),
            },
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

pub(super) fn run(input: &Input) -> HelperResult {
    let runtime = Builder::new_current_thread()
        .build()
        .map_err(|error| reports::invalid_input(error.to_string()))?;
    let steps: Vec<Value> = match input {
        Input::AppendMessage(steps) => steps
            .iter()
            .map(|step| message_step(&runtime, step, Requested::Append))
            .collect::<Result<_, _>>()?,
        Input::ListMessages(steps) => steps
            .iter()
            .map(|step| message_step(&runtime, step, Requested::List))
            .collect::<Result<_, _>>()?,
        Input::MapCreateMessageParams(steps) => steps
            .iter()
            .map(reports::mapped)
            .collect::<Result<_, _>>()?,
    };
    Ok(json!({"steps": steps}))
}

// Every message step shares one shape: build the wire and its flat settings, call the facade, report.
fn message_step(runtime: &Runtime, step: &Step, requested: Requested) -> HelperResult {
    let thread_id = step.thread_id.as_deref().unwrap_or(THREAD_ID);
    let (canned, call) = match requested {
        Requested::Append => {
            let Some(data) = step.data.as_ref() else {
                return Err(reports::invalid_input(String::from(
                    "an append_message step carries no data",
                )));
            };
            (MESSAGE, MessageCall::Append { thread_id, data })
        }
        Requested::List => (PAGE, MessageCall::List(thread_id)),
    };
    let harness = Harness::new(step, canned);
    let request = harness.request(step);
    let supplied = step
        .supplied
        .as_ref()
        .map(|supplied| supplied_client(supplied.kind, &harness.wire));
    let outcome = runtime.block_on(harness.api().call(
        &request,
        supplied,
        flag(step.flag.as_ref()),
        call,
    ));
    reports::operation(&harness.wire, outcome)
}

fn supplied_client(kind: ClientKind, wire: &Arc<Wire>) -> AzureAssistantsClient<'static, Wire> {
    AzureAssistantsClient {
        kind,
        inputs: None,
        transport: Arc::clone(wire),
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
