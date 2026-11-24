use std::collections::BTreeMap;

use betterlite_models::usage_sum::{
    CachedTokensDetails, CompletionTokensDetails, PromptTokensDetails, Usage, UsageNumber,
};
use serde::{Deserialize, Serialize};

use crate::cost::convert::{Amount, optional};

// The usage a Python Usage object carries: its three counters, the two detail objects and any numeric extra.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UsageWire {
    pub(crate) prompt_tokens: i64,
    pub(crate) completion_tokens: i64,
    pub(crate) total_tokens: i64,
    pub(crate) prompt_tokens_details: Option<PromptDetailsWire>,
    pub(crate) completion_tokens_details: Option<CompletionDetailsWire>,
    pub(crate) numeric_extras: BTreeMap<String, UsageNumberView>,
}

impl UsageWire {
    pub(crate) fn usage(&self) -> Usage {
        Usage {
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            total_tokens: self.total_tokens,
            prompt_tokens_details: self
                .prompt_tokens_details
                .as_ref()
                .map(PromptTokensDetails::from),
            completion_tokens_details: self
                .completion_tokens_details
                .as_ref()
                .map(CompletionTokensDetails::from),
            numeric_extras: self
                .numeric_extras
                .iter()
                .map(|(name, number)| (name.clone(), number.number()))
                .collect(),
        }
    }
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
    pub(crate) text_tokens: Option<i64>,
    pub(crate) audio_tokens: Option<i64>,
    pub(crate) image_tokens: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct CompletionDetailsWire {
    pub(crate) text_tokens: Option<i64>,
    pub(crate) audio_tokens: Option<i64>,
    pub(crate) image_tokens: Option<i64>,
    pub(crate) video_tokens: Option<i64>,
    pub(crate) reasoning_tokens: Option<i64>,
    pub(crate) accepted_prediction_tokens: Option<i64>,
    pub(crate) rejected_prediction_tokens: Option<i64>,
}

impl From<&PromptDetailsWire> for PromptTokensDetails {
    fn from(wire: &PromptDetailsWire) -> Self {
        Self {
            text_tokens: wire.text_tokens,
            audio_tokens: wire.audio_tokens,
            image_tokens: wire.image_tokens,
            video_tokens: wire.video_tokens,
            web_search_requests: wire.web_search_requests,
            google_maps_grounding_requests: wire.google_maps_grounding_requests,
            tool_use_tokens: wire.tool_use_tokens,
            character_count: wire.character_count,
            image_count: wire.image_count,
            video_length_seconds: optional(wire.video_length_seconds),
            audio_length_seconds: optional(wire.audio_length_seconds),
            query_count: wire.query_count,
            cache_write_tokens: wire.cache_write_tokens,
            cache_creation_tokens: wire.cache_creation_tokens,
            cached_tokens: wire.cached_tokens,
            cached_tokens_details: wire
                .cached_tokens_details
                .as_ref()
                .map(CachedTokensDetails::from),
        }
    }
}

impl From<&PromptTokensDetails> for PromptDetailsWire {
    fn from(details: &PromptTokensDetails) -> Self {
        Self {
            text_tokens: details.text_tokens,
            audio_tokens: details.audio_tokens,
            image_tokens: details.image_tokens,
            video_tokens: details.video_tokens,
            web_search_requests: details.web_search_requests,
            google_maps_grounding_requests: details.google_maps_grounding_requests,
            tool_use_tokens: details.tool_use_tokens,
            character_count: details.character_count,
            image_count: details.image_count,
            video_length_seconds: details.video_length_seconds.map(Amount),
            audio_length_seconds: details.audio_length_seconds.map(Amount),
            query_count: details.query_count,
            cache_write_tokens: details.cache_write_tokens,
            cache_creation_tokens: details.cache_creation_tokens,
            cached_tokens: details.cached_tokens,
            cached_tokens_details: details
                .cached_tokens_details
                .as_ref()
                .map(CachedDetailsWire::from),
        }
    }
}

impl From<&CachedDetailsWire> for CachedTokensDetails {
    fn from(wire: &CachedDetailsWire) -> Self {
        Self {
            text_tokens: wire.text_tokens,
            audio_tokens: wire.audio_tokens,
            image_tokens: wire.image_tokens,
        }
    }
}

impl From<&CachedTokensDetails> for CachedDetailsWire {
    fn from(details: &CachedTokensDetails) -> Self {
        Self {
            text_tokens: details.text_tokens,
            audio_tokens: details.audio_tokens,
            image_tokens: details.image_tokens,
        }
    }
}

impl From<&CompletionDetailsWire> for CompletionTokensDetails {
    fn from(wire: &CompletionDetailsWire) -> Self {
        Self {
            text_tokens: wire.text_tokens,
            audio_tokens: wire.audio_tokens,
            image_tokens: wire.image_tokens,
            video_tokens: wire.video_tokens,
            reasoning_tokens: wire.reasoning_tokens,
            accepted_prediction_tokens: wire.accepted_prediction_tokens,
            rejected_prediction_tokens: wire.rejected_prediction_tokens,
        }
    }
}

impl From<&CompletionTokensDetails> for CompletionDetailsWire {
    fn from(details: &CompletionTokensDetails) -> Self {
        Self {
            text_tokens: details.text_tokens,
            audio_tokens: details.audio_tokens,
            image_tokens: details.image_tokens,
            video_tokens: details.video_tokens,
            reasoning_tokens: details.reasoning_tokens,
            accepted_prediction_tokens: details.accepted_prediction_tokens,
            rejected_prediction_tokens: details.rejected_prediction_tokens,
        }
    }
}

// The combined usage as LiteLLM's Usage model reads it back, aliases and numeric extras included.
#[derive(Debug, Serialize)]
pub(crate) struct UsageView {
    pub(crate) prompt_tokens: i64,
    pub(crate) completion_tokens: i64,
    pub(crate) total_tokens: i64,
    pub(crate) prompt_tokens_details: Option<PromptDetailsWire>,
    pub(crate) completion_tokens_details: Option<CompletionDetailsWire>,
    // LiteLLM's combined Usage carries an extra as a plain attribute, so the answer spells them flat.
    #[serde(flatten)]
    pub(crate) numeric_extras: BTreeMap<String, UsageNumberView>,
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
    fn number(&self) -> UsageNumber {
        match self {
            Self::Integer(value) => UsageNumber::Integer(*value),
            Self::Amount(value) => UsageNumber::Decimal(value.0),
            Self::Boolean(flag) => UsageNumber::Boolean(*flag),
        }
    }
}

impl From<&Usage> for UsageView {
    fn from(usage: &Usage) -> Self {
        Self {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
            prompt_tokens_details: usage
                .prompt_tokens_details
                .as_ref()
                .map(PromptDetailsWire::from),
            completion_tokens_details: usage
                .completion_tokens_details
                .as_ref()
                .map(CompletionDetailsWire::from),
            numeric_extras: usage
                .numeric_extras
                .iter()
                .map(|(name, number)| (name.clone(), UsageNumberView::from(number)))
                .collect(),
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
