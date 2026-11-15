use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;

use betterlite_bedrock::converse_options::budget::clamp_thinking_budget_tokens;
use betterlite_bedrock::converse_options::config::{
    CUSTOM_LLM_PROVIDER, class_attributes, config_blocks, get_config,
};
use betterlite_bedrock::converse_options::error::ConverseOptionsError;
use betterlite_bedrock::converse_options::family::{
    is_deepseek_model, is_nova_2_model, is_openai_gpt_reasoning_model,
    model_rejects_reasoning_effort_param, requires_min_max_tokens,
};
use betterlite_bedrock::converse_options::reasoning::{
    ReasoningSources, handle_reasoning_effort_parameter,
};
use betterlite_bedrock::converse_options::supported::{
    auto_tool_choice, model_accepts_anthropic_thinking_param, supported_openai_params,
    supports_sampling_params,
};
use betterlite_bedrock::converse_options::web_search::map_web_search_options;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use super::fixture::{CachedModelInfo, Fixture};
use super::input::{
    CatalogModels, CatalogPairs, ClampCase, ClassDefaults, HandleCase, Input, ModelPair,
};
use crate::helpers::{HelperError, HelperResult};

// The budget section only reads the environment, so the case's variables reach it through the loader.
const FIXTURE_CONFIG: &str =
    "model_list:\n  - model_name: fixture\n    litellm_params: {model: bedrock/fixture}\n";

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "BedrockConverseOptions".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

// Every rejection carries the same 400 LiteLLM's BadRequestError does; an unhashable effort carries none.
fn converse_failure(error: &ConverseOptionsError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "BedrockConverseOptions".to_owned(),
        status: error.status_code(),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn budgets(environment: &BTreeMap<String, String>) -> Result<Settings, Box<HelperError>> {
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load(FIXTURE_CONFIG)
    .map_err(failure)?;
    Ok(config.sections.anthropic_capability_catalog)
}

fn current_thread_runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread().build().map_err(failure)
}

fn flag_results<Gate>(models: &[String], gate: Gate) -> Value
where
    Gate: Fn(&str) -> bool,
{
    let results: Vec<Value> = models.iter().map(|model| json!(gate(model))).collect();
    json!({ "results": results })
}

fn handle(case: &HandleCase) -> HelperResult {
    let fixture = Fixture::load(&case.model_cost).map_err(failure)?;
    let budget_settings = budgets(&case.environment)?;
    let resolution = fixture.resolution();
    let catalogs = fixture.catalogs();
    let lookup = CachedModelInfo {
        fixture: &fixture,
        resolution: &resolution,
        capabilities: BTreeSet::new(),
    };
    let mut optional_params = case.optional_params.clone();
    let runtime = current_thread_runtime()?;
    let mut keys = ModelCostKeys::default();
    let dynamic = NoDynamicMetadata;
    runtime
        .block_on(async {
            let mut environment = fixture.capability_environment(&resolution, &mut keys, &dynamic);
            let sources = ReasoningSources {
                catalogs: &catalogs,
                budgets: &budget_settings,
                lookup: &lookup,
                packaged: &fixture.packaged,
            };
            handle_reasoning_effort_parameter(
                &case.model,
                &case.effort,
                &mut optional_params,
                &mut environment,
                &sources,
            )
            .await
        })
        .map_err(|error| converse_failure(&error))?;
    Ok(json!({ "params": serde_json::to_value(optional_params).map_err(failure)? }))
}

fn deepseek_flags(cases: &[ModelPair]) -> Value {
    let results: Vec<Value> = cases
        .iter()
        .map(|case| {
            json!({
                "deepseek": is_deepseek_model(&case.model, &case.base_model),
                "rejects": model_rejects_reasoning_effort_param(&case.model, &case.base_model),
            })
        })
        .collect();
    json!({ "results": results })
}

fn thinking_accepts(case: &CatalogPairs) -> HelperResult {
    let fixture = Fixture::load(&case.model_cost).map_err(failure)?;
    let resolution = fixture.resolution();
    let runtime = current_thread_runtime()?;
    let mut keys = ModelCostKeys::default();
    let dynamic = NoDynamicMetadata;
    let mut results = Vec::with_capacity(case.cases.len());
    for pair in &case.cases {
        let mut environment = fixture.capability_environment(&resolution, &mut keys, &dynamic);
        results.push(json!(runtime.block_on(
            model_accepts_anthropic_thinking_param(&pair.model, &pair.base_model, &mut environment),
        )));
    }
    Ok(json!({ "results": results }))
}

fn sampling(case: &CatalogModels) -> HelperResult {
    let fixture = Fixture::load(&case.model_cost).map_err(failure)?;
    let catalogs = fixture.catalogs();
    Ok(flag_results(&case.models, |model| {
        supports_sampling_params(model, &catalogs)
    }))
}

fn supported_params(case: &CatalogModels) -> HelperResult {
    let fixture = Fixture::load(&case.model_cost).map_err(failure)?;
    let resolution = fixture.resolution();
    let catalogs = fixture.catalogs();
    let runtime = current_thread_runtime()?;
    let mut keys = ModelCostKeys::default();
    let dynamic = NoDynamicMetadata;
    let mut results = Vec::with_capacity(case.models.len());
    for model in &case.models {
        let mut environment = fixture.capability_environment(&resolution, &mut keys, &dynamic);
        results.push(json!(runtime.block_on(supported_openai_params(
            model,
            &mut environment,
            &catalogs,
        ))));
    }
    Ok(json!({ "results": results }))
}

fn clamp(case: &ClampCase) -> HelperResult {
    let minimum = budgets(&case.environment)?.bedrock_min;
    let mut results = Vec::with_capacity(case.cases.len());
    for values in &case.cases {
        let mut optional_params = values.optional_params.clone();
        clamp_thinking_budget_tokens(&mut optional_params, minimum);
        results.push(serde_json::to_value(optional_params).map_err(failure)?);
    }
    Ok(json!({ "results": results }))
}

fn web_search(models: &[String]) -> Value {
    let results: Vec<Value> = models
        .iter()
        .map(|model| json!(map_web_search_options(model)))
        .collect();
    json!({ "results": results })
}

fn class_defaults(case: &ClassDefaults) -> HelperResult {
    let attributes = class_attributes(&case.arguments);
    Ok(json!({
        "config": serde_json::to_value(get_config(&attributes)).map_err(failure)?,
        "custom_llm_provider": CUSTOM_LLM_PROVIDER,
        "config_blocks": serde_json::to_value(config_blocks()).map_err(failure)?,
    }))
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::Nova2Models(case) => Ok(flag_results(&case.models, is_nova_2_model)),
        Input::GptModels(case) => Ok(flag_results(&case.models, is_openai_gpt_reasoning_model)),
        Input::MinMaxTokens(case) => Ok(flag_results(&case.models, requires_min_max_tokens)),
        Input::Handle(case) => handle(case),
        Input::DeepseekFlags(case) => Ok(deepseek_flags(&case.cases)),
        Input::ThinkingAccepts(case) => thinking_accepts(case),
        Input::Sampling(case) => sampling(case),
        Input::SupportedParams(case) => supported_params(case),
        Input::Clamp(case) => clamp(case),
        Input::WebSearch(case) => Ok(web_search(&case.models)),
        Input::ClassDefaults(case) => class_defaults(case),
        Input::AutoToolChoice => {
            Ok(json!({ "block": serde_json::to_value(auto_tool_choice()).map_err(failure)? }))
        }
    }
}
