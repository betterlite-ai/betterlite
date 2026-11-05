use super::wire::{Part, TaskResponse, TaskResult};
use crate::completion_bridge::EventSource;

mod tests;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResponseText {
    pub text: String,
    pub message_id: String,
    pub parts: Vec<Part>,
}

pub fn extract_response_text<S>(response: &TaskResponse, source: &mut S) -> ResponseText
where
    S: EventSource,
{
    let result = &response.result;
    if let Some(artifact) = artifact_text(result, source) {
        return artifact;
    }
    for message in result
        .history
        .iter()
        .rev()
        .filter(|message| message.role == "agent")
    {
        let parts = message.parts.as_deref().unwrap_or_default();
        let text = parts.iter().map(Part::text_content).collect::<String>();
        if !text.is_empty() {
            return ResponseText {
                text,
                message_id: message
                    .message_id
                    .clone()
                    .unwrap_or_else(|| source.next_uuid().to_string()),
                parts: parts.to_vec(),
            };
        }
    }
    let parts = result.message.parts.as_deref().unwrap_or_default();
    ResponseText {
        text: parts.iter().map(Part::text_content).collect(),
        message_id: result
            .message
            .message_id
            .clone()
            .unwrap_or_else(|| source.next_uuid().to_string()),
        parts: parts.to_vec(),
    }
}

fn artifact_text<S>(result: &TaskResult, source: &mut S) -> Option<ResponseText>
where
    S: EventSource,
{
    for artifact in &result.artifacts {
        let first_text = artifact
            .parts
            .iter()
            .map(Part::text_content)
            .find(|text| !text.is_empty());
        if let Some(text) = first_text {
            return Some(ResponseText {
                text: text.to_owned(),
                message_id: source.next_uuid().to_string(),
                parts: artifact.parts.clone(),
            });
        }
    }
    None
}
