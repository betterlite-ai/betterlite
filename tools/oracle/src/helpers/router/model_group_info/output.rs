use std::future::Future;

use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::names::ModelCostMap;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::model_group_info::aggregate::get_model_group_info;
use betterlite_router::model_group_info::sources::GroupSources;
use betterlite_router::routing_groups::index::RoutingGroupIndex;
use serde_json::Value;

use super::error::Failure;
use super::input::Input;
use super::snapshot::Snapshot;
use super::sources::{CaseInfo, CaseParameters, CaseProvider};

// The answer get_model_group_info gives for the Router the case describes.
pub(crate) fn answer(input: &Input) -> Result<Value, Failure> {
    let snapshot = Snapshot::build(input)?;
    let info = CaseInfo::new(&input.deployment_info, input.lookup_error)?;
    let parameters = CaseParameters::new(&input.parameters)?;
    let provider = CaseProvider {
        error: input.provider_error,
    };
    let cost_map = cost_map(&input.model_cost)?;
    let fast_mode = fast_mode_catalog(&input.model_cost)?;
    let groups = RoutingGroupIndex::default();
    let lookup = CatalogLookup {
        catalog: &snapshot.catalog,
        model_group_alias: &snapshot.aliases,
        routing_groups: &groups,
        inferred_provider: &no_inferred,
    };
    let sources = GroupSources {
        lookup: &lookup,
        deployment_info: &info,
        providers: &provider,
        parameters: &parameters,
        cost_map: &cost_map,
        fast_mode_catalog: &fast_mode,
    };
    let mut keys = ModelCostKeys::default();
    let group = block_on(get_model_group_info(
        &sources,
        &input.model_group,
        &mut keys,
    ))?;
    match group {
        Some(group) => Ok(serde_json::to_value(group.to_json())?),
        None => Ok(Value::Null),
    }
}

fn cost_map(cost: &JsonObject) -> Result<ModelCostMap, Failure> {
    Ok(ModelCostMap::parse(&serde_json::to_vec(cost)?)?)
}

// The catalog AnthropicModelInfo.supports_fast_mode reads, built from the same model_cost the Python side installs.
fn fast_mode_catalog(cost: &JsonObject) -> Result<RawCatalog, Failure> {
    Ok(serde_json::from_slice(&serde_json::to_vec(cost)?)?)
}

fn block_on<F: Future>(future: F) -> Result<F::Output, Failure> {
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    Ok(runtime.block_on(future))
}

const fn no_inferred(_model: &str) -> Option<String> {
    None
}
