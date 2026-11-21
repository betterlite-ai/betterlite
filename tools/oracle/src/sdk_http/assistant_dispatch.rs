use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::{azure_endpoints, resource_sdk_settings};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::assistant_read_delete::sdk_dispatch::NoSources;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_server::api::assistant_dispatch::AssistantDispatcher;
use betterlite_server::api::assistant_dispatch::request::AssistantRequest;
use serde::{Deserialize, Serialize};

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;
use super::vector_query_embedding::outbound_client;

pub mod model;
mod projection;
mod request;

use model::{OperationCall, Step};
use projection::project;
use request::{assistant_config, data};

// A case runs the installed wrappers under one assistants_config each, in order.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub runs: Vec<OperationCall>,
}

// The ids the wrapper returned, the status a polled run carries, the events a consumed stream yielded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub results: Vec<Step>,
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
    project(dispatcher.dispatch(&request).await).await
}
