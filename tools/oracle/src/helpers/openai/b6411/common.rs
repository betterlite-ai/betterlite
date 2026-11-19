use betterlite_openai::sail_common::error::SailError;

// ast-grep-ignore: no-long-names
use betterlite_openai::sail_common::completion_window_for_service_tier as window_for;

// ast-grep-ignore: no-long-names
use betterlite_openai::sail_common::responses_params_with_completion_window as params_with_window;

use betterlite_openai::sail_common::{
    CompletionWindow, SailSettings, chat_request_for_sail, extra_body_for_sail,
};
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::helper::HelperResult;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WindowSteps {
    model: String,
    steps: Vec<WindowStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WindowStep {
    service_tier: Json,
    drop_params: bool,
    global_drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChatRequestSteps {
    model: String,
    steps: Vec<ChatRequestStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChatRequestStep {
    request: Object,
    drop_params: bool,
    global_drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExtraBodySteps {
    model: String,
    steps: Vec<ExtraBodyStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtraBodyStep {
    extra_body: Object,
    request_metadata: Json,
    drop_params: bool,
    global_drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponsesParamsSteps {
    model: String,
    steps: Vec<ResponsesParamsStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponsesParamsStep {
    params: Object,
    drop_params: bool,
    global_drop_params: bool,
}

pub(super) fn windows(steps: &WindowSteps) -> Value {
    let mut records = Vec::with_capacity(steps.steps.len());
    for step in &steps.steps {
        let settings = SailSettings {
            global_drop_params: step.global_drop_params,
        };
        let record = match window_for(
            Some(&step.service_tier),
            &steps.model,
            step.drop_params,
            &settings,
        ) {
            Ok(window) => {
                json!({"ok": true, "value": window.map(CompletionWindow::as_str), "status": null})
            }
            Err(error) => json!({"ok": false, "value": null, "status": error.status_code()}),
        };
        records.push(record);
    }
    Value::Array(records)
}

pub(super) fn chat_requests(steps: &ChatRequestSteps) -> HelperResult {
    let mut records = Vec::with_capacity(steps.steps.len());
    for step in &steps.steps {
        let settings = SailSettings {
            global_drop_params: step.global_drop_params,
        };
        let rewritten =
            chat_request_for_sail(&step.request, &steps.model, step.drop_params, &settings);
        records.push(body_record(rewritten)?);
    }
    Ok(Value::Array(records))
}

pub(super) fn extra_bodies(steps: &ExtraBodySteps) -> HelperResult {
    let mut records = Vec::with_capacity(steps.steps.len());
    for step in &steps.steps {
        let settings = SailSettings {
            global_drop_params: step.global_drop_params,
        };
        let rewritten = extra_body_for_sail(
            &step.extra_body,
            Some(&step.request_metadata),
            &steps.model,
            step.drop_params,
            &settings,
        );
        records.push(body_record(rewritten)?);
    }
    Ok(Value::Array(records))
}

pub(super) fn responses_params(steps: &ResponsesParamsSteps) -> HelperResult {
    let mut records = Vec::with_capacity(steps.steps.len());
    for step in &steps.steps {
        let settings = SailSettings {
            global_drop_params: step.global_drop_params,
        };
        let rewritten = params_with_window(&step.params, &steps.model, step.drop_params, &settings);
        records.push(body_record(rewritten)?);
    }
    Ok(Value::Array(records))
}

fn body_record(result: Result<Object, SailError>) -> HelperResult {
    match result {
        Ok(rewritten) => {
            let value = serde_json::to_value(rewritten)?;
            Ok(json!({"ok": true, "value": value, "status": null}))
        }
        Err(error) => Ok(json!({"ok": false, "value": null, "status": error.status_code()})),
    }
}
