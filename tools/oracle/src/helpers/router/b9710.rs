use std::future::{Future, ready};

use betterlite_context::bedrock_model::get_bedrock_base_model;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::decisions::Json;
use betterlite_models::lookup::cost_map_keys::backend_cost_map_keys;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_registration::{
    CostMapFetcher, JsonObject, ModelCostEntries, ModelInfoLookup, ModelRegistry,
    RegistrationEnvironment,
};
use betterlite_models::provider_model_sets::ProviderModelSets;
use betterlite_router::deployment_pricing::PricingInputs;
use betterlite_router::deployment_pricing::deployment::Deployment;
use betterlite_router::deployment_pricing::register::register_deployment_pricing;
use serde::Deserialize;
use serde_json::{Number, Value};

use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

const RATE_FIELDS: [&str; 2] = ["input_cost_per_token", "output_cost_per_token"];
// get_model_info reads a name like responses/gpt-4.1 as another provider, so only the providers the cases use are stripped.
const PROVIDER_PREFIXES: [&str; 2] = ["openai/", "anthropic/"];

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    deployment: DeploymentInput,
    shared_before: Option<JsonObject>,
}

#[derive(Debug, Deserialize)]
struct DeploymentInput {
    litellm_params: JsonObject,
    model_info: JsonObject,
}

// Finds an entry by its name or without a provider prefix and fills a missing flat rate with 0, as get_model_info does.
struct BareLookup;

impl ModelInfoLookup for BareLookup {
    fn get_model_info(
        &self,
        model: &str,
        model_cost: &JsonObject,
        _keys: &mut ModelCostKeys,
    ) -> impl Future<Output = Option<JsonObject>> + Send {
        let bare = PROVIDER_PREFIXES
            .iter()
            .find_map(|prefix| model.strip_prefix(prefix))
            .unwrap_or(model);
        let found = [model, bare]
            .into_iter()
            .find_map(|key| match model_cost.get(key) {
                Some(Json::Object(entry)) => Some((key, entry)),
                _ => None,
            });
        ready(found.map(|(key, entry)| {
            let mut info = entry.clone();
            info.insert("key".to_owned(), Json::Text(key.to_owned()));
            for field in RATE_FIELDS {
                let rate = match info.get(field) {
                    None | Some(Json::Null) => Json::Number(Number::from(0_u8)),
                    Some(stored) => stored.clone(),
                };
                info.insert(field.to_owned(), rate);
            }
            info
        }))
    }

    fn clear_caches(&self) {}
}

struct NoFetch;

#[derive(Debug, thiserror::Error)]
#[error("a deployment registration must not fetch a cost map")]
struct NoFetchError;

impl CostMapFetcher for NoFetch {
    type Error = NoFetchError;

    fn get_model_cost_map(
        &self,
        _url: &str,
        _max_attempts: u32,
    ) -> impl Future<Output = Result<ModelCostEntries, NoFetchError>> + Send {
        ready(Err(NoFetchError))
    }
}

fn shown(entry: &Json) -> Value {
    let Json::Object(fields) = entry else {
        return Value::Null;
    };
    let kept = fields
        .iter()
        .filter(|(_, value)| !matches!(value, Json::Null))
        .filter_map(|(name, value)| Some((name.clone(), serde_json::to_value(value).ok()?)));
    Value::Object(kept.collect())
}

fn bundled_registry() -> Result<ModelRegistry, Box<HelperError>> {
    let bytes = (BundledCatalog::default().asset)().or_value_error()?;
    Ok(ModelRegistry {
        model_cost: serde_json::from_slice(&bytes).or_value_error()?,
        runtime_registered: ModelCostEntries::new(),
        provider_sets: ProviderModelSets::default(),
        keys: ModelCostKeys::default(),
        live_replay: None,
    })
}

// What ModelInfo.model_dump(exclude_none=True) holds for a deployment that sets only some fields.
fn dumped_model_info(given: &JsonObject) -> JsonObject {
    let mut dumped = given.clone();
    for flag in ["db_model", "member_auto_router"] {
        dumped.entry(flag.to_owned()).or_insert(Json::Bool(false));
    }
    dumped
}

async fn answer(input: &Input) -> HelperResult {
    let mut registry = bundled_registry()?;
    let params = &input.deployment.litellm_params;
    let model_info = dumped_model_info(&input.deployment.model_info);
    let deployment = Deployment::read(params, &model_info).or_value_error()?;
    let backend_keys = backend_cost_map_keys(
        deployment.target.model,
        deployment.target.custom_llm_provider,
    );
    if let (Some(first), Some(before)) = (backend_keys.first(), &input.shared_before) {
        registry
            .model_cost
            .insert(first.clone(), Json::Object(before.clone()));
    }
    let generalizations = CatalogGeneralizations::default();
    let environment = RegistrationEnvironment {
        lookup: &BareLookup,
        fetcher: &NoFetch,
        generalizations: &generalizations,
        bedrock_base_model: get_bedrock_base_model,
    };
    let inputs = PricingInputs {
        ptu_cost_attribution: false,
        is_strategy_router: |name| name.starts_with("auto_router/"),
    };
    register_deployment_pricing(&mut registry, params, &model_info, &environment, &inputs)
        .await
        .or_value_error()?;
    let by_id = deployment
        .target
        .model_id
        .and_then(|id| registry.model_cost.get(id))
        .map_or(Value::Null, shown);
    let shared: serde_json::Map<String, Value> = backend_keys
        .iter()
        .filter_map(|key| Some((key.clone(), shown(registry.model_cost.get(key)?))))
        .collect();
    Ok(serde_json::json!({"by_id": by_id, "shared": shared}))
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?
            .block_on(answer(input))
    }
}
