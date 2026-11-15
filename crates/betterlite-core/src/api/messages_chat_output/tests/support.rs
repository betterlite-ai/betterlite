#![cfg(test)]

use betterlite_anthropic::stream_state::IdSource;
use betterlite_prompts::arguments::json::Json;
use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::api::messages_chat_output::content::content_blocks;
use crate::api::messages_chat_output::output::ContentEntry;
use crate::api::messages_chat_output::records::{
    ChoiceRecord, ResponseRecord, TerminalRecord, UsageRecord,
};
use crate::api::messages_context_clear::result::{
    AppliedEdit, CompactionBlock, CompactionKind, PolyfillResult, UsageIteration, UsageKind,
};

pub(super) const RESPONSE_ID: &str = "chatcmpl-b966";
const GENERATED_ID: u128 = 96;

pub(super) struct FixedIds(Uuid);

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        self.0
    }
}

pub(super) fn fixed_ids() -> FixedIds {
    FixedIds(Uuid::from_u128(GENERATED_ID))
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("valid fixture")
}

fn rendered<T: Serialize>(value: &T) -> Json {
    let text = serde_json::to_string(value).expect("serializes");
    serde_json::from_str(&text).expect("valid json")
}

pub(super) fn assert_rendered<T: Serialize>(value: &T, expected: &str) {
    assert_eq!(rendered(value), json(expected));
}

fn decode<T: DeserializeOwned>(source: &str) -> T {
    serde_json::from_str(source).expect("valid record")
}

pub(super) fn choices(source: &str) -> Vec<ChoiceRecord> {
    decode(source)
}

pub(super) fn blocks(source: &str) -> Vec<ContentEntry> {
    content_blocks(&choices(source), None).expect("content translates")
}

pub(super) fn response(source: &str) -> ResponseRecord {
    decode(source)
}

pub(super) fn usage(source: &str) -> UsageRecord {
    decode(source)
}

pub(super) fn terminal(source: &str) -> TerminalRecord {
    decode(source)
}

pub(super) fn polyfill_result() -> PolyfillResult {
    PolyfillResult {
        messages: Vec::new(),
        system: None,
        applied_edits: vec![AppliedEdit {
            edit_type: Some("clear_tool_uses_20250919".to_owned()),
            cleared_input_tokens: Some(5),
            ..AppliedEdit::default()
        }],
        compaction_block: Some(CompactionBlock {
            kind: CompactionKind::Compaction,
            content: Some("summary".to_owned()),
            signature: Some("cs".to_owned()),
        }),
        iterations_usage: Some(vec![UsageIteration {
            kind: UsageKind::Compaction,
            input_tokens: Some(10),
            output_tokens: Some(1),
        }]),
    }
}
