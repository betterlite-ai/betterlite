#![expect(
    clippy::disallowed_types,
    reason = "compression normalizes heterogeneous provider JSON before higher layers type it"
)]
#![expect(
    clippy::arithmetic_side_effects,
    clippy::integer_division,
    clippy::integer_division_remainder_used,
    clippy::must_use_candidate,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::excessive_nesting,
    clippy::else_if_without_else,
    clippy::uninlined_format_args,
    clippy::default_numeric_fallback,
    clippy::return_and_then,
    clippy::wildcard_enum_match_arm,
    reason = "normalization mirrors LiteLLM's bounded index, line and heuristic operations"
)]
#![cfg_attr(
    test,
    expect(clippy::disallowed_methods, reason = "serde_json test fixtures are concise and type-directed")
)]

pub mod anthropic_tool_exchange;
pub mod content_detection;
pub mod extract_key;
pub mod message_normalization;
pub mod message_stubbing;
pub mod protected_indices;
pub mod truncate_message;
