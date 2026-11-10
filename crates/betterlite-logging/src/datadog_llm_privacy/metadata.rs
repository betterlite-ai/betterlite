use betterlite_models::decisions::Json;

use super::guardrail_redact::{guardrail_record, redacted_records};
use super::json_number::number as json_number;
use super::latency::{latency_metrics, metrics_object as latency_object};
use super::spend::{metrics_object as spend_object, spend_metrics};
use super::stream::is_streamed_request;
use crate::datadog_llm_obs::constants::METADATA_PROMPT_FIELDS;
use crate::datadog_llm_obs::values::Object;
use crate::fields::payload::StandardLoggingPayload;
use crate::fields::payload::records::StandardLoggingMetadata;

// LiteLLM's _get_dd_llm_obs_payload_metadata: the fixed span fields, the request metadata, router fields, cost tags
#[must_use]
pub fn span_metadata(
    payload: &StandardLoggingPayload,
    router_fields: &Object,
    cost_tags: &[String],
    redact_prompt_text: bool,
) -> Object {
    let mut metadata = fixed_fields(payload, redact_prompt_text);
    for (field, value) in request_fields(&payload.metadata) {
        if redact_prompt_text && METADATA_PROMPT_FIELDS.contains(&field) {
            continue;
        }
        metadata.insert(field.to_owned(), value);
    }
    for (field, value) in router_fields {
        metadata.insert(field.clone(), value.clone());
    }
    if !cost_tags.is_empty() {
        metadata.insert("_dd".to_owned(), Json::Object(dd_fields(cost_tags)));
    }
    metadata
}

fn fixed_fields(payload: &StandardLoggingPayload, redact_prompt_text: bool) -> Object {
    let unknown = || Json::Text("unknown".to_owned());
    let mut metadata = Object::new();
    metadata.insert("model_name".to_owned(), Json::Text(payload.model.clone()));
    let provider = payload.custom_llm_provider.clone();
    metadata.insert(
        "model_provider".to_owned(),
        provider.map_or_else(unknown, Json::Text),
    );
    metadata.insert("id".to_owned(), Json::Text(payload.id.clone()));
    metadata.insert("trace_id".to_owned(), Json::Text(payload.trace_id.clone()));
    metadata.insert(
        "cache_hit".to_owned(),
        payload.cache_hit.map_or_else(unknown, Json::Bool),
    );
    let cache_key = payload.cache_key.clone();
    metadata.insert(
        "cache_key".to_owned(),
        cache_key.map_or_else(unknown, Json::Text),
    );
    let cost = json_number(payload.saved_cache_cost);
    metadata.insert("saved_cache_cost".to_owned(), cost);
    let guardrails = guardrail_value(payload, redact_prompt_text);
    metadata.insert("guardrail_information".to_owned(), guardrails);
    let streamed = Json::Bool(is_streamed_request(payload));
    metadata.insert("is_streamed_request".to_owned(), streamed);
    for (field, value) in metric_fields(payload) {
        metadata.insert(field.to_owned(), value);
    }
    metadata
}

