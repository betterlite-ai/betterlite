pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let raw_usage = input.get("usage");
        let typed_value = if input
            .get("typed_object")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
        {
            raw_usage
                .cloned()
                .map(
                    serde_json::from_value::<
                        betterlite_openai::api::responses_objects::ResponseApiUsage,
                    >,
                )
                .transpose()
                .map_err(|error| invalid_usage(error.to_string()))?
                .map(serde_json::to_value)
                .transpose()
                .map_err(|error| invalid_usage(error.to_string()))?
        } else {
            None
        };
        let normalized_input = typed_value.as_ref().or(raw_usage);
        let normalized = betterlite_core::api::responses_usage::transform_value(normalized_input)
            .map_err(|error| invalid_usage(error.to_string()))?;
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
