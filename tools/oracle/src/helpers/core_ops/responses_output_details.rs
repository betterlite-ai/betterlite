pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let images: Option<Vec<betterlite_core::api::responses_output_details::ImageItem>> = input
            .get("images")
            .map(|images| serde_json::from_value(images.clone()))
            .transpose()
            .map_err(invalid_details)?;
        let result = betterlite_core::api::responses_output_details::image_results(
            images.as_deref(),
            input
                .get("finish_reason")
                .and_then(serde_json::Value::as_str),
            || "oracle-id".to_owned(),
        );
        let status = betterlite_core::api::responses_output_details::image_finish_status(
            input
                .get("finish_reason")
                .and_then(serde_json::Value::as_str),
        );
        let annotations: Option<
            Vec<betterlite_core::api::responses_output_details::ChatAnnotation>,
        > = input
            .get("annotations")
            .filter(|annotations| !annotations.is_null())
            .map(|annotations| serde_json::from_value(annotations.clone()))
            .transpose()
            .map_err(invalid_details)?;
        let annotations =
            betterlite_core::api::responses_output_details::citation_fields(annotations.as_deref());
        let usage = input.get("usage").map_or(
            betterlite_core::api::responses_usage::UsageInput::Absent,
            betterlite_core::api::responses_usage::UsageInput::Mapping,
        );
        let normalized =
            betterlite_core::api::responses_usage::normalize_usage(usage).map_err(|error| {
                Box::new(crate::helpers::HelperError {
                    class: "InvalidInput".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                })
            })?;
        let mut numeric_extras = std::collections::BTreeMap::new();
        if let Some(cost) = normalized.cost {
            numeric_extras.insert(
                "cost".to_owned(),
                betterlite_models::usage_sum::UsageNumber::Decimal(cost),
            );
        }
        let typed_usage = betterlite_models::usage_sum::Usage {
            prompt_tokens: normalized.prompt_tokens,
            completion_tokens: normalized.completion_tokens,
            total_tokens: normalized.total_tokens,
            prompt_tokens_details: normalized.prompt_tokens_details,
            completion_tokens_details: normalized.completion_tokens_details,
            numeric_extras,
        };
        let projected =
            betterlite_core::api::responses_output_details::usage_details(Some(&typed_usage));
        Ok(
            serde_json::json!({"images":result,"status":status,"annotations":annotations,"usage":projected}),
        )
    }
}

fn invalid_details(error: serde_json::Error) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
