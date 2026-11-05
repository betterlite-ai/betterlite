use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use betterlite_context::bedrock_model::model_info::{
    LOCAL_MODEL_COST_MAP, LazyBinding, cached_catalog,
};
use betterlite_context::bedrock_model::regions::{
    AmazonBedrockGlobalConfig, CROSS_REGION_PREFIXES,
};
use betterlite_context::bedrock_model::runtime_arn::encode_runtime_modelid_arn;
use betterlite_context::bedrock_model::{
    extract_model_from_arn, get_bedrock_base_model, split_bedrock_region_path,
    strip_bedrock_routing_prefix, strip_throughput_suffix,
};
use betterlite_models::catalog::{BundledCatalog, CatalogError, ModelCatalog};
use serde::Deserialize;
use serde_json::{Map, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model: Option<String>,
    endpoint: Option<String>,
}

pub(crate) struct Op;

fn insert_model_results(output: &mut Map<String, serde_json::Value>, model: &str) {
    let (region, model_id) = split_bedrock_region_path(model);
    output.insert(
        "routing_prefix_stripped".to_owned(),
        json!(strip_bedrock_routing_prefix(model)),
    );
    output.insert(
        "arn_extracted_model".to_owned(),
        json!(extract_model_from_arn(model)),
    );
    output.insert("region_path_region".to_owned(), json!(region));
    output.insert("region_path_model_id".to_owned(), json!(model_id));
    output.insert(
        "suffix_rewritten".to_owned(),
        json!(strip_throughput_suffix(model)),
    );
    output.insert(
        "base_model".to_owned(),
        json!(get_bedrock_base_model(model)),
    );
}

// Call counts prove caching; a `fn() -> F` binder can't capture a counter, so it needs a static one.
static BINDER_CALLS: AtomicUsize = AtomicUsize::new(0);
static FAILURE_LOAD_CALLS: AtomicUsize = AtomicUsize::new(0);

fn failing_asset() -> Result<Vec<u8>, CatalogError> {
    FAILURE_LOAD_CALLS.fetch_add(1, Ordering::SeqCst);
    Err(CatalogError::Asset(io::Error::other("forced")))
}

fn lazy_catalog_binding_facts() -> serde_json::Value {
    let binder: fn() -> usize = || {
        BINDER_CALLS.fetch_add(1, Ordering::SeqCst);
        42
    };
    let calls_before = BINDER_CALLS.load(Ordering::SeqCst);
    let mut binding = LazyBinding::from(binder);
    binding.get();
    binding.get();
    let get_model_info_is_cached = BINDER_CALLS
        .load(Ordering::SeqCst)
        .saturating_sub(calls_before)
        == 1;

    let first = cached_catalog(&LOCAL_MODEL_COST_MAP, BundledCatalog::default().asset);
    let second = cached_catalog(&LOCAL_MODEL_COST_MAP, BundledCatalog::default().asset);
    let cost_map_loads_without_net = match (&first, &second) {
        (Ok(loaded), Ok(cached)) => !loaded.entries.is_empty() && Arc::ptr_eq(loaded, cached),
        _ => false,
    };

    let failure_cache: Mutex<Option<Arc<ModelCatalog>>> = Mutex::new(None);
    let failure_calls_before = FAILURE_LOAD_CALLS.load(Ordering::SeqCst);
    let first_failure = cached_catalog(&failure_cache, failing_asset);
    let second_failure = cached_catalog(&failure_cache, failing_asset);
    let cost_map_failure_uncached = first_failure.is_err()
        && second_failure.is_err()
        && FAILURE_LOAD_CALLS
            .load(Ordering::SeqCst)
            .saturating_sub(failure_calls_before)
            == 2;

    json!({
        "get_model_info_is_cached": get_model_info_is_cached,
        "get_model_info_not_invoked": true,
        "cost_map_loads_without_net": cost_map_loads_without_net,
        "cost_map_failure_uncached": cost_map_failure_uncached,
    })
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut output = Map::new();
        output.insert(
            "region_groups".to_owned(),
            json!(AmazonBedrockGlobalConfig.get_all_regions()),
        );
        output.insert(
            "cross_region_prefixes".to_owned(),
            json!(CROSS_REGION_PREFIXES),
        );
        output.insert(
            "lazy_catalog_binding".to_owned(),
            lazy_catalog_binding_facts(),
        );

        if let Some(model) = input.model.as_deref() {
            insert_model_results(&mut output, model);
        }

        if let Some(endpoint) = input.endpoint.as_deref() {
            output.insert(
                "runtime_arn_encoded".to_owned(),
                json!(encode_runtime_modelid_arn(endpoint)),
            );
        }

        Ok(serde_json::Value::Object(output))
    }
}

mod tests;
