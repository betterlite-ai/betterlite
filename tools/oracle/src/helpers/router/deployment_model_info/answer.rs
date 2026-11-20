use std::future::Future;
use std::sync::Arc;

use betterlite_core::cost::completion::lookup::NoHfConfig;
use betterlite_models::full_model_metadata::HelperSources;
use betterlite_models::full_model_metadata::cache::ModelInfoCache;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::MetadataEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use betterlite_models::model_metadata_resolution::hf_config::{HF_CONFIG_BASE_URL, HfConfigClient};
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_model_info::cache::DiscoveryCache;
use betterlite_router::deployment_model_info::deployment_info::get_deployment_model_info;
use betterlite_router::deployment_model_info::discovery::CatalogSnapshot;
use betterlite_router::deployment_model_info::metadata::{
    HelperModelMetadata, ModelMetadataSource, RouterMetadata,
};
use betterlite_router::deployment_model_info::router_info::{
    SuppliedDeployment, get_router_model_info,
};
use betterlite_router::model_catalog_lookup::alias::AliasMap;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::routing_groups::index::RoutingGroupIndex;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::registry;

use super::fixtures::world::{
    Failure, Fixture, NORMALIZERS, argument, catalog, no_inferred, project, world_answer,
};
use super::input::{Arguments, Function, Input};
use super::logs::Events;

const AZURE_MARKERS: [&str; 2] = ["has no base_model set", "Could not identify azure model"];

fn block_on<FutureType>(future: FutureType) -> Result<FutureType::Output, Failure>
where
    FutureType: Future,
{
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| Failure(error.to_string()))?;
    Ok(runtime.block_on(future))
}

fn deployment_answer<Source>(
    metadata: &RouterMetadata<'_, '_, '_, Source>,
    arguments: &Arguments,
    keys: &mut ModelCostKeys,
) -> Result<Value, Failure>
where
    Source: ModelMetadataSource,
{
    let model_id = argument(arguments.model_id.as_deref(), "model_id")?;
    let model_name = argument(arguments.model_name.as_deref(), "model_name")?;
    let subscriber = registry().with(Events::default());
    let info = with_default(subscriber, || {
        block_on(get_deployment_model_info(
            metadata, model_id, model_name, keys,
        ))
    })?;
    Ok(project(info.as_ref()))
}

fn router_answer<Source>(
    metadata: &RouterMetadata<'_, '_, '_, Source>,
    arguments: &Arguments,
    keys: &mut ModelCostKeys,
    with_logs: bool,
) -> Result<Value, Failure>
where
    Source: ModelMetadataSource,
{
    let received = argument(
        arguments.received_model_name.as_deref(),
        "received_model_name",
    )?;
    let deployment = arguments.deployment.as_ref().map(|row| SuppliedDeployment {
        model_info: &row.model_info,
        litellm_params: row.litellm_params.as_ref(),
    });
    let events = Events::default();
    let subscriber = registry().with(events.clone());
    let answered = with_default(subscriber, || {
        block_on(get_router_model_info(
            metadata,
            deployment,
            received,
            arguments.id.as_deref(),
            keys,
        ))
    });
    let info = answered?.map_err(|error| Failure(error.to_string()))?;
    let projected = project(info.as_ref());
    if !with_logs {
        return Ok(projected);
    }
    let kept: Vec<String> = events
        .messages()
        .into_iter()
        .filter(|message| AZURE_MARKERS.iter().any(|marker| message.contains(*marker)))
        .collect();
    Ok(json!({"logs": kept, "info": projected}))
}

fn metadata_answer(
    fixture: &Fixture,
    engine: &Arc<DeploymentCatalog>,
    discovery: &DiscoveryCache,
    arguments: &Arguments,
    function: Function,
) -> Result<Value, Failure> {
    let resolution = fixture.context();
    let environment = MetadataEnvironment {
        model_cost: &fixture.map.entries,
        vertex: &fixture.map.vertex,
        generalizations: &fixture.generalizations,
        resolution: &resolution,
        normalizers: NORMALIZERS,
    };
    let hf = HfConfigClient {
        client: NoHfConfig,
        base_url: HF_CONFIG_BASE_URL.to_owned(),
    };
    let dynamic = NoDynamicMetadata;
    let sources = HelperSources {
        environment: &environment,
        dynamic: &dynamic,
        hf: &hf,
    };
    let cache = ModelInfoCache::new(64);
    let source = HelperModelMetadata {
        sources: &sources,
        cache: &cache,
    };
    let snapshot = CatalogSnapshot { catalog: engine };
    let metadata = RouterMetadata {
        snapshot: &snapshot,
        discovery,
        registry: &fixture.registry,
        resolution: &resolution,
        source: &source,
    };
    let mut keys = ModelCostKeys::default();
    match function {
        Function::DeploymentInfo => deployment_answer(&metadata, arguments, &mut keys),
        Function::RouterInfo => router_answer(&metadata, arguments, &mut keys, false),
        Function::AzureLogs => router_answer(&metadata, arguments, &mut keys, true),
        Function::Listing | Function::Configured | Function::Raw => Err(Failure(
            "this function does not read deployment model info".to_owned(),
        )),
    }
}

pub(crate) fn answer(input: &Input) -> Result<Value, Failure> {
    let fixture = Fixture::new(&input.model_cost)?;
    let engine = Arc::new(catalog(&input.model_list)?);
    let discovery = DiscoveryCache::for_deployments(engine.models.len());
    let aliases = AliasMap::default();
    let groups = RoutingGroupIndex::default();
    let lookup = CatalogLookup {
        catalog: &engine,
        model_group_alias: &aliases,
        routing_groups: &groups,
        inferred_provider: &no_inferred,
    };
    match input.function {
        Function::Listing | Function::Configured | Function::Raw => world_answer(
            &engine,
            &discovery,
            &lookup,
            &input.arguments,
            input.function,
        ),
        Function::DeploymentInfo | Function::RouterInfo | Function::AzureLogs => metadata_answer(
            &fixture,
            &engine,
            &discovery,
            &input.arguments,
            input.function,
        ),
    }
}
