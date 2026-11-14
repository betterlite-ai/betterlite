use axum::http::{HeaderMap, HeaderName, HeaderValue};
use betterlite_config::sections::detailed_timing::DetailedTiming;
use betterlite_config::sections::settings::JsonValue;
use betterlite_guardrails::guardrail_metadata::{RequestData, logging_caching_headers};
use betterlite_openai::openai_base_config::Object as JsonMap;

pub mod input;

mod cost;
mod tests;

use cost::BreakdownHeaders;
use input::{
    CustomHeadersInput, KeyHeaderView, api_base, deployment_model_name, python_float, python_str,
    resolved_call_id, timing_values, truthy,
};

// LiteLLM's ProxyBaseLLMRequestProcessing.get_custom_headers, the headers every LLM route answers with
#[must_use]
pub fn custom_headers(input: &CustomHeadersInput, timing: DetailedTiming) -> HeaderMap {
    let breakdown = cost::breakdown_headers(input.logging.as_ref(), input.response_cost.as_ref());
    let mut headers = HeaderSet::default();
    insert_identity(&mut headers, input);
    insert_costs(&mut headers, input, breakdown);
    insert_key_limits(
        &mut headers,
        &input.key,
        cost::updated_spend(input.key.spend, input.response_cost.as_ref()),
    );
    insert_timings(&mut headers, input);
    if timing.enabled {
        insert_detailed_timings(&mut headers, &input.hidden_params);
    }
    insert_batch_headers(&mut headers, input);
    for (name, value) in &input.extra {
        headers.set(name, python_str(name, value));
    }
    if !input.request_data.is_empty() {
        insert_remaining_limits(&mut headers, &input.request_data);
        insert_guardrail_headers(&mut headers, &input.request_data);
    }
    headers.into_header_map()
}

#[derive(Default)]
struct HeaderSet {
    entries: Vec<(String, Option<String>)>,
}

impl HeaderSet {
    fn set(&mut self, name: &str, text: Option<String>) {
        if let Some(existing) = self
            .entries
            .iter_mut()
            .find(|(existing, _)| existing == name)
        {
            existing.1 = text;
            return;
        }
        self.entries.push((name.to_owned(), text));
    }

    fn into_header_map(self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, rendered) in self.entries {
            let Some(text) = rendered.filter(|text| !text.is_empty() && text != "None") else {
                continue;
            };
            insert_header(&mut headers, &name, &text);
        }
        headers
    }
}

fn insert_header(headers: &mut HeaderMap, name: &str, text: &str) {
    let parsed = match HeaderName::from_bytes(name.as_bytes()) {
        Ok(parsed) => parsed,
        Err(error) => {
            tracing::warn!(header = name, %error, "response header name cannot be sent");
            return;
        }
    };
    match HeaderValue::try_from(text) {
        Ok(value) => {
            headers.insert(parsed, value);
        }
        Err(error) => {
            tracing::warn!(header = name, %error, "response header value cannot be sent");
        }
    }
}

fn insert_identity(headers: &mut HeaderSet, input: &CustomHeadersInput) {
    headers.set("x-litellm-call-id", resolved_call_id(input));
    headers.set("x-litellm-model-id", input.model_id.clone());
    headers.set(
        "x-litellm-model-name",
        deployment_model_name(input.logging.as_ref()),
    );
    headers.set("x-litellm-cache-key", input.cache_key.clone());
    headers.set(
        "x-litellm-model-api-base",
        api_base(input.api_base.as_deref()),
    );
    headers.set("x-litellm-version", input.version.clone());
    headers.set("x-litellm-model-region", input.model_region.clone());
}

fn insert_costs(headers: &mut HeaderSet, input: &CustomHeadersInput, breakdown: BreakdownHeaders) {
    let response_cost = input
        .response_cost
        .as_ref()
        .and_then(|value| python_str("x-litellm-response-cost", value));
    headers.set("x-litellm-response-cost", response_cost);
    let parts = [
        ("x-litellm-response-cost-original", breakdown.original_cost),
        (
            "x-litellm-response-cost-discount-amount",
            breakdown.discount_amount,
        ),
        (
            "x-litellm-response-cost-margin-amount",
            breakdown.margin_total_amount,
        ),
        (
            "x-litellm-response-cost-margin-percent",
            breakdown.margin_percent,
        ),
        ("x-litellm-response-cost-input", breakdown.input_cost),
        ("x-litellm-response-cost-output", breakdown.output_cost),
        (
            "x-litellm-response-cost-cache-read",
            breakdown.cache_read_cost,
        ),
        (
            "x-litellm-response-cost-cache-creation",
            breakdown.cache_creation_cost,
        ),
        (
            "x-litellm-response-cost-reasoning",
            breakdown.reasoning_cost,
        ),
        (
            "x-litellm-response-cost-tool-usage",
            breakdown.tool_usage_cost,
        ),
    ];
    for (header, text) in parts {
        headers.set(header, text);
    }
    headers.set(
        "x-litellm-classifier-cost",
        cost::classifier_cost(&input.request_data).map(python_float),
    );
}

