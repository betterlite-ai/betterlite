use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::assistant_settings::{Config as AssistantConfig, Raw, Settings};
use betterlite_config::sections::{azure_endpoints, resource_sdk_settings};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::assistant_read_delete::sdk_dispatch::NoSources;
use betterlite_core::api::assistant_run_sdk::service::RunOutcome;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::thread_runs::stream::RunStream;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_server::api::assistant_dispatch::{
    AssistantDispatchError, AssistantDispatcher, AssistantOutcome, AssistantRequest,
};
use serde::Serialize;

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::Failure;
use super::result::kinds::SdkResult;
use super::vector_query_embedding::outbound_client;

pub mod model;
mod projection;

pub use model::{Call, ConfigProjection, OperationCall, Outcome, Payload, Step};

use projection::{failed_step, named, nullable, record};

// the assistant_settings mapping the startup block reads
#[derive(Serialize)]
struct SettingsRoot<'config> {
    assistant_settings: &'config ConfigProjection,
}

// The loaded config and the borrowed services every run shares.
struct Wiring<'wiring> {
    resources: &'wiring resource_sdk_settings::Settings,
    azure: &'wiring azure_endpoints::Settings,
    environment: &'wiring BTreeMap<String, String>,
    resolution: &'wiring ResolutionContext<'wiring>,
    redactor: &'wiring Redactor,
    transport: &'wiring Arc<ProxiedClient>,
}

impl CallKind for Call {
    async fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> Result<SdkResult, SdkHttpError> {
        execute(self, api_base, env).await
    }
}

async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let loaded = ConfigLoader {
        lookup: env.clone(),
    }
    .load("model_list: []")
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let catalog = ModelCatalog::default();
    let names = BTreeSet::new();
    let resolution = ResolutionContext {
        catalog: &catalog,
        known_model_names: &names,
        custom_providers: &names,
        environment: env,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
    let transport = Arc::new(outbound_client(env)?);
    let wiring = Wiring {
        resources: &loaded.sections.resource_sdk_settings,
        azure: &loaded.sections.azure_endpoints,
        environment: env,
        resolution: &resolution,
        redactor: &redactor,
        transport: &transport,
    };
    let mut results = Vec::with_capacity(call.runs.len());
    for run in &call.runs {
        results.push(one(&wiring, run, api_base).await?);
    }
    Ok(SdkResult::AssistantDispatch {
        value: Box::new(Outcome { results }),
    })
}

async fn one(
    wiring: &Wiring<'_>,
    run: &OperationCall,
    api_base: &str,
) -> Result<Step, SdkHttpError> {
    let settings = assistant_config(run, wiring.environment)?;
    let body = data(run, api_base);
    let request = AssistantRequest {
        operation: run.operation,
        data: &body,
        thread_id: run.thread_id.as_deref(),
        assistant_id: run.assistant_id.as_deref(),
        client: None,
        event_handler: None,
    };
    let dispatcher = AssistantDispatcher {
        settings: &settings,
        resources: wiring.resources,
        azure: wiring.azure,
        environment: wiring.environment,
        sources: NoSources,
        resolution: wiring.resolution,
        redactor: wiring.redactor,
        transport: Arc::clone(wiring.transport),
        limits: ApiLimits::default(),
    };
    match dispatcher.dispatch(&request).await {
        Ok(answer) => project(answer).await,
        Err(error) => dropped(error),
    }
}

// The startup block reads assistants_config from YAML once; a direct one bypasses that block entirely.
fn assistant_config(
    call: &OperationCall,
    env: &BTreeMap<String, String>,
) -> Result<Settings, SdkHttpError> {
    let Some(config) = call.config.as_ref() else {
        return Ok(Settings::default());
    };
    if call.startup {
        let value = serde_json::to_value(SettingsRoot {
            assistant_settings: config,
        })
        .map_err(SdkHttpError::Serialize)?;
        let raw = serde_json::from_value::<Raw>(value)
            .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
        return raw
            .convert(env)
            .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)));
    }
    Ok(Settings {
        config: Some(AssistantConfig {
            custom_llm_provider: config.custom_llm_provider.clone(),
            litellm_params: config.litellm_params.clone(),
        }),
    })
}

