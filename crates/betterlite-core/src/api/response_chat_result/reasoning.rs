use std::slice;

use betterlite_openai::api::responses_objects::ResponseOutputItem;
use betterlite_openai::api::responses_objects::sdk::text::Reasoning;
use betterlite_openai::api::responses_objects::sdk::unions::SdkOutput;
use betterlite_prompts::reasoning_fields::replay_groups::ReasoningItemKind;
use serde_json::Value;

use super::error::{ResponseChatResultError as Error, serialized};
use super::records::{ChatChoice, ChatMessage, ChatRole};
use crate::api::response_chat_history::metadata::IncompleteFinish;
use crate::api::response_chat_history::reasoning::{
    ReasoningOutputItem, ReasoningRecord, ReasoningSummaryBlock, chat_reasoning_items,
    reasoning_records,
};

pub(super) struct PendingReasoning {
    pub(super) text: String,
    pub(super) item: ReasoningRecord,
}

pub(super) fn pending(reasoning: &Reasoning) -> PendingReasoning {
    let item = typed_record(reasoning);
    PendingReasoning {
        text: reasoning_text(slice::from_ref(&item)),
        item,
    }
}

pub(super) fn empty_incomplete_choice(
    output: &[ResponseOutputItem],
    finish: IncompleteFinish,
) -> Result<ChatChoice, Error> {
    let mut records = Vec::new();
    for item in output {
        records.extend(item_reasoning(item)?);
    }
    let text = reasoning_text(&records);
    let mut message = ChatMessage {
        content: Some(String::new()),
        role: ChatRole::Assistant,
        tool_calls: None,
        function_call: None,
        provider_specific_fields: None,
        reasoning_content: None,
        reasoning_items: None,
        annotations: None,
    };
    if !text.is_empty() {
        message.reasoning_content = Some(text);
    }
    message.reasoning_items = chat_reasoning_items(&records);
    Ok(ChatChoice {
        finish_reason: finish_text(finish).to_owned(),
        index: 0,
        message,
    })
}

pub(super) const fn finish_text(finish: IncompleteFinish) -> &'static str {
    match finish {
        IncompleteFinish::Length => "length",
        IncompleteFinish::ContentFilter => "content_filter",
    }
}

// Summary's tag type declares summary_text as its only value, the one every typed block carries
fn typed_record(reasoning: &Reasoning) -> ReasoningRecord {
    ReasoningRecord {
        kind: ReasoningItemKind::Reasoning,
        id: reasoning.id.clone(),
        encrypted_content: reasoning.encrypted_content.clone(),
        summary: reasoning
            .summary
            .iter()
            .map(|block| ReasoningSummaryBlock {
                kind: "summary_text".to_owned(),
                text: block.text.clone(),
            })
            .collect(),
    }
}

fn reasoning_text(records: &[ReasoningRecord]) -> String {
    let mut texts = Vec::new();
    for record in records {
        for block in &record.summary {
            if !block.text.is_empty() {
                texts.push(block.text.as_str());
            }
        }
    }
    texts.join(" ")
}

fn item_reasoning(item: &ResponseOutputItem) -> Result<Vec<ReasoningRecord>, Error> {
    if let ResponseOutputItem::Sdk(sdk) = item
        && let SdkOutput::Reasoning(reasoning) = &**sdk
    {
        return Ok(vec![typed_record(reasoning)]);
    }
    let Value::Object(fields) = serialized(item)? else {
        return Ok(Vec::new());
    };
    reasoning_records(&[ReasoningOutputItem::Dict { value: fields }])
        .map_err(|source| Error::History { source })
}
