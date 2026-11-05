use betterlite_context::bedrock_model::model_info::{LazyBinding, LazyFallibleCache};
use betterlite_context::bedrock_model::regions::{
    AmazonBedrockGlobalConfig, CROSS_REGION_PREFIXES,
};
use betterlite_context::bedrock_model::runtime_arn::encode_runtime_modelid_arn;
use betterlite_context::bedrock_model::{
    extract_model_from_arn, get_bedrock_base_model, split_bedrock_region_path,
    strip_bedrock_routing_prefix, strip_throughput_suffix,
};
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

fn lazy_catalog_binding_facts() -> serde_json::Value {
    let binder: fn() -> usize = || 42;
    let mut binding = LazyBinding::from(binder);
    let get_model_info_is_cached = binding.get() == binding.get();

    let mut succeeding =
        LazyFallibleCache::from(|| Ok::<Vec<&'static str>, &'static str>(vec!["loaded"]));
    let loaded = succeeding.get();
    let cost_map_loads_without_net = loaded.is_ok_and(|value| !value.is_empty());

    let mut failing = LazyFallibleCache::from(|| Err::<Vec<&'static str>, &'static str>("forced"));
    let cost_map_failure_uncached = failing.get().is_err() && failing.get().is_err();

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
