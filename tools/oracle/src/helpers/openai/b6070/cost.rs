use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};

use betterlite_bedrock::mantle_inputs::region::split_region_prefix;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::lemonade_model_metadata::{ModelInfoClient, ModelInfoRequest};
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use betterlite_models::model_metadata_resolution::hf_config::{HF_CONFIG_BASE_URL, HfConfigClient};
use betterlite_models::model_metadata_resolution::names::{
    ModelCostMap, NameNormalizers, VertexModelSets,
};
use betterlite_models::model_metadata_resolution::{
    MetadataEnvironment, MetadataRequest, get_model_info_helper,
};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_models::token_rates::ModelInfo;
use betterlite_openai::fireworks_ai_cost::error::{FireworksCostError, LookupFailed};
use betterlite_openai::fireworks_ai_cost::{
    FireworksPricingTiers, ModelInfoLookup, cost_per_token,
};
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;
use serde_json::value::RawValue;

use betterlite_models::cost_usage::CostUsage;
use betterlite_oracle_helper_api::error::HelperError;
use tokio::runtime::Builder;

const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: get_bedrock_base_model,
    bedrock_routing_prefix: strip_bedrock_routing_prefix,
    mantle_region_split: split_region_prefix,
};

#[derive(Debug, thiserror::Error)]
#[error("the helper must not fetch a Hugging Face config")]
struct NoFetch;

struct RefuseFetch;

impl ModelInfoClient for RefuseFetch {
    type Error = NoFetch;

    fn get(
        &self,
        _request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, NoFetch>> + Send {
        ready(Err(NoFetch))
    }
}
pub(super) struct ModelCostLookup {
    entries: IndexMap<String, Box<RawValue>>,
    vertex: VertexModelSets,
    catalog: ModelCatalog,
}
fn build(entries: IndexMap<String, Box<RawValue>>) -> ModelCostLookup {
    let bytes = serde_json::to_vec(&entries).unwrap_or_default();
    let vertex = VertexModelSets::build(&entries);
    let catalog = ModelCatalog::parse(&bytes, UnknownFields::Warn).unwrap_or_default();
    ModelCostLookup {
        entries,
        vertex,
        catalog,
    }
}
impl ModelCostLookup {
    pub(super) fn from_map(model_cost: &BTreeMap<String, Value>) -> Option<Self> {
        let bytes = serde_json::to_vec(model_cost).ok()?;
        let parsed = ModelCostMap::parse(&bytes).ok()?;
        Some(build(parsed.entries))
    }
}
impl ModelInfoLookup for ModelCostLookup {
    fn get(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Result<ModelInfo, LookupFailed> {
        let generalizations = CatalogGeneralizations::default();
        let known = BTreeSet::new();
        let custom = BTreeSet::new();
        let variables = BTreeMap::new();
        let resolution = ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &known,
            custom_providers: &custom,
            environment: &variables,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        };
        let environment = MetadataEnvironment {
            model_cost: &self.entries,
            vertex: &self.vertex,
            generalizations: &generalizations,
            resolution: &resolution,
            normalizers: NORMALIZERS,
        };
        let request = MetadataRequest {
            model,
            custom_llm_provider,
            api_base: None,
            api_key: None,
        };
        query(&environment, &request)
    }
}
#[expect(
    clippy::map_err_ignore,
    reason = "the gateway lookup trait answers LookupFailed, which holds no detail"
)]
fn query(
    environment: &MetadataEnvironment<'_>,
    request: &MetadataRequest<'_>,
) -> Result<ModelInfo, LookupFailed> {
    let hf = HfConfigClient {
        client: RefuseFetch,
        base_url: HF_CONFIG_BASE_URL.to_owned(),
    };
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| LookupFailed)?;
    let mut keys = ModelCostKeys::default();
    let raw = runtime
        .block_on(get_model_info_helper(
            request,
            environment,
            &mut keys,
            &NoDynamicMetadata,
            &hf,
        ))
        .map_err(|_| LookupFailed)?;
    serde_json::from_str::<ModelInfo>(raw.get()).map_err(|_| LookupFailed)
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UsageInput {
    #[serde(rename = "prompt_tokens")]
    prompt: i64,
    #[serde(rename = "completion_tokens")]
    completion: i64,
    #[serde(rename = "total_tokens")]
    total: i64,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CostCall {
    model: String,
    usage: UsageInput,
    instant: DateTime<Utc>,
}
pub(super) fn answer(
    call: &CostCall,
    lookup: &ModelCostLookup,
) -> Result<(String, String), Box<HelperError>> {
    let usage = CostUsage {
        prompt_tokens: call.usage.prompt,
        completion_tokens: call.usage.completion,
        total_tokens: call.usage.total,
        prompt_details: None,
        completion_details: None,
        cache_read_tokens: None,
        numeric_extras: BTreeMap::new(),
    };
    let (prompt, completion) = cost_per_token(
        &call.model,
        &usage,
        call.instant,
        &FireworksPricingTiers::default(),
        lookup,
    )
    .map_err(|error| cost_error(&error))?;
    Ok((
        prompt.normalize().to_string(),
        completion.normalize().to_string(),
    ))
}
fn cost_error(error: &FireworksCostError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("FireworksCostError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
