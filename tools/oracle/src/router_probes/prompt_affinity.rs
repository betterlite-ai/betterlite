use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_core::message_utilities::prompt_cache_checks::PromptCacheSettings;
use betterlite_logging::logging_metadata::base64::Base64Truncator;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use betterlite_prompts::cache_accounting::dialect::ModelFacts;
use betterlite_prompts::cache_defaults::CacheFacts;
use betterlite_prompts::message_content::Message;
use betterlite_router::deployment_identity::error::InvalidDeployment;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::prompt_affinity::cache::PinCache;
use betterlite_router::prompt_affinity::json::messages_json;
use betterlite_router::prompt_affinity::prefix::prompt_caching_cache_key;
use betterlite_router::prompt_affinity::service::{
    CountingGate, FilterRequest, InjectionDefaults, PromptAffinity, PromptAffinityDependencies,
    SuccessEvent,
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use tokio::runtime::Builder;

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

mod counter;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub model: String,
    pub deployments: Vec<Deployment>,
    #[serde(default)]
    pub write: Option<Write>,
    #[serde(default)]
    pub messages: Option<Vec<Message>>,
    #[serde(default)]
    pub enable_prompt_caching: Option<bool>,
    #[serde(default)]
    pub target_order: Option<i64>,
    #[serde(default)]
    pub global_enabled: bool,
    #[serde(default)]
    pub facts: Facts,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Deployment {
    pub id: String,
    pub model: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Write {
    pub model_id: String,
    #[serde(default = "default_call_type")]
    pub call_type: String,
    pub messages: Vec<Json>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Facts {
    #[serde(default)]
    pub providers: BTreeMap<String, String>,
    #[serde(default)]
    pub supports: BTreeMap<String, bool>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub selected: Vec<usize>,
    pub cache_key: Option<String>,
    pub pin: Option<String>,
}

#[derive(Default)]
struct ProbePins {
    values: Mutex<BTreeMap<String, Json>>,
}

#[async_trait]
impl PinCache for ProbePins {
    async fn store(&self, key: &str, value: &Json) {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value.clone());
    }

    async fn read(&self, keys: &[String]) -> Vec<Option<Json>> {
        let values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        keys.iter().map(|key| values.get(key).cloned()).collect()
    }
}

struct InputFacts {
    providers: BTreeMap<String, String>,
    supports: BTreeMap<String, bool>,
}

impl ModelFacts for InputFacts {
    fn cache_breakpoint_flag(&self, _model: &str) -> Option<bool> {
        None
    }

    fn provider(&self, model: &str) -> Option<String> {
        self.providers.get(model).cloned()
    }
}

impl CacheFacts for InputFacts {
    fn supports_prompt_caching(&self, model: &str, _provider: &str) -> bool {
        self.supports.get(model).copied().unwrap_or(false)
    }
}

fn default_call_type() -> String {
    "completion".to_owned()
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(invalid)?;
    let output = runtime.block_on(simulate(input)).map_err(invalid)?;
    Ok(ProbeOutput::PromptAffinity(output))
}

fn service(input: &Input) -> Result<PromptAffinity, String> {
    let (counter, catalog) = counter::counting_pieces().map_err(|error| error.to_string())?;
    let settings = PromptCacheSettings::default();
    Ok(PromptAffinity::from(PromptAffinityDependencies {
        pins: Arc::new(ProbePins::default()),
        gate: Arc::new(CountingGate {
            counter: Arc::new(counter),
            settings,
            catalog: Arc::clone(&catalog),
        }),
        facts: Arc::new(InputFacts {
            providers: input.facts.providers.clone(),
            supports: input.facts.supports.clone(),
        }),
        catalog: Arc::clone(&catalog),
        settings,
        truncator: Base64Truncator::from_env(None).map_err(|error| error.to_string())?,
        defaults: InjectionDefaults {
            global_enabled: input.global_enabled,
            ttl: None,
            alias_map: BTreeMap::new(),
        },
    }))
}

async fn record(service: &PromptAffinity, input: &Input, write: &Write) -> Result<(), String> {
    let event = SuccessEvent {
        call_type: &write.call_type,
        model: &input.model,
        messages: Some(&write.messages),
        model_id: Some(&write.model_id),
    };
    service
        .record_success(&event)
        .await
        .map_err(|error| error.to_string())
}

async fn simulate(input: &Input) -> Result<Output, String> {
    let service = service(input)?;
    let messages: Option<Vec<Json>> = input.messages.as_deref().map(messages_json);
    if let Some(write) = &input.write {
        record(&service, input, write).await?;
    }
    let pool = pool(input)?;
    let kwargs = request_kwargs(input);
    let selected = service
        .filter_deployments(&FilterRequest {
            model: &input.model,
            deployments: &pool,
            messages: input.messages.as_deref(),
            request_kwargs: Some(&kwargs),
            priority_group: false,
            target_order: input.target_order,
        })
        .await
        .map_err(|error| error.to_string())?;
    let pin = service
        .pin_for(messages.as_deref(), None)
        .await
        .map_err(|error| error.to_string())?;
    let truncator = Base64Truncator::from_env(None).map_err(|error| error.to_string())?;
    Ok(Output {
        selected: selected
            .iter()
            .filter_map(|deployment| pool.iter().position(|entry| entry == deployment))
            .collect(),
        cache_key: prompt_caching_cache_key(messages.as_deref(), None, &truncator)
            .map_err(|error| error.to_string())?,
        pin,
    })
}

// the stand-down and the default-breakpoint scan read the same request kwargs the router passes
fn request_kwargs(input: &Input) -> Object {
    let mut kwargs = Object::new();
    if let Some(flag) = input.enable_prompt_caching {
        kwargs.insert("enable_prompt_caching".to_owned(), PromptJson::Bool(flag));
    }
    kwargs
}

fn pool(input: &Input) -> Result<Vec<DeploymentSpec>, String> {
    let mut pool = Vec::with_capacity(input.deployments.len());
    for deployment in &input.deployments {
        pool.push(deployment_spec(deployment).map_err(|error| error.to_string())?);
    }
    Ok(pool)
}

fn deployment_spec(deployment: &Deployment) -> Result<DeploymentSpec, InvalidDeployment> {
    let mut params = JsonObject::new();
    params.insert("model".to_owned(), Json::Text(deployment.model.clone()));
    let mut info = JsonObject::new();
    info.insert("id".to_owned(), Json::Text(deployment.id.clone()));
    DeploymentSpec::new(
        deployment.model.clone(),
        params,
        info,
        IndexMap::new(),
        || String::new(),
    )
}

fn invalid<E>(error: E) -> OperationError
where
    E: ToString,
{
    OperationError::InvalidRequest {
        message: error.to_string(),
    }
}
