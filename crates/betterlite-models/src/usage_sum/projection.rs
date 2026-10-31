use std::collections::BTreeMap;

use crate::cost_usage::{CompletionDetails, CostUsage, PromptDetails};

use super::{CompletionTokensDetails, PromptTokensDetails, Usage, UsageNumber, decimal_number};

macro_rules! usage_projection {
    (fn $name:ident($arg:ident : $input:ty) -> $output:path { $($field:ident: $value:expr),+ $(,)? }) => {
        #[must_use]
        pub fn $name($arg: $input) -> $output {
            $output { $($field: $value,)+ }
        }
    };
}

impl Usage {
    usage_projection!(fn from_cost_usage(value: &CostUsage) -> Self {
        prompt_tokens: value.prompt_tokens,
        completion_tokens: value.completion_tokens,
        total_tokens: value.total_tokens,
        prompt_tokens_details: value.prompt_details.map(prompt_from_cost),
        completion_tokens_details: value.completion_details.map(completion_from_cost),
        numeric_extras: map_numeric_extras(&value.numeric_extras, |extra| UsageNumber::Decimal(*extra)),
    });

    usage_projection!(fn to_cost_usage(self: &Self) -> CostUsage {
        prompt_tokens: self.prompt_tokens,
        completion_tokens: self.completion_tokens,
        total_tokens: self.total_tokens,
        prompt_details: self.prompt_tokens_details.as_ref().map(prompt_to_cost),
        completion_details: self.completion_tokens_details.as_ref().map(completion_to_cost),
        cache_read_tokens: None,
        numeric_extras: map_numeric_extras(&self.numeric_extras, decimal_number),
    });
}

fn map_numeric_extras<T, V>(
    source: &BTreeMap<String, T>,
    mut convert: impl FnMut(&T) -> V,
) -> BTreeMap<String, V> {
    let mut result = BTreeMap::new();
    for (name, value) in source {
        result.insert(name.clone(), convert(value));
    }
    result
}

macro_rules! detail_projection {
    ($name:ident($arg:ident : $input:ty) -> $output:path { $($field:ident: $value:expr),+ $(,)? }) => {
        const fn $name($arg: $input) -> $output {
            $output { $($field: $value,)+ }
        }
    };
}

detail_projection!(prompt_from_cost(value: PromptDetails) -> PromptTokensDetails {
    text_tokens: value.text_tokens,
    audio_tokens: value.audio_tokens,
    image_tokens: value.image_tokens,
    video_tokens: value.video_tokens,
    web_search_requests: value.web_search_requests,
    google_maps_grounding_requests: None,
    tool_use_tokens: None,
    character_count: None,
    image_count: None,
    video_length_seconds: None,
    audio_length_seconds: None,
    query_count: None,
    cache_write_tokens: None,
    cache_creation_tokens: None,
    cached_tokens_details: None,
});

detail_projection!(completion_from_cost(value: CompletionDetails) -> CompletionTokensDetails {
    text_tokens: value.text_tokens,
    audio_tokens: value.audio_tokens,
    image_tokens: value.image_tokens,
    video_tokens: value.video_tokens,
    reasoning_tokens: value.reasoning_tokens,
    accepted_prediction_tokens: None,
    rejected_prediction_tokens: None,
});

detail_projection!(prompt_to_cost(value: &PromptTokensDetails) -> PromptDetails {
    text_tokens: value.text_tokens,
    audio_tokens: value.audio_tokens,
    image_tokens: value.image_tokens,
    video_tokens: value.video_tokens,
    web_search_requests: value.web_search_requests,
});

detail_projection!(completion_to_cost(value: &CompletionTokensDetails) -> CompletionDetails {
    text_tokens: value.text_tokens,
    audio_tokens: value.audio_tokens,
    image_tokens: value.image_tokens,
    video_tokens: value.video_tokens,
    reasoning_tokens: value.reasoning_tokens,
});
