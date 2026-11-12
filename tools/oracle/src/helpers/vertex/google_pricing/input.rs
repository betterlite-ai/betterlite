use std::collections::BTreeMap;

use betterlite_models::usage_sum::{PromptTokensDetails, Usage};
use betterlite_vertex::google_pricing::usage::{GoogleUsage, ServerToolUse};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::value::RawValue;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    CostRouter,
    CostPerCharacter,
    CostPerToken,
    WebSearchCost,
    MapsGroundingCost,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SearchEntry {
    Gemini,
    VertexGemini,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) operation: Operation,
    #[serde(default)]
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) custom_llm_provider: String,
    #[serde(default)]
    pub(crate) call_type: String,
    pub(crate) record: Option<Box<RawValue>>,
    #[serde(default)]
    pub(crate) usage: UsageInput,
    pub(crate) prompt_characters: Option<Decimal>,
    pub(crate) completion_characters: Option<Decimal>,
    pub(crate) service_tier: Option<String>,
    pub(crate) vertex_location: Option<String>,
    // both LiteLLM entry points run the same shared search calculation, so only Python needs to pick one
    #[serde(rename = "entry")]
    pub(crate) _entry: Option<SearchEntry>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct UsageInput {
    prompt_tokens: i64,
    completion_tokens: i64,
    prompt_tokens_details: Option<DetailsInput>,
    server_tool_use: Option<ServerToolUseInput>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct DetailsInput {
    web_search_requests: Option<i64>,
    google_maps_grounding_requests: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ServerToolUseInput {
    web_search_requests: Option<i64>,
}

impl UsageInput {
    pub(crate) fn google_usage(&self) -> GoogleUsage {
        let details = self
            .prompt_tokens_details
            .as_ref()
            .map(|source| PromptTokensDetails {
                text_tokens: None,
                audio_tokens: None,
                image_tokens: None,
                video_tokens: None,
                web_search_requests: source.web_search_requests,
                google_maps_grounding_requests: source.google_maps_grounding_requests,
                tool_use_tokens: None,
                character_count: None,
                image_count: None,
                video_length_seconds: None,
                audio_length_seconds: None,
                query_count: None,
                cache_write_tokens: None,
                cache_creation_tokens: None,
                cached_tokens: None,
                cached_tokens_details: None,
            });
        GoogleUsage {
            tokens: Usage {
                prompt_tokens: self.prompt_tokens,
                completion_tokens: self.completion_tokens,
                total_tokens: self.prompt_tokens.saturating_add(self.completion_tokens),
                prompt_tokens_details: details,
                completion_tokens_details: None,
                numeric_extras: BTreeMap::new(),
            },
            server_tool_use: ServerToolUse {
                web_search_requests: self
                    .server_tool_use
                    .as_ref()
                    .and_then(|server| server.web_search_requests),
            },
        }
    }
}
