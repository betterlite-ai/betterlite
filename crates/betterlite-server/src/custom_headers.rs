use axum::http::{HeaderMap, HeaderName, HeaderValue};
use betterlite_config::sections::detailed_timing::DetailedTiming;
use betterlite_config::sections::settings::JsonValue;
use betterlite_guardrails::guardrail_metadata::{RequestData, logging_caching_headers};
use betterlite_openai::openai_base_config::Object as JsonMap;
use rust_decimal::Decimal;

pub mod input;

mod cost;
mod tests;

use cost::BreakdownHeaders;
use input::{
    CustomHeadersInput, KeyHeaderView, api_base, deployment_model_name, float_text, header_text,
    non_empty, resolved_call_id, timing_values,
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
        headers.set(name, header_text(name, value));
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
        .and_then(|value| header_text("x-litellm-response-cost", value));
    headers.set("x-litellm-response-cost", response_cost);
    for (header, text) in breakdown.into_headers() {
        headers.set(header, text);
    }
    headers.set(
        "x-litellm-classifier-cost",
        cost::classifier_cost(&input.request_data).map(|cost| cost.to_string()),
    );
}

fn insert_key_limits(headers: &mut HeaderSet, key: &KeyHeaderView, spend: Decimal) {
    headers.set(
        "x-litellm-key-tpm-limit",
        key.tpm_limit.map(|limit| limit.to_string()),
    );
    headers.set(
        "x-litellm-key-rpm-limit",
        key.rpm_limit.map(|limit| limit.to_string()),
    );
    headers.set(
        "x-litellm-key-max-budget",
        key.max_budget.map(|budget| budget.to_string()),
    );
    headers.set("x-litellm-key-spend", Some(spend.to_string()));
}

fn insert_timings(headers: &mut HeaderSet, input: &CustomHeadersInput) {
    let (response_ms, overhead_ms) = timing_values(input);
    let callback_ms = input.hidden_params.get("callback_duration_ms");
    for (header, value) in [
        ("x-litellm-response-duration-ms", response_ms),
        ("x-litellm-overhead-duration-ms", overhead_ms),
        ("x-litellm-callback-duration-ms", callback_ms),
    ] {
        headers.set(header, value.and_then(|value| header_text(header, value)));
    }
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
                .and_then(|value| header_text(header, value)),
        );
    }
}

fn insert_batch_headers(headers: &mut HeaderSet, input: &CustomHeadersInput) {
    let batch = input.fastest_batch.map(|flag| {
        if flag {
            "True".to_owned()
        } else {
            "False".to_owned()
        }
    });
    headers.set("x-litellm-fastest_response_batch_completion", batch);
    headers.set("x-litellm-timeout", input.timeout.map(float_text));
}

fn insert_remaining_limits(headers: &mut HeaderSet, request_data: &JsonMap) {
    let Some(JsonValue::Object(metadata)) = request_data.get("metadata") else {
        return;
    };
    let model_group = metadata.get("model_group");
    let group_text = match model_group {
        Some(value) => match header_text("model_group", value) {
            Some(text) => text,
            None => return,
        },
        None => "None".to_owned(),
    };
    let header_group = model_group.filter(|value| non_empty(value)).map_or_else(
        || "None".to_owned(),
        |_| group_text.replace(['/', ':'], "-"),
    );
    let requests_key = format!("litellm-key-remaining-requests-{group_text}");
    if let Some(value) = metadata.get(&requests_key).filter(|value| non_empty(value)) {
        let header = format!("x-litellm-key-remaining-requests-{header_group}");
        headers.set(&header, header_text(&header, value));
    }
    let tokens_key = format!("litellm-key-remaining-tokens-{group_text}");
    if let Some(value) = metadata.get(&tokens_key).filter(|value| non_empty(value)) {
        let header = format!("x-litellm-key-remaining-tokens-{header_group}");
        headers.set(&header, header_text(&header, value));
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
