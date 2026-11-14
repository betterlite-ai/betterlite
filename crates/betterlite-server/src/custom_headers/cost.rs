use betterlite_config::sections::settings::JsonValue;
use betterlite_models::savings_evidence::numeric::{RoutingDecision, numeric_savings};
use betterlite_openai::openai_base_config::Object as JsonMap;
use rust_decimal::Decimal;

use super::input::{LoggingHeaderView, float_text, header_text, non_empty};

// LiteLLM's NON_INFERENCE_CALL_TYPES: a read or management route replays usage, so its zero price is reported as a component split
const NON_INFERENCE_CALL_TYPES: [&str; 30] = [
    "get_responses",
    "aget_responses",
    "delete_responses",
    "adelete_responses",
    "cancel_responses",
    "acancel_responses",
    "list_input_items",
    "alist_input_items",
    "vector_store_create",
    "avector_store_create",
    "vector_store_retrieve",
    "avector_store_retrieve",
    "vector_store_list",
    "avector_store_list",
    "vector_store_update",
    "avector_store_update",
    "vector_store_delete",
    "avector_store_delete",
    "vector_store_file_create",
    "avector_store_file_create",
    "vector_store_file_list",
    "avector_store_file_list",
    "vector_store_file_retrieve",
    "avector_store_file_retrieve",
    "vector_store_file_content",
    "avector_store_file_content",
    "vector_store_file_update",
    "avector_store_file_update",
    "vector_store_file_delete",
    "avector_store_file_delete",
];

#[derive(Debug, Default)]
pub(crate) struct BreakdownHeaders {
    pub(crate) original_cost: Option<String>,
    pub(crate) discount_amount: Option<String>,
    pub(crate) margin_total_amount: Option<String>,
    pub(crate) margin_percent: Option<String>,
    pub(crate) input_cost: Option<String>,
    pub(crate) output_cost: Option<String>,
    pub(crate) cache_read_cost: Option<String>,
    pub(crate) cache_creation_cost: Option<String>,
    pub(crate) reasoning_cost: Option<String>,
    pub(crate) tool_usage_cost: Option<String>,
}

