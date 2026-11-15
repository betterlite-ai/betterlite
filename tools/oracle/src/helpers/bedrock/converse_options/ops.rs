use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fmt::Display;
use std::future::{Future, ready};

use betterlite_bedrock::bedrock_model_capabilities::ModelInfoLookup;
use betterlite_bedrock::converse_options::budget::clamp_thinking_budget_tokens;
use betterlite_bedrock::converse_options::config::{
    CUSTOM_LLM_PROVIDER, class_attributes, config_blocks, get_config,
};
use betterlite_bedrock::converse_options::error::ConverseOptionsError;
use betterlite_bedrock::converse_options::family::{
    is_nova_2_model, is_openai_gpt_reasoning_model, requires_min_max_tokens,
};
use betterlite_bedrock::converse_options::reasoning::{ReasoningSources, handle_reasoning_effort};
use betterlite_bedrock::converse_options::supported::{
    auto_tool_choice, supported_openai_params, supports_sampling_params,
};
use betterlite_bedrock::converse_options::web_search::map_web_search_options;
use betterlite_bedrock::model_routes::{ParamMap, ParamValue};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::record::ModelInfo;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use super::fixture::Fixture;
use super::input::{Flag, Input};
use crate::helpers::{HelperError, HelperResult};

// The budget section only reads the environment, so the case's variables reach it through the loader.
const FIXTURE_CONFIG: &str =
    "model_list:\n  - model_name: fixture\n    litellm_params: {model: bedrock/fixture}\n";

fn helper_error<E>(error: E, status: Option<u16>) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "BedrockConverseOptions".to_owned(),
        status,
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
    .map_err(|error| helper_error(error, None))?;
    Ok(config.sections.anthropic_capability_catalog)
}

fn current_thread_runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .build()
        .map_err(|error| helper_error(error, None))
}

// The ceiling fields are all normalize_opus_effort reads; the case map supplies them directly.
struct CaseLookup<'case> {
    entries: &'case ParamMap,
}

impl ModelInfoLookup for CaseLookup<'_> {
    type Error = Infallible;

    fn bedrock_model_info(
        &self,
        model: &str,
    ) -> impl Future<Output = Result<ModelInfo, Infallible>> + Send {
        let mut fields = vec![("key".to_owned(), Json::Text(model.to_owned()))];
        if let Some(ParamValue::Mapping(entry)) = self.entries.get(model)
            && let Some(ParamValue::Text(ceiling)) =
                entry.get("bedrock_output_config_effort_ceiling")
        {
            fields.push((
                "bedrock_output_config_effort_ceiling".to_owned(),
                Json::Text(ceiling.clone()),
            ));
        }
        ready(Ok(ModelInfo {
            fields: fields.into_iter().collect(),
        }))
    }
}

fn flags(kind: &Flag, models: &[String], model_cost: &ParamMap) -> HelperResult {
    let fixture = Fixture::load(model_cost).map_err(|error| helper_error(error, None))?;
    let catalogs = fixture.catalogs();
    let results: Vec<Value> = models
        .iter()
        .map(|model| match kind {
            Flag::Nova2 => json!(is_nova_2_model(model)),
            Flag::Gpt => json!(is_openai_gpt_reasoning_model(model)),
            Flag::MinMax => json!(requires_min_max_tokens(model)),
            Flag::Sampling => json!(supports_sampling_params(model, &catalogs)),
            Flag::WebSearch => json!(map_web_search_options(model)),
        })
        .collect();
    Ok(json!({ "results": results }))
}

fn handle(
    model_cost: &ParamMap,
    environment: &BTreeMap<String, String>,
    model: &str,
    effort: &str,
    optional_params: &ParamMap,
) -> HelperResult {
    let fixture = Fixture::load(model_cost).map_err(|error| helper_error(error, None))?;
    let budget_settings = budgets(environment)?;
    let resolution = fixture.resolution();
    let catalogs = fixture.catalogs();
    let lookup = CaseLookup {
        entries: model_cost,
    };
    let mut params = optional_params.clone();
    let runtime = current_thread_runtime()?;
    let mut keys = ModelCostKeys::default();
    let dynamic = NoDynamicMetadata;
    runtime
        .block_on(async {
            let mut capability = fixture.capability_environment(&resolution, &mut keys, &dynamic);
            let sources = ReasoningSources {
                catalogs: &catalogs,
                budgets: &budget_settings,
                lookup: &lookup,
                packaged: &fixture.packaged,
            };
            handle_reasoning_effort(model, effort, &mut params, &mut capability, &sources).await
        })
        .map_err(|error| helper_error(&error, error.status_code()))?;
    Ok(
        json!({ "params": serde_json::to_value(params).map_err(|error| helper_error(error, None))? }),
    )
}

fn params(model_cost: &ParamMap, models: &[String]) -> HelperResult {
    let fixture = Fixture::load(model_cost).map_err(|error| helper_error(error, None))?;
    let resolution = fixture.resolution();
    let catalogs = fixture.catalogs();
    let runtime = current_thread_runtime()?;
    let mut keys = ModelCostKeys::default();
    let dynamic = NoDynamicMetadata;
    let mut results = Vec::with_capacity(models.len());
    for model in models {
        let mut capability = fixture.capability_environment(&resolution, &mut keys, &dynamic);
        results.push(json!(runtime.block_on(supported_openai_params(
            model,
            &mut capability,
            &catalogs,
        ))));
    }
    Ok(json!({
        "results": results,
        "auto": serde_json::to_value(auto_tool_choice()).map_err(|error| helper_error(error, None))?,
    }))
}

fn clamp(environment: &BTreeMap<String, String>, cases: &[ParamMap]) -> HelperResult {
    let minimum = budgets(environment)?.bedrock_min;
    let mut results = Vec::with_capacity(cases.len());
    for values in cases {
        let mut optional_params = values.clone();
        clamp_thinking_budget_tokens(&mut optional_params, minimum);
        results.push(
            serde_json::to_value(optional_params).map_err(|error| helper_error(error, None))?,
        );
    }
    Ok(json!({ "results": results }))
}

fn defaults(arguments: &ParamMap) -> HelperResult {
    let attributes = class_attributes(arguments);
    Ok(json!({
        "config": serde_json::to_value(get_config(&attributes)).map_err(|error| helper_error(error, None))?,
        "custom_llm_provider": CUSTOM_LLM_PROVIDER,
        "config_blocks": serde_json::to_value(config_blocks()).map_err(|error| helper_error(error, None))?,
    }))
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::Flags {
            kind,
            model_cost,
            models,
        } => flags(kind, models, model_cost),
        Input::Handle {
            model_cost,
            environment,
            model,
            effort,
            optional_params,
        } => handle(model_cost, environment, model, effort, optional_params),
        Input::Params { model_cost, models } => params(model_cost, models),
        Input::Clamp { environment, cases } => clamp(environment, cases),
        Input::Defaults { arguments } => defaults(arguments),
    }
}
