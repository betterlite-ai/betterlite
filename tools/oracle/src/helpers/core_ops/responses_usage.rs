pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let raw_usage = input.get("usage");
        let normalized = if input
            .get("typed_object")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
            && let Some(raw_value) = raw_usage.filter(|usage| usage.is_object())
        {
            let response = serde_json::from_value::<
                betterlite_openai::api::responses_objects::ResponseApiUsage,
            >(raw_value.clone())
            .map_err(|error| invalid_usage(error.to_string()))?;
            betterlite_core::api::responses_usage::normalize_and_project(
                betterlite_core::api::responses_usage::UsageInput::Response(&response),
            )
            .map_err(|error| invalid_usage(error.to_string()))?
        } else {
            let usage = raw_usage.map_or(
                betterlite_core::api::responses_usage::UsageInput::Absent,
                betterlite_core::api::responses_usage::UsageInput::Mapping,
            );
            betterlite_core::api::responses_usage::normalize_and_project(usage)
                .map_err(|error| invalid_usage(error.to_string()))?
        };
        Ok(serde_json::json!({"usage": normalized}))
    }
}

fn invalid_usage(message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
