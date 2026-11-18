use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::{ResponseApiUsage, ResponsesApiResponse};
use indexmap::IndexMap;

// ResponsesAPIResponse's optional fields in model order; unchecked construction fills them with null.
const DEFAULTED_FIELDS: &str = "error incomplete_details instructions metadata model object \
     parallel_tool_calls temperature tool_choice tools top_p max_output_tokens previous_response_id \
     reasoning status text truncation usage user store";

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
        UsageSource::Value(value) => serde_json::to_value(value)
            .ok()
            .and_then(|converted| serde_json::from_value(converted).ok()),
    }
}

pub enum BilledTerminal<'a> {
    Same(&'a ResponsesApiResponse),
    Estimated(Box<ResponsesApiResponse>),
}

#[must_use]
pub fn billed_terminal_response<'a, E: UsageEstimate>(
    response: &'a ResponsesApiResponse,
    estimate: Option<&E>,
) -> BilledTerminal<'a> {
    let Some(estimator) = estimate else {
        return BilledTerminal::Same(response);
    };
    if response.usage.is_some() {
        return BilledTerminal::Same(response);
    }
    let mut copied = response.clone();
    copied.usage = estimator.estimate();
    BilledTerminal::Estimated(Box::new(copied))
}

#[must_use]
pub fn billed_terminal_object<E: UsageEstimate>(
    object: &Json,
    estimate: Option<&E>,
) -> Option<IndexMap<String, Json>> {
    let Json::Object(raw) = object else {
        return None;
    };
    let mut constructed = raw.clone();
    for field in DEFAULTED_FIELDS.split_whitespace() {
        constructed.entry(field.to_owned()).or_insert(Json::Null);
    }
    filter_reasoning_nulls(&mut constructed);
    let present_usage = raw
        .get("usage")
        .is_some_and(|value| !matches!(value, Json::Null));
    if present_usage {
        return Some(constructed);
    }
    let Some(estimator) = estimate else {
        return Some(constructed);
    };
    let usage = estimator.estimate();
    constructed.insert("usage".to_owned(), usage_json(usage.as_ref()));
    Some(constructed)
}

fn filter_reasoning_nulls(constructed: &mut IndexMap<String, Json>) {
    let Some(Json::List(items)) = constructed.get_mut("output") else {
        return;
    };
    for item in items {
        if let Json::Object(fields) = item
            && matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "reasoning")
        {
            fields.retain(|key, value| {
                !matches!(key.as_str(), "status" | "content" | "encrypted_content")
                    || !matches!(value, Json::Null)
            });
        }
    }
}

fn usage_json(usage: Option<&ResponseApiUsage>) -> Json {
    usage
        .and_then(|reported| serde_json::to_value(reported).ok())
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or(Json::Null)
}
