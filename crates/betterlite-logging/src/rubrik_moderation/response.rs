use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::rubrik::block_decision::extract_response_block;
use crate::rubrik::moderation_payload::{
    ResponseModerationPayload, build_response_moderation, join_texts,
};
use crate::rubrik::request_data::{correlation_id, extract_request_data, resolve_model};
use crate::rubrik::tool_calls::{ChatCompletionMessageToolCall, normalize_tool_calls};
use crate::rubrik_delivery::logger::RubrikLogger;
use crate::rubrik_delivery::post::post_response_moderation;

use super::context::{
    LoggingObject, RequestData, call_details, request_context, request_data_json,
    stash_block_context, text_field,
};
use super::error::{ModerationError, ModifyResponseException};
use super::inputs::GuardrailInputs;
use super::service::service_response;

pub async fn moderate_response(
    logger: &RubrikLogger,
    inputs: GuardrailInputs,
    request_data: &mut RequestData,
    logging_obj: Option<&Arc<LoggingObject>>,
) -> Result<GuardrailInputs, ModerationError> {
    if inputs.tool_calls.is_empty() && inputs.texts.is_empty() {
        return Ok(inputs);
    }
    let message_tool_calls = normalize_tool_calls(&inputs.tool_calls)?;
    let sent_content = join_texts(&inputs.texts);

    let details = call_details(logging_obj);
    if logging_obj.is_some() && details.is_empty() {
        tracing::warn!(
            "Rubrik: logging_obj present but model_call_details is empty -- request context will be missing"
        );
    }
    let (response_data, request_body) = build_payloads(
        logger,
        request_data,
        &details,
        &message_tool_calls,
        &sent_content,
    )?;

    let fields = post_response_moderation(logger, &response_data, &request_body).await?;
    let block = extract_response_block(
        &service_response(&fields)?,
        &message_tool_calls,
        &sent_content,
    )?;
    let Some(blocked) = block else {
        return Ok(inputs);
    };
    let model = resolve_model(&request_data.response, text_field(&details, "model")?);
    stash_block_context(logging_obj, request_data);
    Err(ModerationError::Blocked(Box::new(
        ModifyResponseException {
            message: blocked.explanation,
            model,
            request_data: Box::new(request_data.clone()),
            guardrail_name: logger.settings.guardrail_name.clone(),
        },
    )))
}

fn build_payloads(
    logger: &RubrikLogger,
    request_data: &RequestData,
    details: &IndexMap<String, Json>,
    tool_calls: &[ChatCompletionMessageToolCall],
    sent_content: &str,
) -> Result<(ResponseModerationPayload, Json), ModerationError> {
    let request_id = correlation_id(
        text_field(details, "litellm_call_id")?,
        request_data.litellm_call_id.as_deref(),
    );
    let created = i64::try_from(logger.clock.now_unix().as_secs())
        .map_err(|_overflow| ModerationError::Timestamp)?;
    let response_data =
        build_response_moderation(tool_calls, sent_content, request_id.as_deref(), created);
    let call_source = if details.is_empty() {
        None
    } else {
        Some(request_context(details)?)
    };
    let request_source = if request_data.is_empty() {
        None
    } else {
        Some(request_data.request_context_source())
    };
    let request_body = match extract_request_data(call_source.as_ref(), request_source.as_ref()) {
        None => Json::Object(IndexMap::new()),
        Some(payload) => request_data_json(payload.messages, payload.model, payload.tools),
    };
    Ok((response_data, request_body))
}
