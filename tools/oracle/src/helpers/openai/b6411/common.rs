use betterlite_openai::sail_common::error::SailError;

// ast-grep-ignore: no-long-names
use betterlite_openai::sail_common::completion_window_for_service_tier as window_for;

use betterlite_openai::sail_common::{CompletionWindow, SailSettings};
use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};
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
    pub(super) model: String,
    pub(super) steps: Vec<ChatRequestStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ChatRequestStep {
    pub(super) request: Object,
    pub(super) drop_params: bool,
    pub(super) global_drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExtraBodySteps {
    pub(super) model: String,
    pub(super) steps: Vec<ExtraBodyStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExtraBodyStep {
    pub(super) extra_body: Object,
    pub(super) request_metadata: Json,
    pub(super) drop_params: bool,
    pub(super) global_drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponsesParamsSteps {
    pub(super) model: String,
    pub(super) steps: Vec<ResponsesParamsStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResponsesParamsStep {
    pub(super) params: Object,
    pub(super) drop_params: bool,
    pub(super) global_drop_params: bool,
}

pub(super) fn windows(steps: &WindowSteps) -> HelperResult {
    records(steps.steps.iter().map(|step| {
        let settings = SailSettings {
            global_drop_params: step.global_drop_params,
        };
        window_for(
            Some(&step.service_tier),
            &steps.model,
            step.drop_params,
            &settings,
        )
        .map(|window| window.map(CompletionWindow::as_str))
    }))
}

pub(super) fn records<I, T>(rewrites: I) -> HelperResult
where
    I: IntoIterator<Item = Result<T, SailError>>,
    T: Serialize,
{
    let mut records = Vec::new();
    for rewritten in rewrites {
        let record = match rewritten {
            Ok(value) => {
                let body = serde_json::to_value(value)?;
                json!({"ok": true, "value": body, "status": null})
            }
            Err(error) => json!({"ok": false, "value": null, "status": error.status_code()}),
        };
        records.push(record);
    }
    Ok(Value::Array(records))
}
