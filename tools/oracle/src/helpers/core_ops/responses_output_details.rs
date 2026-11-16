use betterlite_core::api::responses_output_details::ChatAnnotation;
use betterlite_core::api::responses_output_details::ImageItem;
use betterlite_core::api::responses_usage::UsageInput;
use betterlite_core::api::{responses_output_details, responses_usage};
use betterlite_models::usage_sum::{Usage, UsageNumber};
use betterlite_openai::api::responses_objects::ResponseApiUsage;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::de::DeserializeOwned;
use std::collections::BTreeMap;

pub(crate) struct Op;

impl Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> HelperResult {
        let images: Option<Vec<ImageItem>> = parse_records_at(input, "images")?;
        let result = responses_output_details::image_results(
            images.as_deref(),
            input
                .get("finish_reason")
                .and_then(serde_json::Value::as_str),
            || "oracle-id".to_owned(),
        );
        let status = responses_output_details::image_finish_status(
            input
                .get("finish_reason")
                .and_then(serde_json::Value::as_str),
        );
        let annotations: Option<Vec<ChatAnnotation>> = parse_records_at(input, "annotations")?;
        let projected_annotations =
            responses_output_details::citation_fields(annotations.as_deref());
        let projected_usage = projected_usage(input)?;
        Ok(
            serde_json::json!({"images":result,"status":status,"annotations":projected_annotations,"usage":projected_usage}),
        )
    }
}

fn parse_records_at<T>(
    input: &serde_json::Value,
    field: &str,
) -> Result<Option<T>, Box<HelperError>>
where
    T: DeserializeOwned,
{
    input
        .get(field)
        .filter(|value| !value.is_null())
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()
        .map_err(|error: serde_json::Error| {
            Box::new(HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
}

fn projected_usage(input: &serde_json::Value) -> Result<ResponseApiUsage, Box<HelperError>> {
    let usage_input = input
        .get("usage")
        .map_or(UsageInput::Absent, UsageInput::Mapping);
    let normalized = responses_usage::normalize_usage(usage_input).map_err(|error| {
        Box::new(HelperError {
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })?;
    let mut numeric_extras = BTreeMap::new();
    if let Some(cost) = normalized.cost {
        numeric_extras.insert("cost".to_owned(), UsageNumber::Decimal(cost));
    }
    let typed_usage = Usage {
        prompt_tokens: normalized.prompt_tokens,
        completion_tokens: normalized.completion_tokens,
        total_tokens: normalized.total_tokens,
        prompt_tokens_details: normalized.prompt_tokens_details,
        completion_tokens_details: normalized.completion_tokens_details,
        numeric_extras,
    };
    Ok(responses_output_details::usage_details(Some(&typed_usage)))
}
