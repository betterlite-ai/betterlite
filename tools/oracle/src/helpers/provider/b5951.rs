use betterlite_prompts::arguments::json::Json;
use betterlite_provider::sap_models::SapModelError;
use betterlite_provider::sap_models::validate_chat_completion_tool;
use betterlite_provider::sap_models::validate_masking_module;
use betterlite_provider::sap_models::validate_orchestration_request;
use betterlite_provider::sap_models::validate_response_format;
use betterlite_provider::sap_models::validate_sap_message;

use crate::helpers::HelperError;

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model: String,
    data: Json,
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("SapModelError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn value(result: Result<Json, SapModelError>) -> crate::helpers::HelperResult {
    let model = result.map_err(|error| failure(error.to_string()))?;
    let converted = serde_json::to_value(&model);
    converted.map_err(|error| failure(error.to_string()))
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input.model.as_str() {
            "orchestration_request" => value(validate_orchestration_request(&input.data)),
            "sap_message" => value(validate_sap_message(&input.data)),
            "chat_completion_tool" => value(validate_chat_completion_tool(&input.data)),
            "response_format" => value(validate_response_format(&input.data)),
            "masking_module" => value(validate_masking_module(&input.data)),
            other => Err(failure(format!("unknown SAP model {other}"))),
        }
    }
}
