use betterlite_config::spend_storage::MissingSessionId;
use betterlite_logging::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};
use betterlite_logging::spend_bodies::SpendStoragePolicy;
use betterlite_logging::spend_bodies::messages::storage_messages;
use betterlite_logging::spend_bodies::placeholder::placeholder_request_body;
use betterlite_logging::spend_bodies::request::{RequestKwargs, storage_request_body};
use betterlite_logging::spend_bodies::response::{ResponseKwargs, storage_response};
use betterlite_logging::spend_bodies::session::{
    SessionInput, batch_trace_session_id, omits_session_id_when_missing, request_duration_ms,
    session_id,
};
use betterlite_logging::spend_metadata::records::SpendLogsMetadata;
use betterlite_logging::spend_metadata::{
    SpendLogsPayload, optional_text_field, status_for_spend_log,
};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use chrono::TimeZone;
use rust_decimal::Decimal;

use super::{
    CompletionStart, Context, Pieces, SpendPayloadError, SpendPayloadSettings, cache_hit_text,
    counter, instant,
};

// The SpendLogsPayload literal, one field per entry the source passes, with the times normalized to UTC.
pub(crate) fn record<T>(
    context: &Context<'_>,
    settings: &SpendPayloadSettings,
    zone: &T,
    pieces: &Pieces,
) -> Result<SpendLogsPayload, SpendPayloadError>
where
    T: TimeZone,
{
    let start = instant(context.request.start_time, zone, "startTime")?;
    let end = instant(context.request.end_time, zone, "endTime")?;
    let completion_start = match context.request.completion_start_time {
        CompletionStart::Absent => Some(end),
        CompletionStart::Null => None,
        CompletionStart::At(moment) => Some(instant(moment, zone, "completionStartTime")?),
    };
    let params = Json::Object(context.litellm_params.clone());
    let metadata_object = Json::Object(context.metadata.clone());
    let kwargs_object = Json::Object(context.kwargs.clone());
    let (total, prompt, completion) = token_totals(context, &pieces.usage)?;
    Ok(SpendLogsPayload {
        request_id: pieces.request_id.clone(),
        call_type: context.call_type.unwrap_or("").to_owned(),
        api_key: pieces.envelope.api_key.clone(),
        spend: spend(context)?,
        total_tokens: total,
        prompt_tokens: prompt,
        completion_tokens: completion,
        start_time: start,
        end_time: end,
        completion_start_time: completion_start,
        model: pieces.attribution.model_name.clone(),
        model_id: pieces.attribution.model_id.clone(),
        model_group: pieces.attribution.model_group.clone(),
        mcp_namespaced_tool_name: namespaced_tool(&pieces.clean)?,
        billing_agent_id: pieces.clean.billing_agent_id.clone().flatten(),
        agent_id: agent_id(context)?,
        api_base: pieces.attribution.api_base.clone(),
        user: pieces.envelope.user.clone(),
        metadata: metadata_text(&pieces.clean)?,
        cache_hit: cache_hit_text(context.cache_hit),
        cache_key: pieces.cache_key.clone(),
        request_tags: pieces.envelope.request_tags.clone(),
        team_id: pieces.envelope.team_id.clone(),
        organization_id: pieces.envelope.organization_id.clone(),
        end_user: pieces.envelope.end_user.clone(),
        requester_ip_address: pieces.clean.requester_ip_address.clone(),
        custom_llm_provider: pieces.attribution.provider.clone().unwrap_or_default(),
        messages: storage_messages(context.standard_raw, settings.storage.sanitize.store_bodies),
        response: storage_response(
            context.standard_raw,
            Some(&ResponseKwargs {
                realtime_tool_calls: context.kwargs.get("realtime_tool_calls"),
                litellm_params: Some(&params),
                dynamic_params: context.kwargs.get("standard_callback_dynamic_params"),
            }),
            settings.storage,
        ),
        proxy_server_request: request_body(context, settings.storage, pieces, &params),
        session_id: session(context, settings, pieces, &kwargs_object, &metadata_object),
        request_duration_ms: request_duration_ms(
            context.request.start_time,
            context.request.end_time,
        ),
        status: status_for_spend_log(&context.metadata),
        litellm_call_id: pieces.attribution.call_id.clone(),
    })
}

