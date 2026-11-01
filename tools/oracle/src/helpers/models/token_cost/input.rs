use std::collections::BTreeMap;

use betterlite_models::cost_usage::modality_projection::{
    CacheCreationTokenDetails, CachedTokenCounters,
};
use betterlite_models::cost_usage::{CompletionDetails, CostUsage, PromptDetails};
use betterlite_models::off_peak::schema::OffPeakBlock;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::tiered_pricing::Tier;
use betterlite_models::token_cost::prompt_overlap::PromptExtras;
use betterlite_models::token_rates::ModelInfo;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;

// Generates the From impl for a plain field-by-field XInput -> domain-type conversion.
macro_rules! copy_fields {
    ($from:ty => $to:ty, |$input:ident| { $($field:ident: $value:expr),+ $(,)? }) => {
        impl From<$from> for $to {
            fn from($input: $from) -> Self {
                Self { $($field: $value),+ }
            }
        }
    };
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
struct PromptDetailsInput {
    #[serde(default)]
    text_tokens: Option<i64>,
    #[serde(default)]
    audio_tokens: Option<i64>,
    #[serde(default)]
    image_tokens: Option<i64>,
    #[serde(default)]
    video_tokens: Option<i64>,
    #[serde(default)]
    web_search_requests: Option<i64>,
}

copy_fields!(PromptDetailsInput => PromptDetails, |fields| {
    text_tokens: fields.text_tokens,
    audio_tokens: fields.audio_tokens,
    image_tokens: fields.image_tokens,
    video_tokens: fields.video_tokens,
    web_search_requests: fields.web_search_requests,
});

#[derive(Debug, Clone, Copy, Default, Deserialize)]
struct CompletionDetailsInput {
    #[serde(default)]
    text_tokens: Option<i64>,
    #[serde(default)]
    audio_tokens: Option<i64>,
    #[serde(default)]
    image_tokens: Option<i64>,
    #[serde(default)]
    video_tokens: Option<i64>,
    #[serde(default, rename = "reasoning_tokens")]
    reasoning: Option<i64>,
}

copy_fields!(CompletionDetailsInput => CompletionDetails, |fields| {
    text_tokens: fields.text_tokens,
    audio_tokens: fields.audio_tokens,
    image_tokens: fields.image_tokens,
    video_tokens: fields.video_tokens,
    reasoning_tokens: fields.reasoning,
});

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct UsageInput {
    #[serde(default)]
    prompt_tokens: i64,
    #[serde(default)]
    completion_tokens: i64,
    #[serde(default)]
    total_tokens: i64,
    #[serde(default)]
    prompt_details: Option<PromptDetailsInput>,
    #[serde(default)]
    completion_details: Option<CompletionDetailsInput>,
    #[serde(default)]
    cache_read_tokens: Option<i64>,
}

copy_fields!(UsageInput => CostUsage, |fields| {
    prompt_tokens: fields.prompt_tokens,
    completion_tokens: fields.completion_tokens,
    total_tokens: fields.total_tokens,
    prompt_details: fields.prompt_details.map(PromptDetails::from),
    completion_details: fields.completion_details.map(CompletionDetails::from),
    cache_read_tokens: fields.cache_read_tokens,
    numeric_extras: BTreeMap::new(),
});

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct PromptExtrasInput {
    #[serde(default)]
    cache_write_tokens: Option<i64>,
    #[serde(default)]
    cache_creation_tokens: Option<i64>,
    #[serde(default)]
    cache_creation_token_details: Option<CacheCreationTokenDetails>,
    #[serde(default)]
    cached_tokens_details: Option<CachedTokenCounters>,
    #[serde(default)]
    character_count: Option<i64>,
    #[serde(default)]
    image_count: Option<i64>,
    #[serde(default)]
    video_length_seconds: Option<Decimal>,
    #[serde(default)]
    audio_length_seconds: Option<Decimal>,
    #[serde(default)]
    query_count: Option<i64>,
}

copy_fields!(PromptExtrasInput => PromptExtras, |fields| {
    cache_write_tokens: fields.cache_write_tokens,
    cache_creation_tokens: fields.cache_creation_tokens,
    cache_creation_token_details: fields.cache_creation_token_details,
    cached_tokens_details: fields.cached_tokens_details,
    character_count: fields.character_count,
    image_count: fields.image_count,
    video_length_seconds: fields.video_length_seconds,
    audio_length_seconds: fields.audio_length_seconds,
    query_count: fields.query_count,
});

const fn default_moment() -> DateTime<Utc> {
    DateTime::UNIX_EPOCH
}

#[derive(Debug, Deserialize)]
pub(crate) struct GenericInput {
    #[serde(default)]
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) usage: UsageInput,
    #[serde(default)]
    pub(crate) prompt_extras: Option<PromptExtrasInput>,
    #[serde(default)]
    pub(crate) supplied_model_info: Option<ModelInfo>,
    #[serde(default)]
    pub(crate) service_tier: Option<String>,
    #[serde(default)]
    pub(crate) data_residency: Option<String>,
    #[serde(default)]
    pub(crate) vertex_location: Option<String>,
    #[serde(default = "default_moment")]
    pub(crate) moment: DateTime<Utc>,
    #[serde(default)]
    pub(crate) tiered_pricing: Vec<Tier>,
    #[serde(default)]
    pub(crate) off_peak_pricing: Option<OffPeakBlock>,
    #[serde(default)]
    pub(crate) off_peak_cache_read_present: bool,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation")]
pub(crate) enum Input {
    #[serde(rename = "generic")]
    Generic(Box<GenericInput>),
    #[serde(rename = "provider_geo")]
    ProviderGeo {
        #[serde(default)]
        provider_specific_entry: Option<PriceRecord>,
        #[serde(default)]
        inference_geo: Option<String>,
    },
    #[serde(rename = "fireworks_cache_default")]
    FireworksCacheDefault(Box<FireworksCacheDefaultInput>),
}

#[derive(Debug, Deserialize)]
pub(crate) struct FireworksCacheDefaultInput {
    pub(crate) model_info: ModelInfo,
    #[serde(default)]
    pub(crate) custom_llm_provider: Option<String>,
    #[serde(default)]
    pub(crate) off_peak_pricing: Option<OffPeakBlock>,
    #[serde(default)]
    pub(crate) off_peak_cache_read_present: bool,
}
