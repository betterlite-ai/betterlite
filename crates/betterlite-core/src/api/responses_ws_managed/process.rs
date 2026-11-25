use betterlite_error::error::{ErrorClass, class_message};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;

use super::history::save_turn_history;
use super::provider::ResponsesStreaming;
use super::stream::stream_and_forward;
use crate::api::responses_ws_frame_policy::quota::enforce_frame_project_quota;
use crate::api::responses_ws_history::call_kwargs::{
    build_base_call_kwargs, inject_credentials, update_proxy_request,
};
use crate::api::responses_ws_history::error::ResponsesWsHistoryError;
use crate::api::responses_ws_history::history::{apply_history, get_history_messages};
use crate::api::responses_ws_history::messages::input_to_messages;
use crate::api::responses_ws_history::serialization::send_error;
use crate::api::responses_ws_history::warmup::{is_warmup_frame, send_warmup_ack};
use crate::api::responses_ws_history::{FrameSink, ManagedResponsesWebSocket};

pub const RATE_LIMIT_EXCEEDED: &str = "rate_limit_exceeded";

// what leaves _process_response_create for run's unexpected-error path
#[derive(Debug, Error)]
pub enum ManagedTurnError {
    #[error(transparent)]
    History(#[from] ResponsesWsHistoryError),
    // only a rate limit becomes a frame here; every other quota failure reaches the connection loop
    #[error("{}", class_message(.0))]
    Quota(ErrorClass),
}

// the provider call one frame builds, with the messages its history save needs
struct PreparedCall {
    model: String,
    call_kwargs: IndexMap<String, Json>,
    prior_history: Vec<IndexMap<String, Json>>,
    current_messages: Vec<IndexMap<String, Json>>,
}

pub async fn process_response_create<Sink, Provider>(
    handler: &mut ManagedResponsesWebSocket<Sink>,
    provider: &Provider,
    raw_message: &str,
) -> Result<(), ManagedTurnError>
where
    Sink: FrameSink,
    Provider: ResponsesStreaming,
{
    let Some(message) = accepted_message(handler, raw_message).await? else {
        return Ok(());
    };
    let prepared = prepare_call(handler, &message)?;
    let forwarded =
        stream_and_forward(handler, provider, &prepared.model, &prepared.call_kwargs).await;
    let completed = match forwarded {
        Ok(completed) => completed,
        Err(error) => {
            tracing::error!("ManagedResponsesWS: error processing response.create: {error}");
            send_error(handler, &error.to_string(), None).await;
            return Ok(());
        }
    };
    save_turn_history(
        handler,
        completed.as_ref(),
        &prepared.prior_history,
        &prepared.current_messages,
    )?;
    Ok(())
}

// the message a turn processes: invalid JSON is answered, a warmup is acknowledged, a refused frame is answered
async fn accepted_message<Sink>(
    handler: &ManagedResponsesWebSocket<Sink>,
    raw_message: &str,
) -> Result<Option<IndexMap<String, Json>>, ManagedTurnError>
where
    Sink: FrameSink,
{
    let Some(message) = handler.parse_message(raw_message).await? else {
        return Ok(None);
    };
    if is_warmup_frame(&message) {
        if let Err(error) = send_warmup_ack(handler, &message).await {
            tracing::debug!("ManagedResponsesWS: error sending warmup ack: {error}");
        }
        return Ok(None);
    }
    if !admits_frame(handler, raw_message).await? {
        return Ok(None);
    }
    Ok(Some(message))
}

// the call kwargs one frame builds: routed model, applied history, credentials and proxy body
fn prepare_call<Sink>(
    handler: &ManagedResponsesWebSocket<Sink>,
    message: &IndexMap<String, Json>,
) -> Result<PreparedCall, ManagedTurnError>
where
    Sink: FrameSink,
{
    let mut call_kwargs = build_base_call_kwargs(message);
    call_kwargs.insert("stream".to_owned(), Json::Bool(true));
    let requested_model = optional_string(call_kwargs.shift_remove("model"));
    let model = routed_model(handler, requested_model.as_deref());
    let previous_response_id = optional_string(call_kwargs.shift_remove("previous_response_id"));
    let input = call_kwargs.get("input").cloned().unwrap_or(Json::Null);
    let current_messages = input_to_messages(&input);
    let prior_history = previous_response_id
        .as_deref()
        .map_or_else(Vec::new, |previous| get_history_messages(handler, previous));
    apply_history(
        &mut call_kwargs,
        previous_response_id.as_deref(),
        &current_messages,
        &prior_history,
    );
    inject_credentials(handler, &mut call_kwargs, Some(&model))?;
    update_proxy_request(
        &mut call_kwargs,
        proxy_model(handler, requested_model.as_deref(), &model),
    )?;
    call_kwargs.extend(handler.extra_kwargs.clone());
    Ok(PreparedCall {
        model,
        call_kwargs,
        prior_history,
        current_messages,
    })
}

// the quota gate runs before any provider preparation, and only a rate limit answers with a frame
async fn admits_frame<Sink>(
    handler: &ManagedResponsesWebSocket<Sink>,
    raw_message: &str,
) -> Result<bool, ManagedTurnError>
where
    Sink: FrameSink,
{
    let groups = handler.model_group.as_deref();
    let requested_model = groups.filter(|group| !group.is_empty());
    let quota = enforce_frame_project_quota(
        &handler.quota_callbacks,
        handler.user_api_key_dict.as_ref(),
        Some(requested_model.unwrap_or(&handler.model)),
        raw_message,
    )
    .await;
    match quota {
        Ok(()) => Ok(true),
        Err(error) => {
            if !matches!(error, ErrorClass::RateLimit(_)) {
                return Err(ManagedTurnError::Quota(error));
            }
            let message = class_message(&error);
            send_error(handler, &message, Some(RATE_LIMIT_EXCEEDED)).await;
            Ok(false)
        }
    }
}

// a non-string model or previous_response_id is popped and dropped, since the wire is caller-controlled
fn optional_string(value: Option<Json>) -> Option<String> {
    match value {
        Some(Json::Text(text)) => Some(text),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

// a frame repeating the public alias reuses the router-resolved connection model
fn routed_model<Sink>(
    handler: &ManagedResponsesWebSocket<Sink>,
    requested_model: Option<&str>,
) -> String
where
    Sink: FrameSink,
{
    match requested_model {
        Some(requested) if handler.model_group.as_deref() != Some(requested) => {
            requested.to_owned()
        }
        Some(_) | None => handler.model.clone(),
    }
}

fn proxy_model<'model, Sink>(
    handler: &'model ManagedResponsesWebSocket<Sink>,
    requested_model: Option<&'model str>,
    model: &'model str,
) -> &'model str
where
    Sink: FrameSink,
{
    let requested = requested_model.filter(|requested| !requested.is_empty());
    let groups = handler
        .model_group
        .as_deref()
        .filter(|group| !group.is_empty());
    requested.or(groups).unwrap_or(model)
}
