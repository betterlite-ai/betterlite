use std::fmt::Display;

use betterlite_bedrock::message_options::OptionsSources;
use betterlite_bedrock::message_options::cache_ttl::remove_ttl_from_cache_control;
use betterlite_bedrock::message_options::effort::clamp_adaptive_effort;
use betterlite_bedrock::message_options::thinking::{
    ensure_clear_thinking, supports_extended_thinking,
};
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use super::fixture::{CostLookup, Fixture};
use super::input::{ClampCase, EnsureCase, Input, TtlCase};
use crate::helpers::{HelperError, HelperResult};

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "BedrockMessageOptions".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn remove_ttl(model_cost: &ParamMap, cases: &[TtlCase]) -> HelperResult {
    let mut requests = Vec::with_capacity(cases.len());
    for case in cases {
        let mut request = case.request.clone();
        let outcome =
            remove_ttl_from_cache_control(&mut request, case.model.as_deref(), model_cost);
        match outcome {
            Ok(()) => requests.push(serde_json::to_value(request).map_err(failure)?),
            Err(_) => requests.push(json!({"raised": true})),
        }
    }
    Ok(json!({ "requests": requests }))
}

fn ensure_thinking(model_cost: &ParamMap, cases: &[EnsureCase]) -> HelperResult {
    let fixture = Fixture::load(model_cost).map_err(failure)?;
    let resolution = fixture.resolution();
    let dynamic = NoDynamicMetadata;
    let mut keys = ModelCostKeys::default();
    let budgets = Settings::default();
    let catalogs = fixture.catalogs();
    let runtime = Builder::new_current_thread().build().map_err(failure)?;
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        let mut request = case.request.clone();
        let mut environment = fixture.capability_environment(&resolution, &mut keys, &dynamic);
        let mut sources = OptionsSources {
            environment: &mut environment,
            catalogs: &catalogs,
            budgets: &budgets,
        };
        let outcome = runtime.block_on(ensure_clear_thinking(
            &mut request,
            &case.model,
            &mut sources,
        ));
        match outcome {
            Ok(injected) => results.push(json!({"injected": injected, "request": request})),
            Err(_) => results.push(json!({"raised": true})),
        }
    }
    Ok(json!({ "results": results }))
}

fn extended_thinking(model_cost: &ParamMap, models: &[String]) -> HelperResult {
    let fixture = Fixture::load(model_cost).map_err(failure)?;
    let resolution = fixture.resolution();
    let dynamic = NoDynamicMetadata;
    let mut keys = ModelCostKeys::default();
    let budgets = Settings::default();
    let catalogs = fixture.catalogs();
    let runtime = Builder::new_current_thread().build().map_err(failure)?;
    let mut results: Vec<Value> = Vec::with_capacity(models.len());
    for model in models {
        let mut environment = fixture.capability_environment(&resolution, &mut keys, &dynamic);
        let mut sources = OptionsSources {
            environment: &mut environment,
            catalogs: &catalogs,
            budgets: &budgets,
        };
        results.push(json!(
            runtime.block_on(supports_extended_thinking(model, &mut sources))
        ));
    }
    Ok(json!({ "results": results }))
}

fn clamp_effort(model_cost: &ParamMap, cases: &[ClampCase]) -> HelperResult {
    let fixture = Fixture::load(model_cost).map_err(failure)?;
    let resolution = fixture.resolution();
    let lookup = CostLookup {
        fixture: &fixture,
        resolution: &resolution,
    };
    let dynamic = NoDynamicMetadata;
    let mut keys = ModelCostKeys::default();
    let budgets = Settings::default();
    let catalogs = fixture.catalogs();
    let runtime = Builder::new_current_thread().build().map_err(failure)?;
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        let mut params = case.params.clone();
        let mut environment = fixture.capability_environment(&resolution, &mut keys, &dynamic);
        let mut sources = OptionsSources {
            environment: &mut environment,
            catalogs: &catalogs,
            budgets: &budgets,
        };
        let outcome = runtime.block_on(clamp_adaptive_effort(
            &case.model,
            &mut params,
            &mut sources,
            &lookup,
            &fixture.packaged,
        ));
        match outcome {
            Ok(()) => results.push(serde_json::to_value(params).map_err(failure)?),
            Err(_) => results.push(json!({"raised": true})),
        }
    }
    Ok(json!({ "results": results }))
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::RemoveTtl { model_cost, cases } => remove_ttl(model_cost, cases),
        Input::EnsureThinking { model_cost, cases } => ensure_thinking(model_cost, cases),
        Input::SupportsExtendedThinking { model_cost, models } => {
            extended_thinking(model_cost, models)
        }
        Input::ClampEffort { model_cost, cases } => clamp_effort(model_cost, cases),
    }
}
