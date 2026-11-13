use std::collections::BTreeSet;
use std::fmt::Display;

use betterlite_bedrock::bedrock_model_capabilities::cache_policy::{
    accepts_cache_points, is_claude_4_5_on_bedrock, supports_parallel_tools,
};
use betterlite_bedrock::bedrock_model_capabilities::effort::normalize_opus_effort;
use betterlite_bedrock::bedrock_model_capabilities::endpoints::supports_openai_responses;
use betterlite_bedrock::bedrock_model_capabilities::mantle::mantle_messages_url;
use betterlite_bedrock::bedrock_model_capabilities::strict_tools::converse_supports_strict_tools;
use betterlite_bedrock::bedrock_model_capabilities::tool_search::supports_tool_search;
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_config::loader::ConfigLoader;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use super::fixture::{CachedModelInfo, Fixture};
use super::input::{EffortCase, Input, MantleCase};
use crate::helpers::{HelperError, HelperResult};

const FIXTURE_CONFIG: &str =
    "model_list:\n  - model_name: fixture\n    litellm_params: {model: bedrock/fixture}\n";

fn failure<E: Display>(error: E) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ModelCapabilities".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn mantle_api_base(region: &str, cases: &[MantleCase]) -> HelperResult {
    let mut urls = Vec::with_capacity(cases.len());
    for case in cases {
        let config = ConfigLoader {
            lookup: case.environment.clone(),
        }
        .load(FIXTURE_CONFIG)
        .map_err(failure)?;
        let mantle_api_base = config.sections.bedrock_mantle_inputs.api_base.as_deref();
        urls.push(json!(mantle_messages_url(
            case.api_base.as_deref(),
            case.aws_bedrock_runtime_endpoint.as_deref(),
            mantle_api_base,
            region,
        )));
    }
    Ok(json!({ "urls": urls }))
}

#[derive(Clone, Copy)]
enum FlagGate {
    NativeResponses,
    CachePoints,
    ParallelTools,
    ExtendedCacheTtl,
}

fn flag_results(model_cost: &ParamMap, models: &[Option<String>], gate: FlagGate) -> HelperResult {
    let results: Vec<Value> = models
        .iter()
        .map(|model| {
            let answer = match gate {
                FlagGate::NativeResponses => {
                    supports_openai_responses(model.as_deref(), model_cost)
                }
                FlagGate::CachePoints => accepts_cache_points(model.as_deref(), model_cost),
                // the two strict gates take a name, and their cases never carry null
                FlagGate::ParallelTools => model
                    .as_deref()
                    .map_or(Ok(false), |name| supports_parallel_tools(name, model_cost)),
                FlagGate::ExtendedCacheTtl => model
                    .as_deref()
                    .map_or(Ok(false), |name| is_claude_4_5_on_bedrock(name, model_cost)),
            };
            answer.map_or_else(|_| json!({"raised": true}), |value| json!(value))
        })
        .collect();
    Ok(json!({ "results": results }))
}

fn strict_tools(model_cost: &ParamMap, models: &[String]) -> HelperResult {
    let fixture = Fixture::load(model_cost).map_err(failure)?;
    let resolution = fixture.resolution();
    let lookup = CachedModelInfo {
        fixture: &fixture,
        resolution: &resolution,
        capabilities: BTreeSet::new(),
    };
    let runtime = Builder::new_current_thread().build().map_err(failure)?;
    let mut results = Vec::with_capacity(models.len());
    for model in models {
        results.push(json!(runtime.block_on(converse_supports_strict_tools(
            model,
            &lookup,
            &fixture.packaged,
        ))));
    }
    Ok(json!({ "results": results }))
}

fn effort_ceiling(model_cost: &ParamMap, cases: &[EffortCase]) -> HelperResult {
    let fixture = Fixture::load(model_cost).map_err(failure)?;
    let resolution = fixture.resolution();
    let lookup = CachedModelInfo {
        fixture: &fixture,
        resolution: &resolution,
        capabilities: BTreeSet::new(),
    };
    let runtime = Builder::new_current_thread().build().map_err(failure)?;
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        let mut output_config = case.output_config.clone();
        let outcome = runtime.block_on(normalize_opus_effort(
            &case.model,
            &mut output_config,
            &lookup,
            &fixture.packaged,
        ));
        match outcome {
            Ok(()) => results.push(serde_json::to_value(output_config).map_err(failure)?),
            Err(_) => results.push(json!({"raised": true})),
        }
    }
    Ok(json!({ "results": results }))
}

fn tool_search(model_cost: &ParamMap, models: &[String]) -> HelperResult {
    let fixture = Fixture::load(model_cost).map_err(failure)?;
    let resolution = fixture.resolution();
    let dynamic = NoDynamicMetadata;
    let mut keys = ModelCostKeys::default();
    let catalogs = fixture.catalogs();
    let runtime = Builder::new_current_thread().build().map_err(failure)?;
    let mut results = Vec::with_capacity(models.len());
    for model in models {
        let mut environment = fixture.capability_environment(&resolution, &mut keys, &dynamic);
        results.push(json!(runtime.block_on(supports_tool_search(
            model,
            &mut environment,
            &catalogs,
        ))));
    }
    Ok(json!({ "results": results }))
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::MantleApiBase { region, cases } => mantle_api_base(region, cases),
        Input::NativeResponses { model_cost, models } => {
            flag_results(model_cost, models, FlagGate::NativeResponses)
        }
        Input::CachePointUnknownModel { model_cost, models } => {
            flag_results(model_cost, models, FlagGate::CachePoints)
        }
        Input::StrictToolsFallback { model_cost, models } => strict_tools(model_cost, models),
        Input::ToolSearch { model_cost, models } => tool_search(model_cost, models),
        Input::EffortCeiling { model_cost, cases } => effort_ceiling(model_cost, cases),
        Input::ParallelTools { model_cost, models } => {
            flag_results(model_cost, models, FlagGate::ParallelTools)
        }
        Input::ExtendedCacheTtl { model_cost, models } => {
            flag_results(model_cost, models, FlagGate::ExtendedCacheTtl)
        }
    }
}
