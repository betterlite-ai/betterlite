use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

pub mod blocked_usage;
pub mod fingerprint;
pub mod merge_messages;
pub mod merge_tools;
pub mod scoped_indices;
pub mod stream_chunk_events;
pub mod tool_names;
pub mod unappliable_rewrite;

pub type Object = BTreeMap<String, GuardrailValue>;

#[derive(Debug, Clone, PartialEq)]
pub enum StreamItem {
    Json(GuardrailValue),
    Bytes(Vec<u8>),
}
