use betterlite_core::api::responses_usage;
use betterlite_core::api::responses_usage::UsageInput;
use betterlite_openai::api::responses_objects::ResponseApiUsage;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> HelperResult {
        let raw_usage = input.get("usage");
        let normalized = if input
            .get("typed_object")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
            && let Some(raw_value) = raw_usage.filter(|usage| usage.is_object())
        {
            let response = serde_json::from_value::<ResponseApiUsage>(raw_value.clone())
                .map_err(|error| invalid_usage(error.to_string()))?;
            responses_usage::normalize_and_project(UsageInput::Response(&response))
                .map_err(|error| invalid_usage(error.to_string()))?
        } else {
            let usage = raw_usage.map_or(UsageInput::Absent, UsageInput::Mapping);
            responses_usage::normalize_and_project(usage)
                .map_err(|error| invalid_usage(error.to_string()))?
        };
        Ok(serde_json::json!({"usage": normalized}))
    }
}

fn invalid_usage(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