fn request_body(
    context: &Context<'_>,
    policy: SpendStoragePolicy,
    pieces: &Pieces,
    params: &Json,
) -> String {
    let request_kwargs = RequestKwargs {
        standard_logging_object: context.standard_raw,
        realtime_tools: context.kwargs.get("realtime_tools"),
        dynamic_params: context.kwargs.get("standard_callback_dynamic_params"),
    };
    if !pieces.attribution.placeholdered {
        return storage_request_body(params, Some(&request_kwargs), policy);
    }
    let placeholdered = placeholder_request_body(
        params,
        &pieces.attribution.model_group,
        &pieces.attribution.raw_model,
    );
    storage_request_body(&placeholdered, Some(&request_kwargs), policy)
}

fn session(
    context: &Context<'_>,
    settings: &SpendPayloadSettings,
    pieces: &Pieces,
    kwargs: &Json,
    metadata: &Json,
) -> Option<String> {
    let configured = settings.missing_session_id == Some(MissingSessionId::Omit);
    let batch = batch_trace_session_id(context.call_type, Some(&pieces.request_id));
    session_id(&SessionInput {
        kwargs: Some(kwargs),
        metadata: Some(metadata),
        standard_logging_payload: context.standard_raw,
        omit_when_missing: omits_session_id_when_missing(Some(metadata), configured),
        batch_trace_session_id: batch.as_deref(),
        injected_uuid: context.request.session_uuid,
    })
}

fn metadata_text(clean: &SpendLogsMetadata) -> Result<String, SpendPayloadError> {
    safe_dumps(clean, DEFAULT_MAX_RECURSE_DEPTH, None).map_err(|source| SpendPayloadError::Json {
        record: "metadata",
        source,
    })
}

// kwargs.get("response_cost", 0): absent reads as zero, null stays null, and a wrong type is named.
fn spend(context: &Context<'_>) -> Result<Option<Decimal>, SpendPayloadError> {
    match context.kwargs.get("response_cost") {
        None => Ok(Some(Decimal::ZERO)),
        Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => Ok(Some(parse_decimal_lexeme(&number.to_string())?)),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(SpendPayloadError::FieldType {
                field: "response_cost",
            })
        }
    }
}

// usage.get(field, standard): a source usage field wins by presence, so a null there stays null.
fn token_totals(
    context: &Context<'_>,
    usage: &JsonObject,
) -> Result<(Option<i64>, Option<i64>, Option<i64>), SpendPayloadError> {
    let standard = context.standard.as_ref();
    let fallback = |field: fn(&super::source::StandardSource) -> Option<Option<i64>>| {
        standard.map_or(Some(0), |source| field(source).unwrap_or(Some(0)))
    };
    Ok((
        counted(
            usage,
            "total_tokens",
            fallback(|source| source.total_tokens),
        )?,
        counted(
            usage,
            "prompt_tokens",
            fallback(|source| source.prompt_tokens),
        )?,
        counted(
            usage,
            "completion_tokens",
            fallback(|source| source.completion_tokens),
        )?,
    ))
}

fn counted(
    usage: &JsonObject,
    field: &'static str,
    standard: Option<i64>,
) -> Result<Option<i64>, SpendPayloadError> {
    match usage.get(field) {
        Some(value) => counter(Some(value), field),
        None => Ok(standard),
    }
}

// clean_metadata.get("mcp_tool_call_metadata"), then its namespaced_tool_name.
fn namespaced_tool(clean: &SpendLogsMetadata) -> Result<Option<String>, SpendPayloadError> {
    let Some(Json::Object(fields)) = clean.mcp_tool_call_metadata.as_ref() else {
        return Ok(None);
    };
    Ok(
        optional_text_field(fields.get("namespaced_tool_name"), "namespaced_tool_name")?
            .map(ToOwned::to_owned),
    )
}

// kwargs.get("agent_id") or metadata.get("agent_id"): an empty kwarg falls through to the metadata's value.
fn agent_id(context: &Context<'_>) -> Result<Option<String>, SpendPayloadError> {
    let kwarg = optional_text_field(context.kwargs.get("agent_id"), "agent_id")?;
    match kwarg.filter(|id| !id.is_empty()) {
        Some(id) => Ok(Some(id.to_owned())),
        None => Ok(
            optional_text_field(context.metadata.get("agent_id"), "agent_id")?
                .map(ToOwned::to_owned),
        ),
    }
}
