use std::collections::BTreeMap;
use std::time::Duration;

use serde::Serialize;

use crate::completion_bridge::EventSource;
use crate::completion_bridge::events::Event;
use crate::pydantic_ai_transformation::fake_streaming::StreamPacing;
use crate::pydantic_ai_transformation::fake_streaming::events::{
    ArtifactChunk, ArtifactUpdateResult, StatusState, StatusUpdateResult, TaskState,
};
use crate::pydantic_ai_transformation::wire::Part;

mod tests;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmittedTask {
    pub context_id: String,
    pub id: String,
    pub kind: &'static str,
    pub status: StatusState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum WxoStreamResult {
    Task(SubmittedTask),
    Status(StatusUpdateResult),
    Artifact(ArtifactUpdateResult),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WxoStreamStep {
    pub event: Event<WxoStreamResult>,
    pub delay_after: Duration,
}

struct StreamIds {
    request: String,
    task: String,
    context: String,
    artifact: String,
}

impl StreamIds {
    fn step(&self, result: WxoStreamResult, delay_after: Duration) -> WxoStreamStep {
        WxoStreamStep {
            event: Event::from((self.request.clone(), result)),
            delay_after,
        }
    }

    fn status(&self, state: TaskState, final_event: bool, delay_after: Duration) -> WxoStreamStep {
        let result = WxoStreamResult::Status(StatusUpdateResult {
            context_id: self.context.clone(),
            final_event,
            kind: "status-update",
            status: StatusState { state },
            task_id: self.task.clone(),
        });
        self.step(result, delay_after)
    }

    fn artifact(&self, text: String, delay_after: Duration) -> WxoStreamStep {
        let part = Part {
            kind: Some("text".to_owned()),
            text: Some(text),
            extra: BTreeMap::new(),
        };
        let result = WxoStreamResult::Artifact(ArtifactUpdateResult {
            context_id: self.context.clone(),
            kind: "artifact-update",
            task_id: self.task.clone(),
            artifact: ArtifactChunk {
                artifact_id: self.artifact.clone(),
                parts: vec![part],
            },
        });
        self.step(result, delay_after)
    }
}

pub fn fake_streaming_from_text<S>(
    text: &str,
    request_id: &str,
    pacing: StreamPacing,
    source: &mut S,
) -> Vec<WxoStreamStep>
where
    S: EventSource,
{
    let ids = StreamIds {
        request: request_id.to_owned(),
        task: source.next_uuid().to_string(),
        context: source.next_uuid().to_string(),
        artifact: source.next_uuid().to_string(),
    };
    let submitted = WxoStreamResult::Task(SubmittedTask {
        context_id: ids.context.clone(),
        id: ids.task.clone(),
        kind: "task",
        status: StatusState {
            state: TaskState::Submitted,
        },
    });
    let characters = text.chars().collect::<Vec<_>>();
    let mut chunks = characters
        .chunks(pacing.chunk_size.get())
        .map(|chunk| chunk.iter().collect::<String>())
        .collect::<Vec<_>>();
    // empty text still produces one empty artifact
    let last_chunk = chunks.pop().unwrap_or_default();
    let mut steps = vec![
        ids.step(submitted, Duration::ZERO),
        ids.status(TaskState::Working, false, pacing.delay),
    ];
    steps.extend(
        chunks
            .into_iter()
            .map(|chunk| ids.artifact(chunk, pacing.delay)),
    );
    steps.push(ids.artifact(last_chunk, Duration::ZERO));
    steps.push(ids.status(TaskState::Completed, true, Duration::ZERO));
    steps
}