// The proxy's **data: the typed body, its parameters, the explicit provider and the mock origin.
fn data(call: &OperationCall, api_base: &str) -> BTreeMap<String, Json> {
    let payload = &call.payload;
    let mut data = payload.params.clone();
    for (name, value) in [
        ("assistant_id", &payload.assistant_id),
        ("model", &payload.model),
        ("name", &payload.name),
        ("role", &payload.role),
        ("instructions", &payload.instructions),
        ("additional_instructions", &payload.additional_instructions),
    ] {
        if let Some(value) = value {
            data.insert(name.to_owned(), Json::Text(value.clone()));
        }
    }
    for (name, value) in [
        ("content", &payload.content),
        ("attachments", &payload.attachments),
        ("messages", &payload.messages),
        ("metadata", &payload.metadata),
        ("tool_resources", &payload.tool_resources),
        ("stream", &payload.stream),
        ("tools", &payload.tools),
    ] {
        if let Some(value) = value {
            data.insert(name.to_owned(), value.clone());
        }
    }
    if let Some(provider) = &call.provider {
        data.insert(
            String::from("custom_llm_provider"),
            Json::Text(provider.clone()),
        );
    }
    data.insert(String::from("api_base"), Json::Text(api_base.to_owned()));
    data
}

async fn project(
    answer: Result<AssistantOutcome<RunStream<ProxiedClient>>, AssistantDispatchError>,
) -> Result<Step, SdkHttpError> {
    match answer {
        // entering the manager starts the request, so a consumed stream has issued it by the time it returns
        Ok(AssistantOutcome::Run(RunOutcome::Streaming(stream))) => {
            let consumed = stream
                .consume()
                .await
                .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
            Ok(Step {
                ids: Vec::new(),
                status: None,
                events: Some(
                    consumed
                        .iter()
                        .map(|event| event.name().to_owned())
                        .collect(),
                ),
                failure: None,
            })
        }
        Ok(outcome) => {
            let (ids, status) = answered(outcome);
            Ok(record(ids, status))
        }
        Err(error) => dropped(error),
    }
}

// The ids each non-streaming answer returns, and the status a polled run carries.
fn answered(outcome: AssistantOutcome<RunStream<ProxiedClient>>) -> (Vec<String>, Option<String>) {
    match outcome {
        AssistantOutcome::Assistant(assistant) => (vec![assistant.id], None),
        AssistantOutcome::Assistants(page) => (
            page.data
                .into_iter()
                .map(|assistant| assistant.id)
                .collect(),
            None,
        ),
        AssistantOutcome::Deleted(deleted) => (vec![deleted.id], None),
        AssistantOutcome::Thread(thread) => (
            thread
                .map(|thread_record| thread_record.id)
                .into_iter()
                .collect(),
            None,
        ),
        AssistantOutcome::Message(message) => (vec![message.id], None),
        AssistantOutcome::Messages(page) => (
            page.data.into_iter().map(|message| message.id).collect(),
            None,
        ),
        AssistantOutcome::Run(RunOutcome::Polled(reply)) => {
            (*reply).as_ref().map_or((Vec::new(), None), |run| {
                (vec![run.id.clone()], Some(run.status.clone()))
            })
        }
        AssistantOutcome::Run(RunOutcome::Streaming(_)) => (Vec::new(), None),
    }
}

// The factory's own errors have no status, type, param or code; the async wrapper's classes do.
fn dropped(error: AssistantDispatchError) -> Result<Step, SdkHttpError> {
    let failure = match error {
        AssistantDispatchError::Exception { message } => named("Exception", message),
        AssistantDispatchError::KeyError { key } => named("KeyError", format!("'{key}'")),
        AssistantDispatchError::TypeError { message }
        | AssistantDispatchError::Invalid { message } => named("TypeError", message),
        AssistantDispatchError::Failure { failure } => {
            let Some(status) = failure.status else {
                return Err(SdkHttpError::Invalid(format!(
                    "the dispatcher raised {} without a status code",
                    failure.class
                )));
            };
            Failure {
                class: failure.class,
                status,
                kind: nullable(failure.kind),
                param: nullable(failure.param),
                code: nullable(failure.code),
                message: failure.message,
            }
        }
        other @ (AssistantDispatchError::ReadDelete(_)
        | AssistantDispatchError::Message(_)
        | AssistantDispatchError::Run(_)
        | AssistantDispatchError::Thread { .. }) => {
            return Err(SdkHttpError::Invalid(format!(
                "the dispatcher answered with {other}"
            )));
        }
    };
    Ok(failed_step(failure))
}
