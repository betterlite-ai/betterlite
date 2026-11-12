use std::collections::BTreeMap;

use betterlite_models::usage_sum::{PromptTokensDetails, Usage};
use betterlite_vertex::google_pricing::usage::{GoogleUsage, ServerToolUse};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::value::RawValue;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Operation {
    CostRouter,
    CostPerCharacter,
    CostPerToken,
    WebSearchCost,
    MapsGroundingCost,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SearchEntry {
    Gemini,
    VertexGemini,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    pub(super) operation: Operation,
    #[serde(default)]
    pub(super) model: String,
    #[serde(default)]
    pub(super) custom_llm_provider: String,
    #[serde(default)]
    pub(super) call_type: String,
    pub(super) record: Option<Box<RawValue>>,
    #[serde(default)]
    pub(super) usage: UsageInput,
    pub(super) prompt_characters: Option<Decimal>,
    pub(super) completion_characters: Option<Decimal>,
    pub(super) service_tier: Option<String>,
    pub(super) vertex_location: Option<String>,
    // both LiteLLM entry points run the same shared search calculation, so only Python needs to pick one
    #[serde(rename = "entry")]
    pub(super) _entry: Option<SearchEntry>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct UsageInput {
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
    pub(super) fn google_usage(&self) -> GoogleUsage {
        let mut details: Option<PromptTokensDetails> = None;
        if let Some(source) = self.prompt_tokens_details.as_ref() {
            details = Some(PromptTokensDetails {
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
        }
        GoogleUsage {
            tokens: Usage {
                prompt_tokens: self.prompt_tokens,
                completion_tokens: self.completion_tokens,
                total_tokens: self.prompt_tokens + self.completion_tokens,
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
