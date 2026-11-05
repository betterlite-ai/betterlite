use chrono::SecondsFormat;

use super::events::{
    Artifact, ArtifactUpdateResult, HistoryMessage, StatusMessage, StatusRecord,
    StatusUpdateResult, StreamEvent, StreamResult, SubmittedStatus, TaskEventResult,
};
use super::wire::TextPart;
use super::{A2AStreamingContext, EventSource};

#[derive(Clone, Copy)]
pub enum EventContent<'text> {
    Task,
    Status {
        state: &'text str,
        final_event: bool,
        message: Option<&'text str>,
    },
    Artifact(&'text str),
}

pub fn stream_event<S>(
    ctx: &A2AStreamingContext,
    content: EventContent<'_>,
    source: &mut S,
) -> StreamEvent
where
    S: EventSource,
{
    let result = match content {
        EventContent::Task => StreamResult::Task(TaskEventResult {
            context_id: ctx.context_id.clone(),
            history: vec![history(ctx, source)],
            id: ctx.task_id.clone(),
            kind: "task".to_owned(),
            status: SubmittedStatus {
                state: "submitted".to_owned(),
            },
        }),
        EventContent::Status {
            state,
            final_event,
            message,
        } => StreamResult::Status(status(ctx, state, final_event, message, source)),
        EventContent::Artifact(text) => StreamResult::Artifact(ArtifactUpdateResult {
            artifact: Artifact {
                artifact_id: source.next_uuid().to_string(),
                name: "response".to_owned(),
                parts: vec![TextPart {
                    kind: "text".to_owned(),
                    text: text.to_owned(),
                }],
            },
            context_id: ctx.context_id.clone(),
            kind: "artifact-update".to_owned(),
            task_id: ctx.task_id.clone(),
        }),
    };
    StreamEvent::from((ctx.request_id.clone(), result))
}

fn history<S>(ctx: &A2AStreamingContext, source: &mut S) -> HistoryMessage
where
    S: EventSource,
{
    let message_id = ctx
        .input_message
        .message_id
        .clone()
        .unwrap_or_else(|| source.next_uuid().simple().to_string());
    HistoryMessage {
        context_id: ctx.context_id.clone(),
        kind: "message".to_owned(),
        message_id,
        parts: ctx.input_message.parts.clone(),
        role: ctx.input_message.role.0.clone(),
        task_id: ctx.task_id.clone(),
    }
}

fn status<S>(
    ctx: &A2AStreamingContext,
    state: &str,
    final_event: bool,
    message_text: Option<&str>,
    source: &mut S,
) -> StatusUpdateResult
where
    S: EventSource,
{
    let message = if state == "working" {
        message_text
            .filter(|text| !text.is_empty())
            .map(|text| StatusMessage {
                context_id: ctx.context_id.clone(),
                kind: "message".to_owned(),
                message_id: source.next_uuid().to_string(),
                parts: vec![TextPart {
                    kind: "text".to_owned(),
                    text: text.to_owned(),
                }],
                role: "agent".to_owned(),
                task_id: ctx.task_id.clone(),
            })
    } else {
        None
    };
    let now = source.now();
    let precision = if now.timestamp_subsec_micros() == 0 {
        SecondsFormat::Secs
    } else {
        SecondsFormat::Micros
    };
    StatusUpdateResult {
        context_id: ctx.context_id.clone(),
        final_event,
        kind: "status-update".to_owned(),
        status: StatusRecord {
            state: state.to_owned(),
            timestamp: now.to_rfc3339_opts(precision, false),
            message,
        },
        task_id: ctx.task_id.clone(),
    }
}
