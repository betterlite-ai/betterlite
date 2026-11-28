use std::sync::Arc;
use std::time::Duration;

use betterlite_azure::azure_assistant_definitions::client::AzureAssistantsClient;
use betterlite_azure::azure_assistant_definitions::operations::{
    AssistantsApi, ClientRequest, DefinitionCall,
};
use betterlite_azure::endpoints::projection::SessionKind;
use betterlite_config::sections::azure_endpoints::Settings;
use betterlite_http::client_kind::ClientKind;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::helper::HelperResult;

use super::fixture::{ASSISTANT, DELETED, NoSources, PAGE, Wire};
use super::reports;
use super::{Input, Step};

const DEFAULT_TIMEOUT: f64 = 600.0;
const SUPPLIED_BASE_URL: &str = "https://supplied.test/openai";
const SUPPLIED_API_VERSION: &str = "2023-05-01";
const SUPPLIED_DEADLINE: Duration = Duration::from_secs(42);

// Which facade the step exercises; the canned reply is the SDK body the case did not override.
#[derive(Clone, Copy)]
enum Requested {
    List,
    Create,
    Delete,
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

    fn api(&self) -> AssistantsApi<Wire> {
        AssistantsApi {
            transport: Arc::clone(&self.wire),
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
        Input::ListAssistants(steps) => steps
            .iter()
            .map(|step| definition_step(&runtime, step, Requested::List))
            .collect::<Result<_, _>>()?,
        Input::CreateAssistant(steps) => steps
            .iter()
            .map(|step| definition_step(&runtime, step, Requested::Create))
            .collect::<Result<_, _>>()?,
        Input::DeleteAssistant(steps) => steps
            .iter()
            .map(|step| definition_step(&runtime, step, Requested::Delete))
            .collect::<Result<_, _>>()?,
        Input::ClientArguments(steps) => steps.iter().map(client_step).collect::<Result<_, _>>()?,
    };
    Ok(json!({"steps": steps}))
}

// Every definition step shares one shape: build the wire and its flat settings, call the facade, report.
fn definition_step(runtime: &Runtime, step: &Step, requested: Requested) -> HelperResult {
    let (canned, call) = match requested {
        Requested::List => (PAGE, DefinitionCall::List),
        Requested::Create => {
            let Some(data) = step.data.as_ref() else {
                return Err(reports::invalid_input(String::from(
                    "a create_assistant step carries no data",
                )));
            };
            (ASSISTANT, DefinitionCall::Create(data))
        }
        Requested::Delete => {
            let Some(assistant_id) = step.assistant_id.as_deref() else {
                return Err(reports::invalid_input(String::from(
                    "a delete_assistant step carries no assistant_id",
                )));
            };
            (DELETED, DefinitionCall::Delete(assistant_id))
        }
    };
    let harness = Harness::new(step, canned);
    let request = harness.request(step);
    let outcome = runtime.block_on(harness.api().call(
        &request,
        None,
        flag(step.flag.as_ref()),
        call,
    ));
    reports::operation(&harness.wire, outcome)
}

// A client step constructs from the same inputs and reports the client, or the construction failure.
fn client_step(step: &Step) -> HelperResult {
    let harness = Harness::new(step, PAGE);
    let request = harness.request(step);
    let kind = step.supplied.map_or(step.kind, |supplied| supplied.kind);
    let supplied = step
        .supplied
        .as_ref()
        .map(|supplied| supplied_client(supplied.kind, &harness.wire));
    match harness.api().azure_client(&request, supplied, kind) {
        Ok(client) => Ok(reports::client_report(&client)),
        Err(error) => reports::client_failure(kind, &error),
    }
}

fn supplied_client(kind: ClientKind, wire: &Arc<Wire>) -> AzureAssistantsClient<'static, Wire> {
    AzureAssistantsClient {
        kind,
        inputs: None,
        transport: Arc::clone(wire),
        base_url: String::from(SUPPLIED_BASE_URL),
        api_version: String::from(SUPPLIED_API_VERSION),
        key: None,
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