fn metric_fields(payload: &StandardLoggingPayload) -> [(&'static str, Json); 2] {
    let latency = latency_metrics(payload);
    let spend = spend_metrics(payload);
    [
        ("latency_metrics", Json::Object(latency_object(&latency))),
        ("spend_metrics", Json::Object(spend_object(&spend))),
    ]
}

fn guardrail_value(payload: &StandardLoggingPayload, redact_prompt_text: bool) -> Json {
    let Some(records) = payload.guardrail_information.as_deref() else {
        return Json::Null;
    };
    if redact_prompt_text {
        return Json::List(
            redacted_records(records)
                .into_iter()
                .map(Json::Object)
                .collect(),
        );
    }
    Json::List(
        records
            .iter()
            .map(|record| Json::Object(guardrail_record(record)))
            .collect(),
    )
}

fn dd_fields(cost_tags: &[String]) -> Object {
    let mut tags = Vec::with_capacity(cost_tags.len());
    for tag in cost_tags {
        tags.push(Json::Text(tag.clone()));
    }
    let mut fields = Object::new();
    fields.insert("cost_tags".to_owned(), Json::List(tags));
    fields
}

fn request_fields(metadata: &StandardLoggingMetadata) -> Vec<(&'static str, Json)> {
    let mut fields = text_fields(metadata);
    fields.extend(budget_fields(metadata));
    fields.extend(json_fields(metadata));
    fields.extend(header_fields(metadata));
    fields.extend(list_fields(metadata));
    fields.extend(flag_fields(metadata));
    fields
}

fn text_fields(metadata: &StandardLoggingMetadata) -> Vec<(&'static str, Json)> {
    let candidates = [
        ("user_api_key_hash", &metadata.user_api_key_hash),
        ("user_api_key_alias", &metadata.user_api_key_alias),
        (
            "user_api_key_budget_reset_at",
            &metadata.user_api_key_budget_reset_at,
        ),
        ("user_api_key_org_id", &metadata.user_api_key_org_id),
        ("user_api_key_org_alias", &metadata.user_api_key_org_alias),
        ("user_api_key_team_id", &metadata.user_api_key_team_id),
        ("user_api_key_project_id", &metadata.user_api_key_project_id),
        (
            "user_api_key_project_alias",
            &metadata.user_api_key_project_alias,
        ),
        ("user_api_key_user_id", &metadata.user_api_key_user_id),
        ("user_api_key_user_email", &metadata.user_api_key_user_email),
        ("user_api_key_team_alias", &metadata.user_api_key_team_alias),
        (
            "user_api_key_end_user_id",
            &metadata.user_api_key_end_user_id,
        ),
        (
            "user_api_key_request_route",
            &metadata.user_api_key_request_route,
        ),
        ("requester_ip_address", &metadata.requester_ip_address),
        ("user_agent", &metadata.user_agent),
        ("cold_storage_object_key", &metadata.cold_storage_object_key),
        ("team_alias", &metadata.team_alias),
        ("team_id", &metadata.team_id),
    ];
    let mut fields = Vec::new();
    for (field, value) in candidates {
        if let Some(text) = value {
            fields.push((field, Json::Text(text.clone())));
        }
    }
    fields
}

fn budget_fields(metadata: &StandardLoggingMetadata) -> Vec<(&'static str, Json)> {
    let candidates = [
        ("user_api_key_spend", &metadata.user_api_key_spend),
        ("user_api_key_max_budget", &metadata.user_api_key_max_budget),
        ("user_api_key_user_spend", &metadata.user_api_key_user_spend),
        (
            "user_api_key_user_max_budget",
            &metadata.user_api_key_user_max_budget,
        ),
        ("user_api_key_team_spend", &metadata.user_api_key_team_spend),
        (
            "user_api_key_team_max_budget",
            &metadata.user_api_key_team_max_budget,
        ),
    ];
    let mut fields = Vec::new();
    for (field, value) in candidates {
        if let Some(money) = value {
            fields.push((field, json_number(money.0)));
        }
    }
    fields
}

fn json_fields(metadata: &StandardLoggingMetadata) -> Vec<(&'static str, Json)> {
    let candidates = [
        ("spend_logs_metadata", &metadata.spend_logs_metadata),
        ("requester_metadata", &metadata.requester_metadata),
        (
            "prompt_management_metadata",
            &metadata.prompt_management_metadata,
        ),
        ("mcp_tool_call_metadata", &metadata.mcp_tool_call_metadata),
        ("routing_decision", &metadata.routing_decision),
        ("usage_object", &metadata.usage_object),
    ];
    let mut fields = Vec::new();
    for (field, value) in candidates {
        if let Some(present) = value {
            fields.push((field, present.clone()));
        }
    }
    if let Some(blocks) = &metadata.vector_store_request_metadata {
        fields.push(("vector_store_request_metadata", Json::List(blocks.clone())));
    }
    fields
}

fn header_fields(metadata: &StandardLoggingMetadata) -> Vec<(&'static str, Json)> {
    let candidates = [
        (
            "user_api_key_auth_metadata",
            &metadata.user_api_key_auth_metadata,
        ),
        (
            "requester_custom_headers",
            &metadata.requester_custom_headers,
        ),
    ];
    let mut fields = Vec::new();
    for (field, value) in candidates {
        if let Some(headers) = value {
            fields.push((field, Json::Object(headers_map(headers))));
        }
    }
    fields
}

fn headers_map(headers: &indexmap::IndexMap<String, String>) -> Object {
    let mut object = Object::new();
    for (name, value) in headers {
        object.insert(name.clone(), Json::Text(value.clone()));
    }
    object
}

fn list_fields(metadata: &StandardLoggingMetadata) -> Vec<(&'static str, Json)> {
    let mut fields = Vec::new();
    if let Some(names) = &metadata.applied_guardrails {
        let mut items = Vec::with_capacity(names.len());
        for name in names {
            items.push(Json::Text(name.clone()));
        }
        fields.push(("applied_guardrails", Json::List(items)));
    }
    fields
}

fn flag_fields(metadata: &StandardLoggingMetadata) -> Vec<(&'static str, Json)> {
    let mut fields = Vec::new();
    if let Some(flag) = metadata.used_client_oauth_token {
        fields.push(("used_client_oauth_token", Json::Bool(flag)));
    }
    fields
}

mod tests;
