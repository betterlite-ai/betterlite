use std::collections::BTreeMap;

use rust_decimal::Decimal;

use crate::cost_usage::{CompletionDetails, CostUsage, PromptDetails};

use super::{CompletionTokensDetails, PromptTokensDetails, Usage, UsageNumber, decimal_number};

fn prompt_details_from_cost(
    details: Option<PromptDetails>,
    cache_read_tokens: Option<i64>,
) -> Option<PromptTokensDetails> {
    match (details, cache_read_tokens) {
        (None, None) => None,
        (maybe_details, read_tokens) => {
            let defaulted = maybe_details.unwrap_or_default();
            Some(PromptTokensDetails {
                text_tokens: defaulted.text_tokens,
                audio_tokens: defaulted.audio_tokens,
                image_tokens: defaulted.image_tokens,
                video_tokens: defaulted.video_tokens,
                web_search_requests: defaulted.web_search_requests,
                google_maps_grounding_requests: None,
                tool_use_tokens: None,
                character_count: None,
                image_count: None,
                video_length_seconds: None,
                audio_length_seconds: None,
                query_count: None,
                cache_write_tokens: None,
                cache_creation_tokens: None,
                cached_tokens: read_tokens,
                cached_tokens_details: None,
            })
        }
    }
}

impl From<CompletionDetails> for CompletionTokensDetails {
    fn from(details: CompletionDetails) -> Self {
        let CompletionDetails {
            text_tokens,
            audio_tokens,
            image_tokens,
            video_tokens,
            reasoning_tokens,
        } = details;
        Self {
            text_tokens,
            audio_tokens,
            image_tokens,
            video_tokens,
            reasoning_tokens,
            accepted_prediction_tokens: None,
            rejected_prediction_tokens: None,
        }
    }
}

impl From<&CostUsage> for Usage {
    fn from(value: &CostUsage) -> Self {
        let mut usage = Self {
            prompt_tokens: value.prompt_tokens,
            completion_tokens: value.completion_tokens,
            total_tokens: value.total_tokens,
            prompt_tokens_details: prompt_details_from_cost(
                value.prompt_details,
                value.cache_read_tokens,
            ),
            completion_tokens_details: value.completion_details.map(CompletionTokensDetails::from),
            numeric_extras: BTreeMap::new(),
        };
        for (name, extra) in &value.numeric_extras {
            usage
                .numeric_extras
                .insert(name.clone(), UsageNumber::Decimal(*extra));
        }
        usage
    }
}

impl From<&CompletionTokensDetails> for CompletionDetails {
    fn from(details: &CompletionTokensDetails) -> Self {
        Self {
            text_tokens: details.text_tokens,
            audio_tokens: details.audio_tokens,
            image_tokens: details.image_tokens,
            video_tokens: details.video_tokens,
            reasoning_tokens: details.reasoning_tokens,
        }
    }
}

impl From<&Usage> for CostUsage {
    fn from(value: &Usage) -> Self {
        let mut cost = Self {
            prompt_tokens: value.prompt_tokens,
            completion_tokens: value.completion_tokens,
            total_tokens: value.total_tokens,
            prompt_details: None,
            completion_details: value
                .completion_tokens_details
                .as_ref()
                .map(CompletionDetails::from),
            cache_read_tokens: None,
            numeric_extras: decimal_numeric_extras(&value.numeric_extras),
        };
        if let Some(details) = value.prompt_tokens_details.as_ref() {
            cost.prompt_details = Some(PromptDetails {
                text_tokens: details.text_tokens,
                audio_tokens: details.audio_tokens,
                image_tokens: details.image_tokens,
                video_tokens: details.video_tokens,
                web_search_requests: details.web_search_requests,
            });
            cost.cache_read_tokens = details.cached_tokens;
        }
        cost
    }
}

fn decimal_numeric_extras(source: &BTreeMap<String, UsageNumber>) -> BTreeMap<String, Decimal> {
    let mut result = BTreeMap::new();
    for (name, extra) in source {
        result.insert(name.clone(), decimal_number(extra));
    }
    result
}
