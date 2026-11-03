use std::collections::BTreeMap;

use crate::helpers::HelperError;
use betterlite_models::usage_sum::{
    CompletionTokensDetails, PromptTokensDetails, Usage, UsageNumber, usage_sum,
};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::{Value, json};
pub(crate) mod input;

use input::Input;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> crate::helpers::HelperResult {
        let usages = input
            .usages
            .iter()
            .map(|usage| {
                Ok(Usage {
                    prompt_tokens: usage.prompt_tokens.unwrap_or(0),
                    completion_tokens: usage.completion_tokens.unwrap_or(0),
                    total_tokens: usage.total_tokens.unwrap_or(0),
                    prompt_tokens_details: usage
                        .prompt_tokens_details
                        .as_ref()
                        .map(PromptTokensDetails::from),
                    completion_tokens_details: usage
                        .completion_tokens_details
                        .as_ref()
                        .map(CompletionTokensDetails::from),
                    numeric_extras: convert_entries(&usage.numeric_extras, number_from_json)?,
                })
            })
            .collect::<Result<Vec<_>, Box<HelperError>>>()?;
        let result = usage_sum(&usages).map_err(|error| {
            Box::new(HelperError {
                class: "UsageSumError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        usage_value(&result)
    }
}

fn convert_entries<T, V, F>(
    source: &BTreeMap<String, T>,
    mut convert: F,
) -> Result<BTreeMap<String, V>, Box<HelperError>>
where
    F: FnMut(&str, &T) -> Result<V, Box<HelperError>>,
{
    let mut result = BTreeMap::new();
    for (name, value) in source {
        result.insert(name.clone(), convert(name, value)?);
    }
    Ok(result)
}

fn number_from_json(name: &str, raw: &Value) -> Result<UsageNumber, Box<HelperError>> {
    if let Some(integer) = raw.as_i64() {
        return Ok(UsageNumber::Integer(integer));
    }
    if let Some(boolean) = raw.as_bool() {
        return Ok(UsageNumber::Boolean(boolean));
    }
    let Some(float) = raw.as_f64() else {
        return Err(Box::new(HelperError {
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: format!("extra {name} is not numeric"),
        }));
    };
    Decimal::try_from(float)
        .map(UsageNumber::Decimal)
        .map_err(|error| {
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

fn usage_value(usage: &Usage) -> Result<Value, Box<HelperError>> {
    let prompt_tokens_details = usage
        .prompt_tokens_details
        .as_ref()
        .map(prompt_details_value)
        .transpose()?;
    let completion_tokens_details = usage.completion_tokens_details.as_ref().map(|details| {
        json!({
            "text_tokens": details.text_tokens,
            "reasoning_tokens": details.reasoning_tokens,
            "accepted_prediction_tokens": details.accepted_prediction_tokens,
            "rejected_prediction_tokens": details.rejected_prediction_tokens,
            "audio_tokens": details.audio_tokens,
            "image_tokens": details.image_tokens,
            "video_tokens": details.video_tokens,
        })
    });
    let numeric_extras = convert_entries(&usage.numeric_extras, |_, value| number_value(value))?;
    Ok(json!({
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "prompt_tokens_details": prompt_tokens_details,
        "completion_tokens_details": completion_tokens_details,
        "numeric_extras": numeric_extras,
    }))
}

fn prompt_details_value(details: &PromptTokensDetails) -> Result<Value, Box<HelperError>> {
    let cached_tokens_details = details.cached_tokens_details.as_ref().map(|cached| {
        json!({
            "text_tokens": cached.text_tokens,
            "audio_tokens": cached.audio_tokens,
            "image_tokens": cached.image_tokens,
        })
    });
    Ok(json!({
        "text_tokens": details.text_tokens,
        "audio_tokens": details.audio_tokens,
        "image_tokens": details.image_tokens,
        "video_tokens": details.video_tokens,
        "cache_write_tokens": details.cache_write_tokens,
        "cache_creation_tokens": details.cache_creation_tokens,
        "cached_tokens": details.cached_tokens,
        "cached_tokens_details": cached_tokens_details,
        "web_search_requests": details.web_search_requests,
        "google_maps_grounding_requests": details.google_maps_grounding_requests,
        "tool_use_tokens": details.tool_use_tokens,
        "character_count": details.character_count,
        "image_count": details.image_count,
        "video_length_seconds": decimal_value("video_length_seconds", details.video_length_seconds)?,
        "audio_length_seconds": decimal_value("audio_length_seconds", details.audio_length_seconds)?,
        "query_count": details.query_count,
    }))
}

fn decimal_value(name: &str, value: Option<Decimal>) -> Result<Value, Box<HelperError>> {
    let Some(decimal) = value else {
        return Ok(Value::Null);
    };
    decimal_to_value(decimal, &format!("decimal {name}"))
}

fn decimal_to_value(decimal: Decimal, label: &str) -> Result<Value, Box<HelperError>> {
    decimal.to_f64().map(|float| json!(float)).ok_or_else(|| {
        Box::new(HelperError {
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: format!("{label} {decimal} has no f64 representation"),
        })
    })
}

fn number_value(value: &UsageNumber) -> Result<Value, Box<HelperError>> {
    match value {
        UsageNumber::Boolean(inner) => Ok(json!(inner)),
        UsageNumber::Integer(inner) => Ok(json!(inner)),
        UsageNumber::Decimal(inner) => decimal_to_value(*inner, "decimal extra"),
    }
}