fn insert_key_limits(headers: &mut HeaderSet, key: &KeyHeaderView, spend: f64) {
    headers.set(
        "x-litellm-key-tpm-limit",
        key.tpm_limit.map(|limit| limit.to_string()),
    );
    headers.set(
        "x-litellm-key-rpm-limit",
        key.rpm_limit.map(|limit| limit.to_string()),
    );
    headers.set("x-litellm-key-max-budget", key.max_budget.map(python_float));
    headers.set("x-litellm-key-spend", Some(python_float(spend)));
}

fn insert_timings(headers: &mut HeaderSet, input: &CustomHeadersInput) {
    let (response_ms, overhead_ms) = timing_values(input);
    headers.set(
        "x-litellm-response-duration-ms",
        response_ms.and_then(|value| python_str("x-litellm-response-duration-ms", value)),
    );
    headers.set(
        "x-litellm-overhead-duration-ms",
        overhead_ms.and_then(|value| python_str("x-litellm-overhead-duration-ms", value)),
    );
    headers.set(
        "x-litellm-callback-duration-ms",
        input
            .hidden_params
            .get("callback_duration_ms")
            .and_then(|value| python_str("x-litellm-callback-duration-ms", value)),
    );
}

fn insert_detailed_timings(headers: &mut HeaderSet, hidden_params: &JsonMap) {
    for (header, field) in [
        (
            "x-litellm-timing-pre-processing-ms",
            "timing_pre_processing_ms",
        ),
        ("x-litellm-timing-llm-api-ms", "timing_llm_api_ms"),
        (
            "x-litellm-timing-post-processing-ms",
            "timing_post_processing_ms",
        ),
        ("x-litellm-timing-message-copy-ms", "timing_message_copy_ms"),
    ] {
        headers.set(
            header,
            hidden_params
                .get(field)
                .and_then(|value| python_str(header, value)),
        );
    }
}

fn insert_batch_headers(headers: &mut HeaderSet, input: &CustomHeadersInput) {
    let batch = input.fastest_response_batch_completion.map(|flag| {
        if flag {
            "True".to_owned()
        } else {
            "False".to_owned()
        }
    });
    headers.set("x-litellm-fastest_response_batch_completion", batch);
    headers.set("x-litellm-timeout", input.timeout.map(python_float));
}

fn insert_remaining_limits(headers: &mut HeaderSet, request_data: &JsonMap) {
    let Some(JsonValue::Object(metadata)) = request_data.get("metadata") else {
        return;
    };
    let model_group = metadata.get("model_group");
    let group_text = match model_group {
        Some(value) if !value.is_null() => match python_str("model_group", value) {
            Some(text) => text,
            None => return,
        },
        None | Some(_) => "None".to_owned(),
    };
    let header_group = model_group.filter(|value| truthy(value)).map_or_else(
        || "None".to_owned(),
        |_| group_text.replace('/', "-").replace(':', "-"),
    );
    let requests_key = format!("litellm-key-remaining-requests-{group_text}");
    if let Some(value) = metadata.get(&requests_key).filter(|value| truthy(value)) {
        let header = format!("x-litellm-key-remaining-requests-{header_group}");
        headers.set(&header, python_str(&header, value));
    }
    let tokens_key = format!("litellm-key-remaining-tokens-{group_text}");
    if let Some(value) = metadata.get(&tokens_key).filter(|value| truthy(value)) {
        let header = format!("x-litellm-key-remaining-tokens-{header_group}");
        headers.set(&header, python_str(&header, value));
    }
}

fn insert_guardrail_headers(headers: &mut HeaderSet, request_data: &JsonMap) {
    let Ok(text) = serde_json::to_string(request_data) else {
        return;
    };
    let Ok(fields) = serde_json::from_str::<RequestData>(&text) else {
        tracing::warn!("the request data cannot be read as guardrail metadata");
        return;
    };
    match logging_caching_headers(&fields) {
        Ok(entries) => {
            for (name, value) in entries {
                headers.set(&name, Some(value));
            }
        }
        Err(error) => tracing::warn!(%error, "the guardrail headers cannot be built"),
    }
}
