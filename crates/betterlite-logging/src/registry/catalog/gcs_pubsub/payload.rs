use betterlite_config::spend_storage::DEFAULT_MAX_PROMPT_LENGTH;
use betterlite_db::search_tools::convert::isoformat;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use super::ProjectionError;
use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;
use crate::latency_alerts::presentation::float_text;
use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};
use crate::spend_bodies::SpendStoragePolicy;
use crate::spend_bodies::messages::storage_messages;
use crate::spend_bodies::request::storage_request_body;
use crate::spend_bodies::response::storage_response;
use crate::spend_bodies::session::{SpendTimestamp, batch_trace_session_id, request_duration_ms};
use crate::spend_event::CACHE_OFF_KEY;
use crate::spend_metadata::records::SpendLogsMetadata;
use crate::spend_metadata::redaction::redact_logged_api_key;
use crate::spend_metadata::{
    BATCH_COST_REQUEST_ID_SUFFIX, SpendLogsPayload, as_object, as_text, optional_text_field,
    status_for_spend_log,
};
use crate::spend_sanitize::SpendSanitizePolicy;

mod metadata;

use metadata::clean_metadata;

const ARETRIEVE_BATCH: &str = "aretrieve_batch";
const MISSING_ID: &str = "None";
const CACHE_HIT_SUFFIX: &str = "_cache_hit";

// LiteLLM keeps request and response bodies out of spend logs unless store_prompts_in_spend_logs is on, and a logger event carries no such setting
const NO_BODY_STORAGE: SpendStoragePolicy = SpendStoragePolicy {
    sanitize: SpendSanitizePolicy {
        store_bodies: false,
        max_string_length: DEFAULT_MAX_PROMPT_LENGTH,
    },
    turn_off_message_logging: false,
};

#[derive(Clone, Copy)]
struct Times {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    completion_start: DateTime<Utc>,
}

// The values the SpendLogsPayload literal reads besides the event's payload and metadata.
struct Parts {
    stamps: Times,
    request_id: String,
    session_id: String,
    cache_key: String,
    request_tags: String,
    messages: String,
    duration: Option<i64>,
}

pub(crate) fn spend_log_payload(event: &LoggerEvent) -> Result<Json, ProjectionError> {
    let payload = event.payload.as_ref();
    let merged = event.litellm_metadata.as_ref();
    let clean = clean_metadata(payload, merged)?;
    let parts = parts(payload)?;
    let projected = payload_row(payload, merged, &clean, parts)?;
    serde_json::to_value(projected)
        .and_then(serde_json::from_value::<Json>)
        .map_err(ProjectionError::Encode)
}

// get_logging_payload's SpendLogsPayload literal, over the fields a logger event carries
fn payload_row(
    payload: &StandardLoggingPayload,
    merged: &JsonObject,
    clean: &SpendLogsMetadata,
    parts: Parts,
) -> Result<SpendLogsPayload, ProjectionError> {
    Ok(SpendLogsPayload {
        request_id: parts.request_id,
        call_type: payload.call_type.clone(),
        api_key: api_key(payload, clean),
        spend: spend(payload),
        total_tokens: Some(payload.total_tokens),
        prompt_tokens: Some(payload.prompt_tokens),
        completion_tokens: Some(payload.completion_tokens),
        start_time: isoformat(parts.stamps.start),
        end_time: isoformat(parts.stamps.end),
        completion_start_time: Some(isoformat(parts.stamps.completion_start)),
        model: payload.model.clone(),
        model_id: payload.model_id.clone().unwrap_or_default(),
        model_group: payload.model_group.clone().unwrap_or_default(),
        mcp_namespaced_tool_name: namespaced_tool(clean)?,
        billing_agent_id: clean.billing_agent_id.clone().flatten(),
        // metadata.get("agent_id"): an event carries no agent_id kwarg to win over it
        agent_id: optional_text_field(merged.get("agent_id"), "agent_id")?.map(ToOwned::to_owned),
        api_base: payload.api_base.clone(),
        user: text_or_empty(merged, "user_api_key_user_id")?,
        metadata: metadata_text(clean)?,
        cache_hit: cache_hit_text(payload.cache_hit),
        cache_key: parts.cache_key,
        request_tags: parts.request_tags,
        team_id: text_or_empty(merged, "user_api_key_team_id")?,
        organization_id: text_or_empty(merged, "user_api_key_org_id")?,
        end_user: end_user(payload, merged)?,
        requester_ip_address: clean.requester_ip_address.clone(),
        custom_llm_provider: payload.custom_llm_provider.clone().unwrap_or_default(),
        messages: parts.messages,
        response: storage_response(payload.response.as_ref(), None, NO_BODY_STORAGE),
        proxy_server_request: storage_request_body(&Json::Null, None, NO_BODY_STORAGE),
        session_id: Some(parts.session_id),
        request_duration_ms: parts.duration,
        status: status_for_spend_log(merged),
        litellm_call_id: payload.litellm_call_id.clone(),
    })
}

fn parts(payload: &StandardLoggingPayload) -> Result<Parts, ProjectionError> {
    let stamps = times(payload)?;
    let request_id = request_id(payload);
    let session = session_id(payload, &request_id);
    Ok(Parts {
        stamps,
        request_id,
        session_id: session,
        cache_key: payload
            .cache_key
            .clone()
            .unwrap_or_else(|| CACHE_OFF_KEY.to_owned()),
        request_tags: safe_dumps(
            &Json::List(payload.request_tags.clone()),
            DEFAULT_MAX_RECURSE_DEPTH,
            None,
        )
        .map_err(|source| ProjectionError::Json {
            record: "request_tags",
            source,
        })?,
        messages: storage_messages(
            payload.messages.as_ref(),
            NO_BODY_STORAGE.sanitize.store_bodies,
        ),
        duration: request_duration_ms(
            SpendTimestamp::Aware(stamps.start.fixed_offset()),
            SpendTimestamp::Aware(stamps.end.fixed_offset()),
        ),
    })
}

