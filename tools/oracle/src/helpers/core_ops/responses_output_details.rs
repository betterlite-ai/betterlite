use betterlite_core::api::responses_output_details::ChatAnnotation;
use betterlite_openai::api::responses_shapes::OutputAnnotation;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let result = betterlite_core::api::responses_output_details::image_results(
            parsed_images(input)?.as_deref(),
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
        let projected_annotations = parsed_annotations(input)?;
        let projected_usage = projected_usage(input)?;
        Ok(
            serde_json::json!({"images":result,"status":status,"annotations":projected_annotations,"usage":projected_usage}),
        )
    }
}

fn parsed_images(
    input: &serde_json::Value,
) -> Result<
    Option<Vec<betterlite_core::api::responses_output_details::ImageItem>>,
    Box<crate::helpers::HelperError>,
> {
    input
        .get("images")
        .map(|images| serde_json::from_value(images.clone()))
        .transpose()
        .map_err(|error| invalid_details(&error))
}

fn parsed_annotations(
    input: &serde_json::Value,
) -> Result<Vec<OutputAnnotation>, Box<crate::helpers::HelperError>> {
    let raw_annotations: Option<Vec<ChatAnnotation>> = input
        .get("annotations")
        .filter(|annotations| !annotations.is_null())
        .map(|annotations| serde_json::from_value(annotations.clone()))
        .transpose()
        .map_err(|error| invalid_details(&error))?;
    Ok(betterlite_core::api::responses_output_details::citation_fields(raw_annotations.as_deref()))
}

fn projected_usage(
    input: &serde_json::Value,
) -> Result<
    betterlite_openai::api::responses_objects::ResponseApiUsage,
    Box<crate::helpers::HelperError>,
> {
    let usage_input = input.get("usage").map_or(
        betterlite_core::api::responses_usage::UsageInput::Absent,
        betterlite_core::api::responses_usage::UsageInput::Mapping,
    );
    let normalized =
        betterlite_core::api::responses_usage::normalize_usage(usage_input).map_err(|error| {
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
    Ok(betterlite_core::api::responses_output_details::usage_details(Some(&typed_usage)))
}

fn invalid_details(error: &serde_json::Error) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
