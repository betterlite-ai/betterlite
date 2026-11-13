mod records;
mod watch;

use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_core::api::messages_native_transform::request::TransformRequest;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use crate::helpers::{Helper, HelperError, HelperResult};
use records::Records;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    model_cost: IndexMap<String, Value>,
    #[serde(default)]
    packaged: Value,
    #[serde(default)]
    budgets: Budgets,
    calls: Vec<Call>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Budgets {
    minimal: i64,
    low: i64,
    medium: i64,
    high: i64,
    xhigh: i64,
    max: i64,
}

impl Default for Budgets {
    fn default() -> Self {
        let defaults = Settings::default();
        Self {
            minimal: defaults.minimal,
            low: defaults.low,
            medium: defaults.medium,
            high: defaults.high,
            xhigh: defaults.xhigh,
            max: defaults.max,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) custom_llm_provider: Option<String>,
    #[serde(default)]
    pub(crate) messages: Vec<Json>,
    pub(crate) optional_params: Object,
    #[serde(default)]
    pub(crate) strips_billing_metadata: bool,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let records = Records::load(&input.model_cost, &input.packaged, settings(&input.budgets))?;
        let runtime = runtime()?;
        let mut calls = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            calls.push(answer(&records, call, &runtime)?);
        }
        Ok(json!({ "calls": calls }))
    }
}

fn settings(budgets: &Budgets) -> Settings {
    Settings {
        minimal: budgets.minimal,
        low: budgets.low,
        medium: budgets.medium,
        high: budgets.high,
        xhigh: budgets.xhigh,
        max: budgets.max,
        bedrock_min: Settings::default().bedrock_min,
    }
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .build()
        .map_err(|source| failure(source.to_string()))
}

fn answer(records: &Records, call: &Call, runtime: &Runtime) -> Result<Value, Box<HelperError>> {
    let provider = call.custom_llm_provider.as_deref().unwrap_or("anthropic");
    let request = TransformRequest {
        model: &call.model,
        messages: call.messages.clone(),
        optional_params: call.optional_params.clone(),
        custom_llm_provider: provider,
        strips_billing_metadata: call.strips_billing_metadata,
    };
    let (guard, watch) = watch::install();
    let outcome = records.call(request, runtime);
    drop(guard);
    let (warnings, debug) = watch.take();
    match outcome {
        Ok(body) => Ok(json!({
            "request": serde_json::to_value(body).map_err(|source| failure(source.to_string()))?,
            "warnings": warnings,
            "debug": debug,
        })),
        Err(error) => Err(Box::new(HelperError {
            class: "AnthropicError".to_owned(),
            status: Some(error.status_code()),
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
            message: error.to_string(),
        })),
    }
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
