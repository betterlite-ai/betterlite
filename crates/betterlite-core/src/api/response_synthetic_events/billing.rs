use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::{ResponseApiUsage, ResponsesApiResponse};
use indexmap::IndexMap;

use super::json_value;

const DEFAULTED_FIELDS: [&str; 20] = [
    "error",
    "incomplete_details",
    "instructions",
    "metadata",
    "model",
    "object",
    "parallel_tool_calls",
    "temperature",
    "tool_choice",
    "tools",
    "top_p",
    "max_output_tokens",
    "previous_response_id",
    "reasoning",
    "status",
    "text",
    "truncation",
    "usage",
    "user",
    "store",
];

pub enum UsageSource<'a> {
    Model(&'a ResponseApiUsage),
    Value(&'a Json),
}

pub trait UsageEstimate {
    fn estimate(&self) -> Option<ResponseApiUsage>;
}

#[must_use]
pub fn usage_as_model(source: Option<UsageSource<'_>>) -> Option<ResponseApiUsage> {
    match source? {
        UsageSource::Model(usage) => Some(usage.clone()),
        UsageSource::Value(value) => json_value::from_json::<ResponseApiUsage>(value).ok(),
    }
}

pub enum BilledTerminal<'a> {
    Same(&'a ResponsesApiResponse),
    Estimated(Box<ResponsesApiResponse>),
}

#[must_use]
pub fn billed_terminal_response<'a>(
    response: &'a ResponsesApiResponse,
    estimate: Option<&dyn UsageEstimate>,
) -> BilledTerminal<'a> {
    let Some(estimate) = estimate else {
        return BilledTerminal::Same(response);
    };
    if response.usage.is_some() {
        return BilledTerminal::Same(response);
    }
    let mut copied = response.clone();
    copied.usage = estimate.estimate();
    BilledTerminal::Estimated(Box::new(copied))
}

#[must_use]
pub fn billed_terminal_object(
    object: &Json,
    estimate: Option<&dyn UsageEstimate>,
) -> Option<IndexMap<String, Json>> {
    let Json::Object(raw) = object else {
        return None;
    };
    let mut constructed = raw.clone();
    for field in DEFAULTED_FIELDS {
        constructed.entry(field.to_owned()).or_insert(Json::Null);
    }
    let absent_usage = raw
        .get("usage")
        .is_none_or(|value| matches!(value, Json::Null));
    if !absent_usage {
        return Some(constructed);
    }
    let Some(estimate) = estimate else {
        return Some(constructed);
    };
    let usage = estimate.estimate();
    constructed.insert("usage".to_owned(), usage_json(usage.as_ref()));
    Some(constructed)
}

fn usage_json(usage: Option<&ResponseApiUsage>) -> Json {
    match usage {
        Some(usage) => json_value::to_json(usage).unwrap_or(Json::Null),
        None => Json::Null,
    }
}
