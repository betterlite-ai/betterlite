use std::collections::BTreeMap;

use crate::routes::CallType;
use record_json::RecordJson;

pub mod call_type;
pub mod custom_id;
pub mod error;
pub mod fingerprint;
pub mod hook;
pub mod lines;
pub mod record_json;
pub mod records;
pub mod scan_record;

mod tests;

pub const EMPTY_MAPPING: BTreeMap<String, RecordJson> = BTreeMap::new();

pub const SCAN_WINDOW: usize = 32;

// Past this the rewrite rolls to disk, so a large upload's rewrites stay off the heap
pub const REWRITE_SPOOL_BYTES: usize = 0x0010_0000;

// char::is_ascii_control is exactly LiteLLM's [\x00-\x1f\x7f], so describe needs no pattern
pub const CUSTOM_ID_LOG_LIMIT: usize = 128;
pub const SUMMARY_LIMIT: usize = 50;

pub const SCAN_METADATA_KEY: &str = "litellm_metadata";
pub const SCAN_METADATA_BAGS: [&str; 2] = [SCAN_METADATA_KEY, "metadata"];

pub const ROUTE_APPLIED_KEY: &str = "sensitive_data_routing_applied";

// A record's own guardrails would replace the chain its key and team selected, so these are dropped before the hook and restored after
pub const INJECTED_KEYS: [&str; 3] = ["guardrails", "metadata", SCAN_METADATA_KEY];

pub const SCAN_METADATA_KEYS: [&str; 7] = [
    "guardrails",
    "_guardrail_pipelines",
    "_pipeline_managed_guardrails",
    "user_api_key_metadata",
    "user_api_key_team_metadata",
    "tags",
    "headers",
];

pub const SCANNABLE_CALL_TYPES: [CallType; 5] = [
    CallType::Acompletion,
    CallType::AtextCompletion,
    CallType::Aembedding,
    CallType::Aresponses,
    CallType::AnthropicMessages,
];

// The record classifier of LiteLLM's Bedrock batch files, so a record without a url keeps working
pub const BODY_SHAPE_CALL_TYPES: [(&str, CallType); 3] = [
    ("messages", CallType::Acompletion),
    ("prompt", CallType::AtextCompletion),
    ("input", CallType::Aembedding),
];
