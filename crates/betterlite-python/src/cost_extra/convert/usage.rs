use std::collections::BTreeMap;

use betterlite_models::usage_sum::{
    CachedTokensDetails, CompletionTokensDetails, PromptTokensDetails, Usage, UsageNumber,
};
use serde::{Deserialize, Serialize};

use crate::cost::convert::{Amount, optional};

// The usage a Python Usage object carries: its counters, both detail objects and any extra, which travels flat.
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct UsageWire {
    pub(crate) prompt_tokens: i64,
    pub(crate) completion_tokens: i64,
    pub(crate) total_tokens: i64,
    pub(crate) prompt_tokens_details: Option<PromptDetailsWire>,
    pub(crate) completion_tokens_details: Option<CompletionDetailsWire>,
    #[serde(flatten)]
    pub(crate) numeric_extras: BTreeMap<String, UsageNumberView>,
}

// Every counter LiteLLM's prompt wrapper declares; the wrapper allows extras, so unknown keys are ignored.
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct PromptDetailsWire {
    pub(crate) text_tokens: Option<i64>,
    pub(crate) audio_tokens: Option<i64>,
    pub(crate) image_tokens: Option<i64>,
    pub(crate) video_tokens: Option<i64>,
    pub(crate) web_search_requests: Option<i64>,
    pub(crate) google_maps_grounding_requests: Option<i64>,
    pub(crate) tool_use_tokens: Option<i64>,
    pub(crate) character_count: Option<i64>,
    pub(crate) image_count: Option<i64>,
    pub(crate) video_length_seconds: Option<Amount>,
    pub(crate) audio_length_seconds: Option<Amount>,
    pub(crate) query_count: Option<i64>,
    pub(crate) cache_write_tokens: Option<i64>,
    pub(crate) cache_creation_tokens: Option<i64>,
    pub(crate) cached_tokens: Option<i64>,
    pub(crate) cached_tokens_details: Option<CachedDetailsWire>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct CachedDetailsWire {
    #[serde(rename = "text_tokens")]
    pub(crate) text: Option<i64>,
    pub(crate) audio_tokens: Option<i64>,
    pub(crate) image_tokens: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct CompletionDetailsWire {
    pub(crate) text_tokens: Option<i64>,
    pub(crate) audio_tokens: Option<i64>,
    pub(crate) image_tokens: Option<i64>,
    pub(crate) video_tokens: Option<i64>,
    #[serde(rename = "reasoning_tokens")]
    pub(crate) reasoning: Option<i64>,
    pub(crate) accepted_prediction_tokens: Option<i64>,
    pub(crate) rejected_prediction_tokens: Option<i64>,
}

// One usage extra both ways: an int, an exact decimal or a flag, as LiteLLM's numeric attributes allow.
#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub(crate) enum UsageNumberView {
    Integer(i64),
    Amount(Amount),
    Boolean(bool),
}

impl UsageNumberView {
    const fn number(&self) -> UsageNumber {
        match self {
            Self::Integer(value) => UsageNumber::Integer(*value),
            Self::Amount(value) => UsageNumber::Decimal(value.0),
            Self::Boolean(flag) => UsageNumber::Boolean(*flag),
        }
    }
}

impl From<&UsageNumber> for UsageNumberView {
    fn from(number: &UsageNumber) -> Self {
        match number {
            UsageNumber::Integer(value) => Self::Integer(*value),
            UsageNumber::Decimal(value) => Self::Amount(Amount(*value)),
            UsageNumber::Boolean(flag) => Self::Boolean(*flag),
        }
    }
}

// The wire details and the model details name the same counters; a pair renames the wire field.
macro_rules! mirrored_details {
    ($wire:ident <=> $details:ident { $($name:ident => $field:ident),+ $(,)? }) => {
        impl From<&$wire> for $details {
            fn from(wire: &$wire) -> Self { Self { $($field: wire.$name),+ } }
        }
        impl From<&$details> for $wire {
            fn from(details: &$details) -> Self { Self { $($name: details.$field),+ } }
        }
    };
}

// Prompt details also carry two exact durations and the nested cache counters.
macro_rules! mirrored_prompt_details {
    ($($field:ident),+ ; seconds $($seconds:ident),+ ; nested $nested:ident $(,)?) => {
        impl From<&PromptDetailsWire> for PromptTokensDetails {
            fn from(wire: &PromptDetailsWire) -> Self {
                Self { $($field: wire.$field,)+ $($seconds: optional(wire.$seconds),)+
                    $nested: wire.$nested.as_ref().map(CachedTokensDetails::from) }
            }
        }
        impl From<&PromptTokensDetails> for PromptDetailsWire {
            fn from(details: &PromptTokensDetails) -> Self {
                Self { $($field: details.$field,)+ $($seconds: details.$seconds.map(Amount),)+
                    $nested: details.$nested.as_ref().map(CachedDetailsWire::from) }
            }
        }
    };
}

// The wire and the model share these field names; only how the extras carry differs.
macro_rules! usage_bridge {
    ($source:ident => $target:ident, prompt $prompt:path, completion $completion:path, extra $extra:path $(,)?) => {
        impl From<&$source> for $target {
            fn from(source: &$source) -> Self {
                Self {
                    prompt_tokens: source.prompt_tokens,
                    completion_tokens: source.completion_tokens,
                    total_tokens: source.total_tokens,
                    prompt_tokens_details: source.prompt_tokens_details.as_ref().map($prompt),
                    completion_tokens_details: source
                        .completion_tokens_details
                        .as_ref()
                        .map($completion),
                    numeric_extras: source
                        .numeric_extras
                        .iter()
                        .map(|(name, number)| (name.clone(), $extra(number)))
                        .collect(),
                }
            }
        }
    };
}

mirrored_details!(CachedDetailsWire <=> CachedTokensDetails {
    text => text_tokens, audio_tokens => audio_tokens, image_tokens => image_tokens,
});

mirrored_details!(CompletionDetailsWire <=> CompletionTokensDetails {
    text_tokens => text_tokens, audio_tokens => audio_tokens, image_tokens => image_tokens,
    video_tokens => video_tokens, reasoning => reasoning_tokens,
    accepted_prediction_tokens => accepted_prediction_tokens,
    rejected_prediction_tokens => rejected_prediction_tokens,
});

mirrored_prompt_details!(
    text_tokens, audio_tokens, image_tokens, video_tokens, web_search_requests,
    google_maps_grounding_requests, tool_use_tokens, character_count, image_count,
    query_count, cache_write_tokens, cache_creation_tokens, cached_tokens;
    seconds video_length_seconds, audio_length_seconds;
    nested cached_tokens_details
);

usage_bridge!(
    UsageWire => Usage,
    prompt PromptTokensDetails::from,
    completion CompletionTokensDetails::from,
    extra UsageNumberView::number
);

// The combined usage as LiteLLM's Usage model reads it back: the same wire, extras included.
usage_bridge!(
    Usage => UsageWire,
    prompt PromptDetailsWire::from,
    completion CompletionDetailsWire::from,
    extra UsageNumberView::from
);
