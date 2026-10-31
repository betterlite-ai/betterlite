use std::collections::BTreeMap;

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

impl From<&CostUsage> for Usage {
    fn from(value: &CostUsage) -> Self {
        Self {
            prompt_tokens: value.prompt_tokens,
            completion_tokens: value.completion_tokens,
            total_tokens: value.total_tokens,
            prompt_tokens_details: prompt_details_from_cost(
                value.prompt_details,
                value.cache_read_tokens,
            ),
            completion_tokens_details: value.completion_details.map(|details| {
                CompletionTokensDetails {
                    text_tokens: details.text_tokens,
                    audio_tokens: details.audio_tokens,
                    image_tokens: details.image_tokens,
                    video_tokens: details.video_tokens,
                    reasoning_tokens: details.reasoning_tokens,
                    accepted_prediction_tokens: None,
                    rejected_prediction_tokens: None,
                }
            }),
            numeric_extras: map_numeric_extras(&value.numeric_extras, |extra| {
                UsageNumber::Decimal(*extra)
            }),
        }
    }
}

fn prompt_details_to_cost(
    source: Option<&PromptTokensDetails>,
) -> (Option<PromptDetails>, Option<i64>) {
    let prompt_details = source.map(|details| PromptDetails {
        text_tokens: details.text_tokens,
        audio_tokens: details.audio_tokens,
        image_tokens: details.image_tokens,
        video_tokens: details.video_tokens,
        web_search_requests: details.web_search_requests,
    });
    let cache_read_tokens = source.and_then(|details| details.cached_tokens);
    (prompt_details, cache_read_tokens)
}

impl From<&Usage> for CostUsage {
    fn from(value: &Usage) -> Self {
        let (prompt_details, cache_read_tokens) =
            prompt_details_to_cost(value.prompt_tokens_details.as_ref());
        Self {
            prompt_tokens: value.prompt_tokens,
            completion_tokens: value.completion_tokens,
            total_tokens: value.total_tokens,
            prompt_details,
            completion_details: value.completion_tokens_details.as_ref().map(|details| {
                CompletionDetails {
                    text_tokens: details.text_tokens,
                    audio_tokens: details.audio_tokens,
                    image_tokens: details.image_tokens,
                    video_tokens: details.video_tokens,
                    reasoning_tokens: details.reasoning_tokens,
                }
            }),
            cache_read_tokens,
            numeric_extras: map_numeric_extras(&value.numeric_extras, decimal_number),
        }
    }
}

fn map_numeric_extras<T, V, F>(source: &BTreeMap<String, T>, mut convert: F) -> BTreeMap<String, V>
where
    F: FnMut(&T) -> V,
{
    let mut result = BTreeMap::new();
    for (name, value) in source {
        result.insert(name.clone(), convert(value));
    }
    result
}
