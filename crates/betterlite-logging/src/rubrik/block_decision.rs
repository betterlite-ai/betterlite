use std::collections::HashMap;

use thiserror::Error;

use crate::rubrik::tool_calls::ChatCompletionMessageToolCall;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModerationServiceResponse {
    pub choices: Vec<ModerationChoice>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModerationChoice {
    pub message: Option<ModerationMessage>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModerationMessage {
    pub content: Option<String>,
    pub tool_calls: Vec<ReturnedToolCall>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReturnedToolCall {
    pub id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockedResponseResult {
    pub explanation: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ResponseBlockError {
    #[error("response moderation service returned no choices")]
    MalformedResponse,
}

// choices[0].message.content, defaulting to the generic refusal explanation when the moderation service omitted its own.
#[must_use]
pub fn extract_prompt_refusal(response: &ModerationServiceResponse) -> Option<String> {
    let first_choice = response.choices.first()?;
    let content = first_choice
        .message
        .as_ref()
        .and_then(|message| message.content.as_deref());
    Some(
        non_empty(content)
            .unwrap_or("Request blocked by policy.")
            .to_owned(),
    )
}

// None means nothing was blocked: the returned tool calls account for every declared tool call and the returned text matches (or extends) what was sent.
pub fn extract_response_block(
    response: &ModerationServiceResponse,
    all_tool_calls: &[ChatCompletionMessageToolCall],
    sent_content: &str,
) -> Result<Option<BlockedResponseResult>, ResponseBlockError> {
    let first_choice = response
        .choices
        .first()
        .ok_or(ResponseBlockError::MalformedResponse)?;
    let moderation_message = first_choice.message.as_ref();
    let returned_tool_calls: &[ReturnedToolCall] =
        moderation_message.map_or(&[], |value| value.tool_calls.as_slice());
    let returned_content = moderation_message
        .and_then(|value| value.content.as_deref())
        .unwrap_or("");

    let tools_blocked = tools_were_blocked(returned_tool_calls, all_tool_calls);
    let separator_prefix = format!("{sent_content}\n\n");
    let appended = !sent_content.is_empty() && returned_content.starts_with(&separator_prefix);
    let text_blocked = !sent_content.is_empty() && returned_content != sent_content && !appended;

    if text_blocked {
        return Ok(Some(BlockedResponseResult {
            explanation: non_empty(Some(returned_content))
                .unwrap_or("Response blocked by policy.")
                .to_owned(),
        }));
    }
    if tools_blocked {
        let explanation = if appended {
            returned_content
                .get(sent_content.len()..)
                .unwrap_or("")
                .trim_start_matches('\n')
        } else {
            returned_content
        };
        return Ok(Some(BlockedResponseResult {
            explanation: non_empty(Some(explanation))
                .unwrap_or("Tool call blocked by policy.")
                .to_owned(),
        }));
    }
    Ok(None)
}

fn tools_were_blocked(
    returned_tool_calls: &[ReturnedToolCall],
    all_tool_calls: &[ChatCompletionMessageToolCall],
) -> bool {
    if returned_tool_calls.len() < all_tool_calls.len() {
        return true;
    }
    let returned_id_counts = count_ids(
        returned_tool_calls
            .iter()
            .filter_map(|tool_call| tool_call.id.as_deref())
            .filter(|id| !id.is_empty()),
    );
    let required_id_counts = count_ids(
        all_tool_calls
            .iter()
            .map(|tool_call| tool_call.id.as_str())
            .filter(|id| !id.is_empty()),
    );
    required_id_counts
        .iter()
        .any(|(id, count)| returned_id_counts.get(id).copied().unwrap_or(0) < *count)
}

fn count_ids<'ids>(ids: impl Iterator<Item = &'ids str>) -> HashMap<&'ids str, usize> {
    let mut counts = HashMap::new();
    for id in ids {
        counts
            .entry(id)
            .and_modify(|count: &mut usize| *count = count.saturating_add(1))
            .or_insert(1);
    }
    counts
}

const fn non_empty(text: Option<&str>) -> Option<&str> {
    match text {
        Some(value) if !value.is_empty() => Some(value),
        _ => None,
    }
}
