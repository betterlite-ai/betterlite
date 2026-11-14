use betterlite_bedrock::invoke_primitives::environment::error_class;
use betterlite_bedrock::realtime_session::records::UpdateFrame;
use betterlite_bedrock::realtime_session::{
    REQUIRES_SESSION_CONFIGURATION, RealtimeSession, complete_url,
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::FixedIds;
use super::wire::{event_summary, failure, header_pair, parse_ids, prompt_start_summary};
use crate::helpers::HelperResult;

pub(super) fn update_values(input: &super::Input, ids: &[Uuid]) -> HelperResult {
    let ids = FixedIds::new(ids);
    let mut session = RealtimeSession::new(&ids);
    let mut results: Vec<Value> = Vec::new();
    for update in &input.updates {
        let frames = session.apply_update(update, &ids);
        let value = serde_json::to_value(&frames).map_err(failure)?;
        results.push(json!({ "frames": value }));
    }
    Ok(json!({
        "updates": results,
        "prompt_name": session.names.prompt,
        "prompt_started": session.prompt_started,
        "client_audio_streamed": session.client_audio_streamed,
    }))
}

pub(super) fn tool_values(input: &super::Input, ids: &[Uuid]) -> HelperResult {
    let ids = FixedIds::new(ids);
    let mut session = RealtimeSession::new(&ids);
    let mut results: Vec<Value> = Vec::new();
    for update in &input.updates {
        let frames = session.apply_update(update, &ids);
        let prompt_start = match frames.get(1) {
            Some(UpdateFrame::PromptStart(frame)) => &frame.event.prompt_start,
            _ => return Err(failure("the second frame is promptStart")),
        };
        let summary = prompt_start_summary(prompt_start)?;
        results.push(json!({ "prompt_start": summary }));
    }
    Ok(json!({
        "updates": results,
        "prompt_name": session.names.prompt,
        "prompt_started": session.prompt_started,
        "client_audio_streamed": session.client_audio_streamed,
    }))
}

pub(super) fn default_case(ids: &[Uuid]) -> HelperResult {
    let ids = FixedIds::new(ids);
    let session = RealtimeSession::new(&ids);
    let configuration = session.configuration_request(None);
    Ok(json!({
        "prompt_name": session.names.prompt,
        "content_name": session.names.content,
        "audio_content_name": session.names.audio_content,
        "prompt_started": session.prompt_started,
        "client_audio_streamed": session.client_audio_streamed,
        "configuration": serde_json::to_value(&configuration).map_err(failure)?,
        "prompt_started_after": session.prompt_started,
    }))
}

pub(super) fn created_updated(input: &super::Input) -> HelperResult {
    let ids = FixedIds::new(&parse_ids(&input.ids)?);
    let created =
        RealtimeSession::session_created_event(Some(input.model.as_str()), &input.trace_id, &ids);
    let updated_default = RealtimeSession::session_updated_event(
        Some(input.model.as_str()),
        &input.trace_id,
        None,
        &ids,
    );
    let no_modalities: &[String] = &[];
    let updated_empty = RealtimeSession::session_updated_event(
        Some(input.model.as_str()),
        &input.trace_id,
        Some(no_modalities),
        &ids,
    );
    let updated_explicit = RealtimeSession::session_updated_event(
        Some(input.model.as_str()),
        &input.trace_id,
        input.modalities.as_deref(),
        &ids,
    );
    let model_name = input.models.first().map_or("", String::as_str);
    let model_event =
        RealtimeSession::session_created_event(Some(model_name), &input.trace_id, &ids);
    let urls: Vec<String> = input
        .urls
        .iter()
        .map(|url| complete_url(url.as_deref()))
        .collect();
    let mut validated: Vec<Value> = Vec::new();
    for header in &input.headers {
        let pair = header_pair(header)?;
        validated.push(json!([pair.0, pair.1]));
    }
    let error = match &input.error {
        Some(error) => {
            let mut pairs: Vec<(String, String)> = Vec::new();
            for header in &error.headers {
                pairs.push(header_pair(header)?);
            }
            let built = error_class(error.message.clone(), error.status, pairs).map_err(failure)?;
            (
                built.error.status_code,
                built.error.status_code_is_synthesized,
            )
        }
        None => (0, false),
    };
    Ok(json!({
        "created": event_summary(&created)?,
        "updated_default": event_summary(&updated_default)?,
        "updated_empty": event_summary(&updated_empty)?,
        "updated_explicit": event_summary(&updated_explicit)?,
        "model_event": event_summary(&model_event)?,
        "requires_session_configuration": REQUIRES_SESSION_CONFIGURATION,
        "complete_urls": urls,
        "validated_headers": validated,
        "error_status": error.0,
        "error_synthesized": error.1,
    }))
}