fn times(payload: &StandardLoggingPayload) -> Result<Times, ProjectionError> {
    Ok(Times {
        start: utc_at(payload.start_time).ok_or(ProjectionError::Time { field: "startTime" })?,
        end: utc_at(payload.end_time).ok_or(ProjectionError::Time { field: "endTime" })?,
        completion_start: utc_at(payload.completion_start_time).ok_or(ProjectionError::Time {
            field: "completionStartTime",
        })?,
    })
}

// the standard payload carries epoch seconds with microsecond precision, and spend logs store the isoformat text
fn utc_at(seconds: Decimal) -> Option<DateTime<Utc>> {
    let micros = seconds
        .checked_mul(Decimal::from(1_000_000_i64))?
        .round()
        .to_i64()?;
    DateTime::<Utc>::from_timestamp_micros(micros)
}

// LiteLLM's kwargs carry the LLM cost alone and null when the calculator failed; the payload's response_cost adds the guardrail cost
fn spend(payload: &StandardLoggingPayload) -> Option<Decimal> {
    if payload.cost_failure_info.is_some() {
        return None;
    }
    let charged = payload
        .cost_breakdown
        .as_ref()
        .and_then(|fields| fields.guardrail_cost);
    charged.map_or(Some(payload.response_cost), |amount| {
        payload.response_cost.checked_sub(amount)
    })
}

// get_spend_logs_id and the cache-hit suffix over the event: the response id, then the payload id, then the call id
fn request_id(payload: &StandardLoggingPayload) -> String {
    let response_id = payload
        .response
        .as_ref()
        .and_then(as_object)
        .and_then(|response| response.get("id"))
        .and_then(as_text);
    let candidates = [
        response_id,
        Some(payload.id.as_str()),
        payload.litellm_call_id.as_deref(),
    ];
    let mut id = candidates
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
        .unwrap_or(MISSING_ID)
        .to_owned();
    if payload.call_type == ARETRIEVE_BATCH {
        id.push_str(BATCH_COST_REQUEST_ID_SUFFIX);
    }
    // time.time() is not recorded in an event, so the request's end instant stands in for it
    if payload.cache_hit == Some(true) {
        let epoch = payload.end_time.to_f64().unwrap_or(0.0_f64);
        id.push_str(CACHE_HIT_SUFFIX);
        id.push_str(&float_text(epoch));
    }
    id
}

// _get_session_id_for_spend_log without the omit policy: a batch's id groups its rows, else the trace id
fn session_id(payload: &StandardLoggingPayload, request_id: &str) -> String {
    batch_trace_session_id(Some(payload.call_type.as_str()), Some(request_id))
        .unwrap_or_else(|| payload.trace_id.clone())
}

// The merged metadata key is hashed unless the trusted hash proves it already is a safe value.
fn api_key(payload: &StandardLoggingPayload, clean: &SpendLogsMetadata) -> String {
    clean
        .hashed_user_api_key
        .clone()
        .filter(|key| !key.is_empty())
        .unwrap_or_else(|| {
            redact_logged_api_key(
                payload.metadata.user_api_key_hash.as_deref(),
                true,
                payload.metadata.user_api_key_alias.as_deref(),
            )
            .unwrap_or_default()
        })
}

// get_end_user_id_for_cost_tracking's answer falls back to the standard payload's end user, then to "".
fn end_user(
    payload: &StandardLoggingPayload,
    merged: &JsonObject,
) -> Result<String, ProjectionError> {
    let field = "user_api_key_end_user_id";
    let resolved = optional_text_field(merged.get(field), field)?;
    if let Some(id) = resolved.filter(|id| !id.is_empty()) {
        return Ok(id.to_owned());
    }
    Ok(payload
        .metadata
        .user_api_key_end_user_id
        .clone()
        .unwrap_or_default())
}

fn text_or_empty(merged: &JsonObject, field: &'static str) -> Result<String, ProjectionError> {
    Ok(optional_text_field(merged.get(field), field)?
        .unwrap_or_default()
        .to_owned())
}

fn metadata_text(clean: &SpendLogsMetadata) -> Result<String, ProjectionError> {
    safe_dumps(clean, DEFAULT_MAX_RECURSE_DEPTH, None).map_err(|source| ProjectionError::Json {
        record: "metadata",
        source,
    })
}

// clean_metadata.get("mcp_tool_call_metadata"), then its namespaced_tool_name
fn namespaced_tool(clean: &SpendLogsMetadata) -> Result<Option<String>, ProjectionError> {
    let Some(Json::Object(fields)) = clean.mcp_tool_call_metadata.as_ref() else {
        return Ok(None);
    };
    Ok(
        optional_text_field(fields.get("namespaced_tool_name"), "namespaced_tool_name")?
            .map(ToOwned::to_owned),
    )
}

// str(cache_hit): Python capitalizes its booleans and writes None
fn cache_hit_text(value: Option<bool>) -> String {
    match value {
        Some(true) => "True".to_owned(),
        Some(false) => "False".to_owned(),
        None => "None".to_owned(),
    }
}
