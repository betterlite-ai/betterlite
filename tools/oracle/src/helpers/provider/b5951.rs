use betterlite_prompts::arguments::json::Json;
use betterlite_provider::sap_models::validate_chat_completion_tool;
use betterlite_provider::sap_models::validate_masking_module;
use betterlite_provider::sap_models::validate_orchestration_request;
use betterlite_provider::sap_models::validate_response_format;
use betterlite_provider::sap_models::validate_sap_message;

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    model: String,
    data: Json,
}

// A case holds one or more calls and answers one {ok, value} per call, so a single case can compare both a dump and a rejection.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

fn attempt(call: &Call) -> Option<serde_json::Value> {
    let model = match call.model.as_str() {
        "orchestration_request" => validate_orchestration_request(&call.data),
        "sap_message" => validate_sap_message(&call.data),
        "chat_completion_tool" => validate_chat_completion_tool(&call.data),
        "response_format" => validate_response_format(&call.data),
        "masking_module" => validate_masking_module(&call.data),
        _ => return None,
    }
    .ok()?;
    serde_json::to_value(&model).ok()
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let outcomes: Vec<serde_json::Value> = input
            .calls
            .iter()
            .map(|call| {
                attempt(call).map_or_else(
                    || serde_json::json!({"ok": false}),
                    |value| serde_json::json!({"ok": true, "value": value}),
                )
            })
            .collect();
        Ok(serde_json::Value::Array(outcomes))
    }
}
