use std::time::Duration;

use betterlite_prompts::arguments::json::Json;
use secrecy::SecretString;

pub mod typesafe_api_call;
pub mod typesafe_candidate_exchanges;
pub mod typesafe_config;
pub mod typesafe_failure_handling;
pub mod typesafe_instructions;
pub mod typesafe_message_parsing;
pub mod typesafe_protected_indices;
pub mod typesafe_state_building;
pub mod typesafe_text_extraction;
pub mod typesafe_tool_extraction;
pub mod typesafe_truncation;

#[path = "typesafe/tests/fixtures.rs"]
mod fixtures;

use typesafe_config::TypeSafeSettings;

pub const DEFAULT_API_BASE: &str = "https://api.typesafe.ai";
pub const DEFAULT_MODEL: &str = "jev-latest";
pub const DEFAULT_RELEVANCE_THRESHOLD: f64 = 0.2;
pub const DEFAULT_MIN_CHARS_TO_EVALUATE: usize = 200;
pub const DEFAULT_MAX_RESULT_CHARS: usize = 4000;
pub const MAX_EXCHANGES_EVALUATED: usize = 200;
pub const DEFAULT_JEV_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_RESPONSE_TEXT_LIMIT: usize = 500;
pub const DROPPED_RESULT_TEXT: &str =
    "[Tool result removed by TypeSafe compaction: judged no longer relevant to the current task]";
pub const ELISION_MARKER: &str = "\n... [middle truncated] ...\n";

// content_to_text belongs to the guardrail framework, which reaches a message part list this crate cannot import.
pub type ContentToText = fn(Option<&Json>) -> String;
pub type GetProtectedIndices = fn(&[Json]) -> Vec<usize>;
pub type GroupToolExchanges = fn(&[Json]) -> Vec<Vec<usize>>;

#[derive(Debug)]
pub struct TypeSafeGuardrail {
    pub settings: TypeSafeSettings,
    pub api_key: SecretString,
    pub client: reqwest::Client,
    // LiteLLM falls back to its 30 second Jev timeout only after __init__ passed no timeout.
    pub timeout: Option<Duration>,
    pub content_to_text: ContentToText,
    pub get_protected_indices: GetProtectedIndices,
    pub group_tool_exchanges: GroupToolExchanges,
}