impl BreakdownHeaders {
    pub(crate) fn into_headers(self) -> [(&'static str, Option<String>); 10] {
        [
            ("x-litellm-response-cost-original", self.original_cost),
            (
                "x-litellm-response-cost-discount-amount",
                self.discount_amount,
            ),
            (
                "x-litellm-response-cost-margin-amount",
                self.margin_total_amount,
            ),
            (
                "x-litellm-response-cost-margin-percent",
                self.margin_percent,
            ),
            ("x-litellm-response-cost-input", self.input_cost),
            ("x-litellm-response-cost-output", self.output_cost),
            ("x-litellm-response-cost-cache-read", self.cache_read_cost),
            (
                "x-litellm-response-cost-cache-creation",
                self.cache_creation_cost,
            ),
            ("x-litellm-response-cost-reasoning", self.reasoning_cost),
            ("x-litellm-response-cost-tool-usage", self.tool_usage_cost),
        ]
    }
}

pub(crate) fn breakdown_headers(
    logging_view: Option<&LoggingHeaderView>,
    response_cost: Option<&JsonValue>,
) -> BreakdownHeaders {
    let Some(logging) = logging_view else {
        return BreakdownHeaders::default();
    };
    let Some(breakdown) = logging
        .cost_breakdown
        .as_ref()
        .filter(|breakdown| !breakdown.is_empty())
    else {
        return missing_breakdown(logging, response_cost);
    };
    BreakdownHeaders {
        original_cost: breakdown
            .get("original_cost")
            .and_then(|value| header_text("x-litellm-response-cost-original", value)),
        discount_amount: breakdown
            .get("discount_amount")
            .and_then(|value| header_text("x-litellm-response-cost-discount-amount", value)),
        margin_total_amount: breakdown
            .get("margin_total_amount")
            .and_then(|value| header_text("x-litellm-response-cost-margin-amount", value)),
        margin_percent: breakdown
            .get("margin_percent")
            .and_then(|value| header_text("x-litellm-response-cost-margin-percent", value)),
        input_cost: uncached_input_cost(breakdown),
        output_cost: breakdown
            .get("output_cost")
            .and_then(|value| header_text("x-litellm-response-cost-output", value)),
        cache_read_cost: breakdown
            .get("cache_read_cost")
            .and_then(|value| header_text("x-litellm-response-cost-cache-read", value)),
        cache_creation_cost: breakdown
            .get("cache_creation_cost")
            .and_then(|value| header_text("x-litellm-response-cost-cache-creation", value)),
        reasoning_cost: breakdown
            .get("reasoning_cost")
            .and_then(|value| header_text("x-litellm-response-cost-reasoning", value)),
        tool_usage_cost: breakdown
            .get("tool_usage_cost")
            .and_then(|value| header_text("x-litellm-response-cost-tool-usage", value)),
    }
}

fn missing_breakdown(
    logging: &LoggingHeaderView,
    response_cost: Option<&JsonValue>,
) -> BreakdownHeaders {
    if NON_INFERENCE_CALL_TYPES.contains(&logging.call_type.as_str())
        && totals_to_zero(response_cost)
    {
        let zero = "0.0".to_owned();
        return BreakdownHeaders {
            original_cost: Some(zero.clone()),
            discount_amount: Some(zero.clone()),
            margin_total_amount: Some(zero.clone()),
            margin_percent: Some(zero.clone()),
            input_cost: Some(zero.clone()),
            output_cost: Some(zero.clone()),
            tool_usage_cost: Some(zero),
            cache_read_cost: None,
            cache_creation_cost: None,
            reasoning_cost: None,
        };
    }
    BreakdownHeaders::default()
}

// LiteLLM subtracts the cache costs from the stored input cost and advertises the additive split
fn uncached_input_cost(breakdown: &JsonMap) -> Option<String> {
    let input_cost = breakdown.get("input_cost")?;
    if input_cost.is_null() {
        return None;
    }
    let input = numeric_total(input_cost, "input_cost")?;
    let cache_read = cache_part(breakdown.get("cache_read_cost"), "cache_read_cost")?;
    let cache_creation = cache_part(breakdown.get("cache_creation_cost"), "cache_creation_cost")?;
    // float_arithmetic is denied; a fused multiply-add computes the difference exactly like the subtraction
    let after_cache_read = cache_read.mul_add(-1.0_f64, input);
    Some(float_text(
        cache_creation.mul_add(-1.0_f64, after_cache_read),
    ))
}

// LiteLLM's input_cost must be a number: only None means absent there, and a bool counts as 1 or 0
fn numeric_total(value: &JsonValue, field: &str) -> Option<f64> {
    let total = match value {
        JsonValue::Bool(flag) => Some(if *flag { 1.0 } else { 0.0 }),
        JsonValue::Number(number) => number.as_f64(),
        JsonValue::Null | JsonValue::String(_) | JsonValue::Array(_) | JsonValue::Object(_) => None,
    };
    if total.is_none() {
        tracing::warn!(field, "a cost breakdown part must be a finite number");
    }
    total
}

// LiteLLM's `cache_read_cost or 0.0`: a missing, null or falsey cache cost counts as zero
fn cache_part(value: Option<&JsonValue>, field: &str) -> Option<f64> {
    match value {
        Some(cost) if non_empty(cost) => numeric_total(cost, field),
        Some(_) | None => Some(0.0),
    }
}

// LiteLLM adds only a positive response cost to the key spend
pub(crate) fn updated_spend(spend: Option<Decimal>, response_cost: Option<&JsonValue>) -> f64 {
    let current_spend = spend.map_or(0.0_f64, |value| value.as_f64());
    match response_cost.and_then(float_value) {
        // float_arithmetic is denied; a fused multiply-add with one is the same sum
        Some(cost) if cost > 0.0_f64 => cost.mul_add(1.0_f64, current_spend),
        Some(_) | None => current_spend,
    }
}

// LiteLLM's classifier_cost_from_decision, read from litellm_metadata first and then metadata
pub(crate) fn classifier_cost(request_data: &JsonMap) -> Option<f64> {
    for bucket in ["litellm_metadata", "metadata"] {
        let Some(JsonValue::Object(metadata)) = request_data.get(bucket) else {
            continue;
        };
        let Some(JsonValue::Object(decision)) = metadata.get("routing_decision") else {
            continue;
        };
        let Ok(text) = serde_json::to_string(decision) else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<RoutingDecision>(&text) else {
            continue;
        };
        if let Some(cost) = numeric_savings(parsed.classifier_cost.as_ref()) {
            return Some(cost.as_f64());
        }
    }
    None
}

// Whether the total the headers carry is zero, counting a total no route ever priced as one
fn totals_to_zero(response_cost: Option<&JsonValue>) -> bool {
    match response_cost {
        None | Some(JsonValue::Null) => true,
        Some(JsonValue::String(text)) if text.is_empty() => true,
        Some(value) => float_value(value).is_some_and(|cost| cost == 0.0_f64),
    }
}

// Python's float(): a bool, a JSON number, or text its grammar reads
fn float_value(value: &JsonValue) -> Option<f64> {
    match value {
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
        JsonValue::String(text) => float_from_text(text),
        JsonValue::Bool(flag) => Some(if *flag { 1.0 } else { 0.0 }),
        JsonValue::Number(number) => number.as_f64(),
    }
}

// float() trims spaces and reads a decimal whose digits may carry PEP 515 underscores
fn float_from_text(text: &str) -> Option<f64> {
    let body = text.trim();
    let mut cleaned = String::with_capacity(body.len());
    let mut previous = None;
    for (at, character) in body.char_indices() {
        if character == '_' {
            let next = body.get(at + 1..)?.chars().next()?;
            if previous.is_none_or(|before| !before.is_ascii_digit()) || !next.is_ascii_digit() {
                return None;
            }
            continue;
        }
        cleaned.push(character);
        previous = Some(character);
    }
    cleaned.parse().ok()
}
