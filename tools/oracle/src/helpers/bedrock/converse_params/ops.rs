use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_anthropic::anthropic_capability_catalog::DropParams;
use betterlite_bedrock::converse_params::cache_point::{
    build_cache_point_block, get_cache_point_block,
};
use betterlite_bedrock::converse_params::compose::{ConverseParamsRequest, map_openai_params};
use betterlite_bedrock::converse_params::inference::{
    handle_top_k_value, transform_inference_params,
};
use betterlite_bedrock::converse_params::merge::{
    drop_tool_choice_type, merge_parallel_tool_use_config,
};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::bedrock_tools::name::ToolNameMap;
use serde_json::json;
use tokio::runtime::{Builder, Runtime};

use super::fixture::{CaseLookup, Fixture};
use super::input::{CachePointInput, Input, MapParamsInput, ParallelConfigInput, TopKInput};

const FIXTURE_CONFIG: &str =
    "model_list:\n  - model_name: fixture\n    litellm_params: {model: bedrock/fixture}\n";

fn helper_error<E>(error: E, status: Option<u16>) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "BedrockConverseParams".to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn budgets(environment: &BTreeMap<String, String>) -> Result<Settings, Box<HelperError>> {
    let loader = ConfigLoader {
        lookup: environment.clone(),
    };
    let config = loader
        .load(FIXTURE_CONFIG)
        .map_err(|error| helper_error(error, None))?;
    Ok(config.sections.anthropic_capability_catalog)
}

fn current_thread_runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .build()
        .map_err(|error| helper_error(error, None))
}

fn map_params(call: &MapParamsInput) -> HelperResult {
    let fixture = Fixture::load(&call.model_cost).map_err(|error| helper_error(error, None))?;
    let budget_settings = budgets(&call.environment)?;
    let catalogs = fixture.catalogs();
    let lookup = CaseLookup {
        model_cost: &call.model_cost,
    };
    let mut optional_params = call.optional_params.clone();
    let mut names = ToolNameMap::new();
    let mut keys = ModelCostKeys::default();
    let dynamic = NoDynamicMetadata;
    let resolution = fixture.resolution();
    current_thread_runtime()?
        .block_on(async {
            let mut capability = fixture.capability_environment(&resolution, &mut keys, &dynamic);
            let request = ConverseParamsRequest {
                model: &call.model,
                drop: DropParams {
                    global: call.global_drop,
                    call: call.call_drop,
                },
                non_default_params: &call.non_default_params,
                catalogs: &catalogs,
                budgets: &budget_settings,
                lookup: &lookup,
                packaged: &fixture.packaged,
            };
            map_openai_params(&request, &mut optional_params, &mut capability, &mut names).await
        })
        .map_err(|error| helper_error(&error, error.status_code()))?;
    Ok(json!({ "params": optional_params }))
}

fn cache_point(call: &CachePointInput) -> HelperResult {
    let model = call.model.as_deref();
    let block = get_cache_point_block(&call.message_block, model, &call.model_cost)
        .map_err(|error| helper_error(&error, error.status_code()))?;
    let built = build_cache_point_block(call.control.as_ref(), model, &call.model_cost)
        .map_err(|error| helper_error(&error, error.status_code()))?;
    Ok(json!({ "block": block.clone(), "content": block, "built": built }))
}

fn top_k(call: &TopKInput) -> HelperResult {
    let fixture = Fixture::load(&call.model_cost).map_err(|error| helper_error(error, None))?;
    let catalogs = fixture.catalogs();
    let drop = DropParams {
        global: call.global_drop,
        call: call.call_drop,
    };
    let mut remaining = call.inference_params.clone();
    let mapped = handle_top_k_value(&call.model, &mut remaining, drop, &catalogs)
        .map_err(|error| helper_error(&error, error.status_code()))?;
    let transformed = transform_inference_params(call.inference_params.clone());
    Ok(json!({ "mapped": mapped, "remaining": remaining, "transformed": transformed }))
}

fn parallel_config(call: &ParallelConfigInput) -> HelperResult {
    let merged = merge_parallel_tool_use_config(
        &call.additional_request_params,
        &call.parallel_tool_use_config,
    );
    let mut dropped = merged.clone();
    drop_tool_choice_type(&mut dropped);
    Ok(json!({ "merged": merged, "dropped": dropped }))
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::MapParams(call) => map_params(call),
        Input::CachePoint(call) => cache_point(call),
        Input::TopK(call) => top_k(call),
        Input::ParallelConfig(call) => parallel_config(call),
    }
}
