use std::sync::Arc;
use std::time::Duration;

use betterlite_azure::azure_assistant_definitions::client::AzureAssistantsClient;
use betterlite_azure::azure_assistant_definitions::operations::{AssistantsApi, ClientRequest};
use betterlite_azure::endpoints::projection::SessionKind;
use betterlite_config::sections::azure_endpoints::Settings;
use betterlite_openai::assistants::client::ClientKind;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::helper::HelperResult;

use super::fixture::{ASSISTANT, DELETED, NoSources, PAGE, Wire};
use super::reports;
use super::{Input, KindName, Step};

const DEFAULT_TIMEOUT: f64 = 600.0;
const SUPPLIED_BASE_URL: &str = "https://supplied.test/openai";
const SUPPLIED_API_VERSION: &str = "2023-05-01";
const SUPPLIED_DEADLINE: Duration = Duration::from_secs(42);

pub(super) fn run(input: &Input) -> HelperResult {
    let runtime = Builder::new_current_thread()
        .build()
        .map_err(|error| reports::invalid_input(error.to_string()))?;
    match input {
        Input::ListAssistants(steps) => {
            steps_of(steps.iter().map(|step| list_step(&runtime, step)))
        }
        Input::CreateAssistant(steps) => {
            steps_of(steps.iter().map(|step| create_step(&runtime, step)))
        }
        Input::DeleteAssistant(steps) => {
            steps_of(steps.iter().map(|step| delete_step(&runtime, step)))
        }
        Input::ClientArguments(steps) => steps_of(steps.iter().map(client_step)),
    }
}

fn steps_of(outcomes: impl Iterator<Item = HelperResult>) -> HelperResult {
    let steps = outcomes.collect::<Result<Vec<Value>, _>>()?;
    Ok(json!({"steps": steps}))
}

fn list_step(runtime: &Runtime, step: &Step) -> HelperResult {
    let wire = Wire::replying(step.reply.as_ref(), PAGE);
    let api = AssistantsApi {
        transport: Arc::clone(&wire),
    };
    let settings = Settings::default();
    let sources = NoSources;
    let request = request(step, &settings, &sources);
    let outcome = runtime.block_on(api.get_assistants(&request, None, flag(step.flag.as_ref())));
    reports::operation(&wire, outcome.map(|reply| (reply.kind, reply.reply)))
}

fn create_step(runtime: &Runtime, step: &Step) -> HelperResult {
    let Some(data) = step.data.as_ref() else {
        return Err(reports::invalid_input(String::from(
            "a create_assistant step carries no data",
        )));
    };
    let wire = Wire::replying(step.reply.as_ref(), ASSISTANT);
    let api = AssistantsApi {
        transport: Arc::clone(&wire),
    };
    let settings = Settings::default();
    let sources = NoSources;
    let request = request(step, &settings, &sources);
    let outcome =
        runtime.block_on(api.create_assistants(&request, data, None, flag(step.flag.as_ref())));
    reports::operation(&wire, outcome.map(|reply| (reply.kind, reply.reply)))
}

fn delete_step(runtime: &Runtime, step: &Step) -> HelperResult {
    let Some(assistant_id) = step.assistant_id.as_deref() else {
        return Err(reports::invalid_input(String::from(
            "a delete_assistant step carries no assistant_id",
        )));
    };
    let wire = Wire::replying(step.reply.as_ref(), DELETED);
    let api = AssistantsApi {
        transport: Arc::clone(&wire),
    };
    let settings = Settings::default();
    let sources = NoSources;
    let request = request(step, &settings, &sources);
    let outcome = runtime.block_on(api.delete_assistant(
        &request,
        assistant_id,
        None,
        flag(step.flag.as_ref()),
    ));
    reports::operation(&wire, outcome.map(|reply| (reply.kind, reply.reply)))
}

fn client_step(step: &Step) -> HelperResult {
    let wire = Wire::replying(None, PAGE);
    let api = AssistantsApi {
        transport: Arc::clone(&wire),
    };
    let settings = Settings::default();
    let sources = NoSources;
    let request = request(step, &settings, &sources);
    let kind = step.supplied.map_or(step.kind, |supplied| supplied.kind);
    let supplied = step
        .supplied
        .as_ref()
        .map(|supplied| supplied_client(supplied.kind, &wire));
    let returned = match client_kind(kind) {
        ClientKind::Sync => api.get_azure_client(&request, supplied),
        ClientKind::Async => api.async_get_azure_client(&request, supplied),
    };
    match returned {
        Ok(client) => Ok(reports::client_report(&client)),
        Err(error) => reports::client_failure(client_kind(kind), &error),
    }
}

fn request<'data>(
    step: &'data Step,
    settings: &'data Settings,
    sources: &'data NoSources,
) -> ClientRequest<'data> {
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
        settings,
        environment: &step.environment,
        sources,
    }
}

fn supplied_client(kind: KindName, wire: &Arc<Wire>) -> AzureAssistantsClient<'static, Wire> {
    AzureAssistantsClient {
        kind: client_kind(kind),
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

const fn client_kind(kind: KindName) -> ClientKind {
    match kind {
        KindName::Sync => ClientKind::Sync,
        KindName::Async => ClientKind::Async,
    }
}
