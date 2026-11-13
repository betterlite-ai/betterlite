mod records;
mod watch;

use std::sync::PoisonError;

use betterlite_core::api::messages_native_transform::request::TransformRequest;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use crate::helpers::{Helper, HelperError, HelperResult};
use records::Records;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    model_cost: IndexMap<String, Value>,
    #[serde(default)]
    packaged: IndexMap<String, Value>,
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    model: String,
    #[serde(default)]
    custom_llm_provider: Option<String>,
    #[serde(default)]
    messages: Vec<Json>,
    optional_params: Object,
    #[serde(default)]
    strips_billing_metadata: bool,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let records = Records::load(&input.model_cost, &input.packaged)?;
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|source| invalid(source.to_string()))?;
        let mut calls = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            calls.push(answer(&records, call, &runtime)?);
        }
        Ok(json!({ "calls": calls }))
    }
}

fn answer(records: &Records, call: &Call, runtime: &Runtime) -> Result<Value, Box<HelperError>> {
    let request = TransformRequest {
        model: &call.model,
        messages: call.messages.clone(),
        optional_params: call.optional_params.clone(),
        custom_llm_provider: call.custom_llm_provider.as_deref().unwrap_or("anthropic"),
        strips_billing_metadata: call.strips_billing_metadata,
    };
    let (guard, watch) = watch::install();
    let outcome = records.call(request, runtime);
    drop(guard);
    let body = outcome.map_err(|error| {
        Box::new(HelperError {
            class: "AnthropicError".to_owned(),
            status: Some(error.status_code()),
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
            message: error.to_string(),
        })
    })?;
    let sent = serde_json::to_value(body).map_err(|source| invalid(source.to_string()))?;
    let notes = watch.notes.lock().unwrap_or_else(PoisonError::into_inner);
    Ok(json!({
        "request": sent,
        "warnings": notes.warnings,
        "debug": notes.transformation,
    }))
}

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
