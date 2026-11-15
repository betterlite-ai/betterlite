use std::fmt::Display;

use betterlite_bedrock::message_options::OptionsSources;
use betterlite_bedrock::message_options::cache_ttl::remove_ttl_from_cache_control;
use betterlite_bedrock::message_options::effort::clamp_adaptive_effort;
use betterlite_bedrock::message_options::thinking::{
    ensure_clear_thinking, supports_extended_thinking,
};
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use serde_json::json;
use tokio::runtime::{Builder, Runtime};

use super::fixture::{CostLookup, Fixture, NoHfTransport};
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

// One helper call: which production function runs, and the case it reads.
#[derive(Clone, Copy)]
enum Ask<'case> {
    Thinking(&'case EnsureCase),
    Clamp(&'case ClampCase),
    Supports(&'case String),
}

// The case a helper call runs, so one loop builds the sources for all three operations.
trait AskCase {
    fn ask(&self) -> Ask<'_>;
}

impl AskCase for EnsureCase {
    fn ask(&self) -> Ask<'_> {
        Ask::Thinking(self)
    }
}

impl AskCase for ClampCase {
    fn ask(&self) -> Ask<'_> {
        Ask::Clamp(self)
    }
}

impl AskCase for String {
    fn ask(&self) -> Ask<'_> {
        Ask::Supports(self)
    }
}

fn answer(
    ask: Ask<'_>,
    sources: &mut OptionsSources<'_, '_, NoDynamicMetadata, NoHfTransport>,
    runtime: &Runtime,
    packaged: &ModelCatalog,
    lookup: &CostLookup<'_>,
) -> HelperResult {
    match ask {
        Ask::Thinking(case) => {
            let mut request = case.request.clone();
            let outcome =
                runtime.block_on(ensure_clear_thinking(&mut request, &case.model, sources));
            Ok(outcome.map_or_else(
                |_| json!({"raised": true}),
                |injected| json!({"injected": injected, "request": request}),
            ))
        }
        Ask::Clamp(case) => {
            let mut params = case.params.clone();
            let outcome = runtime.block_on(clamp_adaptive_effort(
                &case.model,
                &mut params,
                sources,
                lookup,
                packaged,
            ));
            Ok(match outcome {
                Ok(()) => serde_json::to_value(params).map_err(failure)?,
                Err(_) => json!({"raised": true}),
            })
        }
        Ask::Supports(model) => Ok(json!(
            runtime.block_on(supports_extended_thinking(model, sources))
        )),
    }
}

fn ask_cases<Case>(model_cost: &ParamMap, cases: &[Case]) -> HelperResult
where
    Case: AskCase,
{
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
        let mut environment = fixture.capability_environment(&resolution, &mut keys, &dynamic);
        let mut sources = OptionsSources {
            environment: &mut environment,
            catalogs: &catalogs,
            budgets: &budgets,
        };
        results.push(answer(
            case.ask(),
            &mut sources,
            &runtime,
            &fixture.packaged,
            &lookup,
        )?);
    }
    Ok(json!({ "results": results }))
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::RemoveTtl { model_cost, cases } => remove_ttl(model_cost, cases),
        Input::EnsureThinking { model_cost, cases } => ask_cases(model_cost, cases),
        Input::SupportsExtendedThinking { model_cost, models } => ask_cases(model_cost, models),
        Input::ClampEffort { model_cost, cases } => ask_cases(model_cost, cases),
    }
}
