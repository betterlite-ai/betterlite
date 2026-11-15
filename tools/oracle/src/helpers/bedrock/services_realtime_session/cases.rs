use betterlite_bedrock::invoke_primitives::environment::error_class;
use betterlite_bedrock::realtime_session::records::{SessionEvent, SessionEventType, UpdateFrame};
use betterlite_bedrock::realtime_session::{
    REQUIRES_SESSION_CONFIGURATION, RealtimeSession, complete_url, validate_environment,
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::FixedIds;
use super::wire::{event_summary, failure, header_pair, parse_ids, prompt_start_summary};
use crate::helpers::{HelperError, HelperResult};

#[derive(Debug, Clone, Copy)]
pub(super) enum Summary {
    Frames,
    PromptStart,
}

pub(super) fn update_values(input: &super::Input, ids: &[Uuid], summary: Summary) -> HelperResult {
    let source = FixedIds::new(ids);
    let mut session = RealtimeSession::new(&source);
    let mut results: Vec<Value> = Vec::new();
    for update in &input.updates {
        let frames = session.apply_update(update, &source);
        results.push(summarize(&session, &frames, summary)?);
    }
    Ok(update_body(&session, results))
}

fn summarize(session: &RealtimeSession, frames: &[UpdateFrame], summary: Summary) -> HelperResult {
    match summary {
        Summary::Frames => Ok(json!({
            "frames": serde_json::to_value(frames).map_err(failure)?,
            "input_audio": input_audio_summary(session),
        })),
        Summary::PromptStart => {
            let Some(UpdateFrame::PromptStart(frame)) = frames.get(1) else {
                return Err(failure("the second frame is promptStart"));
            };
            Ok(json!({ "prompt_start": prompt_start_summary(&frame.event.prompt_start)? }))
        }
    }
}

fn input_audio_summary(session: &RealtimeSession) -> Value {
    let audio = &session.input_audio;
    json!({
        "sample_rate_hertz": audio.sample_rate_hertz,
        "sample_size_bits": audio.sample_size_bits,
        "channel_count": audio.channel_count,
        "encoding": audio.encoding,
        "audio_type": audio.audio_type,
        "media_type": audio.media_type,
    })
}

fn update_body(session: &RealtimeSession, results: Vec<Value>) -> Value {
    json!({
        "updates": results,
        "prompt_name": session.names.prompt,
        "prompt_started": session.prompt_started,
        "client_audio_streamed": session.client_audio_streamed,
    })
}

pub(super) fn default_case(ids: &[Uuid]) -> HelperResult {
    let source = FixedIds::new(ids);
    let session = RealtimeSession::new(&source);
    let configuration = session.configuration_request(None);
    Ok(json!({
        "prompt_name": session.names.prompt,
        "content_name": session.names.content,
        "audio_content_name": session.names.audio_content,
        "prompt_started": session.prompt_started,
        "client_audio_streamed": session.client_audio_streamed,
        "configuration": serde_json::to_value(&configuration).map_err(failure)?,
        "prompt_started_after": session.prompt_started,
        "input_audio": input_audio_summary(&session),
    }))
}

pub(super) fn created_updated(input: &super::Input) -> HelperResult {
    let source = FixedIds::new(&parse_ids(&input.ids)?);
    let events = build_events(input, &source);
    let (error_status, error_synthesized) = error_summary(input)?;
    Ok(json!({
        "created": event_summary(&events.created)?,
        "updated_default": event_summary(&events.updated_default)?,
        "updated_empty": event_summary(&events.updated_empty)?,
        "updated_explicit": event_summary(&events.updated_explicit)?,
        "model_event": event_summary(&events.model_event)?,
        "requires_session_configuration": REQUIRES_SESSION_CONFIGURATION,
        "complete_urls": complete_urls(input),
        "validated_headers": validated_headers(input)?,
        "error_status": error_status,
        "error_synthesized": error_synthesized,
    }))
}

struct Events {
    created: SessionEvent,
    updated_default: SessionEvent,
    updated_empty: SessionEvent,
    updated_explicit: SessionEvent,
    model_event: SessionEvent,
}

fn build_events(input: &super::Input, source: &FixedIds) -> Events {
    let model = Some(input.model.as_str());
    let trace_id = input.trace_id.as_str();
    let created =
        RealtimeSession::session_event(SessionEventType::Created, model, trace_id, None, source);
    let updated_default =
        RealtimeSession::session_event(SessionEventType::Updated, model, trace_id, None, source);
    let no_modalities: &[String] = &[];
    let updated_empty = RealtimeSession::session_event(
        SessionEventType::Updated,
        model,
        trace_id,
        Some(no_modalities),
        source,
    );
    let updated_explicit = RealtimeSession::session_event(
        SessionEventType::Updated,
        model,
        trace_id,
        input.modalities.as_deref(),
        source,
    );
    let model_name = input.models.first().map_or("", String::as_str);
    let model_event = RealtimeSession::session_event(
        SessionEventType::Created,
        Some(model_name),
        trace_id,
        None,
        source,
    );
    Events {
        created,
        updated_default,
        updated_empty,
        updated_explicit,
        model_event,
    }
}

fn complete_urls(input: &super::Input) -> Vec<String> {
    input
        .urls
        .iter()
        .map(|url| complete_url(url.as_deref()))
        .collect()
}

fn validated_headers(input: &super::Input) -> Result<Vec<Value>, Box<HelperError>> {
    let mut caller: Vec<(String, String)> = Vec::new();
    for header in &input.headers {
        caller.push(header_pair(header)?);
    }
    let echoed = validate_environment(&caller);
    let mut validated: Vec<Value> = Vec::new();
    for (name, value) in echoed {
        validated.push(json!([name, value]));
    }
    Ok(validated)
}

fn error_summary(input: &super::Input) -> Result<(u16, bool), Box<HelperError>> {
    let Some(error) = &input.error else {
        return Err(failure(
            "the created_updated_events case needs an error fixture",
        ));
    };
    let mut pairs: Vec<(String, String)> = Vec::new();
    for header in &error.headers {
        pairs.push(header_pair(header)?);
    }
    let built = error_class(error.message.clone(), error.status, pairs).map_err(failure)?;
    Ok((
        built.error.status_code,
        built.error.status_code_is_synthesized,
    ))
}
