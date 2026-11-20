use std::collections::BTreeMap;

use betterlite_bedrock_guardrails::constants::BEDROCK_MAX_CONTENT_BLOCKS;

use crate::bedrock_message_selection::content;
use crate::bedrock_message_selection::wire::{Message, ModelResponse};
use crate::bedrock_records::{ChecksMessage, ChecksTextContent, Role, Source};

// LiteLLM's _chunk_texts_into_checks_messages: every text becomes a block and no block is dropped
#[must_use]
pub fn chunk_texts(role: Role, texts: &[String]) -> Vec<ChecksMessage> {
    texts
        .chunks(BEDROCK_MAX_CONTENT_BLOCKS)
        .map(|chunk| ChecksMessage {
            role: Some(role),
            content: Some(chunk.iter().map(|text| block(text.as_str())).collect()),
            extra: BTreeMap::new(),
        })
        .collect()
}

fn block(text: &str) -> ChecksTextContent {
    ChecksTextContent {
        text: Some(text.to_owned()),
        extra: BTreeMap::new(),
    }
}

// LiteLLM's _build_invoke_guardrail_checks_messages
#[must_use]
pub fn build(
    source: Source,
    messages: Option<&[Message]>,
    response: Option<&ModelResponse>,
) -> Vec<ChecksMessage> {
    match source {
        Source::Input => input(messages),
        Source::Output => output(response),
    }
}

// every INPUT block is retagged user, so a caller's system label cannot skip the promptAttack check
fn input(messages: Option<&[Message]>) -> Vec<ChecksMessage> {
    messages
        .unwrap_or_default()
        .iter()
        .flat_map(|message| chunk_texts(Role::User, &texts(message)))
        .collect()
}

fn texts(message: &Message) -> Vec<String> {
    content::extract(message)
        .unwrap_or_default()
        .into_iter()
        .map(|block| block.text)
        .filter(|text| !text.is_empty())
        .collect()
}

fn output(response: Option<&ModelResponse>) -> Vec<ChecksMessage> {
    let request = content::output(response, None, false);
    let texts: Vec<String> = request
        .content
        .unwrap_or_default()
        .into_iter()
        .filter_map(|item| item.text)
        .filter_map(|text| text.text)
        .filter(|text| !text.is_empty())
        .collect();
    chunk_texts(Role::Assistant, &texts)
}
