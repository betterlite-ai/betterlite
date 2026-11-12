use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::deployment_filters::{OrderedDeployment, get_order_filtered_deployments};
use betterlite_models::deployment_regions::{
    ProviderRegionLists, RegionParams, get_model_region, infer_model_region,
};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperResult, OrHelperError as _};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Order {
    order: Option<i64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Deployment {
    id: String,
    litellm_params: Option<Order>,
    model_info: Option<Order>,
}

impl OrderedDeployment for Deployment {
    fn litellm_params_order(&self) -> Option<i64> {
        self.litellm_params.as_ref()?.order
    }

    fn model_info_order(&self) -> Option<i64> {
        self.model_info.as_ref()?.order
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Params {
    model: String,
    custom_llm_provider: Option<String>,
    api_base: Option<String>,
    region_name: Option<String>,
    vertex_location: Option<String>,
    aws_region_name: Option<String>,
    watsonx_region_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegionCall {
    params: Params,
    lists: ProviderRegionLists,
    default_vertex_location: Option<String>,
    provider: Option<String>,
    allowed_model_region: String,
}

pub(super) fn order_filter(deployments: &[Deployment], target_order: Option<i64>) -> Value {
    let orders: Vec<Option<i64>> = deployments.iter().map(OrderedDeployment::order).collect();
    let kept: Vec<String> = get_order_filtered_deployments(deployments.to_vec(), target_order)
        .into_iter()
        .map(|deployment| deployment.id)
        .collect();
    json!({"orders": orders, "kept": kept})
}

pub(super) fn region(input: &RegionCall) -> HelperResult {
    let found = &input.params;
    let params = RegionParams {
        model: &found.model,
        custom_llm_provider: found.custom_llm_provider.as_deref(),
        api_base: found.api_base.as_deref(),
        api_key: None,
        use_litellm_proxy: false,
        region_name: found.region_name.as_deref(),
        vertex_location: found.vertex_location.as_deref(),
        aws_region_name: found.aws_region_name.as_deref(),
        watsonx_region_name: found.watsonx_region_name.as_deref(),
    };
    let default_vertex = input.default_vertex_location.as_deref();
    let catalog = ModelCatalog::parse(b"{}", UnknownFields::Reject).or_value_error()?;
    let known_model_names = BTreeSet::new();
    let custom_providers = BTreeSet::new();
    let environment = BTreeMap::new();
    let context = ResolutionContext {
        catalog: &catalog,
        known_model_names: &known_model_names,
        custom_providers: &custom_providers,
        environment: &environment,
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let inferred =
        infer_model_region(&params, &context, &input.lists, default_vertex).or_value_error()?;
    let model_region = input
        .provider
        .as_deref()
        .and_then(|provider| get_model_region(provider, &params, default_vertex));
    // is_region_allowed is this one comparison, so the helper holds it where it is used
    let allowed = found.region_name.as_deref() == Some(input.allowed_model_region.as_str());
    Ok(json!({
        "lists": input.lists,
        "model_region": model_region,
        "inferred": inferred.map(|region| region.to_string()),
        "allowed": allowed,
    }))
}
