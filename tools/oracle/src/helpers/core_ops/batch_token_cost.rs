use std::collections::{BTreeMap, BTreeSet};

use betterlite_core::cost::batch::error::BatchCostError;
use betterlite_core::cost::batch::prompt::{PromptCacheRates, batch_prompt_cost};
use betterlite_core::cost::batch::{BatchCostRequest, batch_cost_per_token};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::cost_usage::modality_projection::{
    CacheCreationTokenDetails, CachedTokenCounters,
};
use betterlite_models::cost_usage::{CostUsage, PromptDetails};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_models::token_cost::prompt_overlap::PromptExtras;
use betterlite_models::token_rates::ModelInfo;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PromptDetailsInput {
    #[serde(default, rename = "text_tokens")]
    text: Option<i64>,
    #[serde(default, rename = "audio_tokens")]
    audio: Option<i64>,
    #[serde(default, rename = "image_tokens")]
    image: Option<i64>,
    #[serde(default, rename = "video_tokens")]
    video: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UsageInput {
    #[serde(default)]
    prompt_tokens: i64,
    #[serde(default)]
    completion_tokens: i64,
    #[serde(default)]
    prompt_details: Option<PromptDetailsInput>,
    #[serde(default)]
    cache_read_tokens: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PromptExtrasInput {
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

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Input {
    Calculator(Box<CalculatorInput>),
    Prompt(Box<PromptInput>),
}

// An internally tagged variant keeps the tag in its content map, so the variant structs deny nothing.
#[derive(Deserialize)]
pub(crate) struct CalculatorInput {
    model: String,
    #[serde(default)]
    custom_llm_provider: Option<String>,
    usage: UsageInput,
    #[serde(default)]
    prompt_extras: Option<PromptExtrasInput>,
    #[serde(default)]
    model_info: Option<ModelInfo>,
    #[serde(default)]
    data_residency: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct PromptInput {
    usage: UsageInput,
    #[serde(default)]
    prompt_extras: Option<PromptExtrasInput>,
    model_info: ModelInfo,
    input_rate: Decimal,
    #[serde(default)]
    cache_read_rate: Option<Decimal>,
    #[serde(default)]
    cache_creation_rate: Option<Decimal>,
}

macro_rules! copy_fields {
    ($from:ty => $to:ty, |$input:ident| { $($field:ident: $value:expr),+ $(,)? }) => {
        impl From<$from> for $to {
            fn from($input: $from) -> Self {
                Self { $($field: $value),+ }
            }
        }
    };
}

copy_fields!(&PromptDetailsInput => PromptDetails, |input| {
    text_tokens: input.text,
    audio_tokens: input.audio,
    image_tokens: input.image,
    video_tokens: input.video,
    web_search_requests: None,
});

copy_fields!(&UsageInput => CostUsage, |input| {
    prompt_tokens: input.prompt_tokens,
    completion_tokens: input.completion_tokens,
    total_tokens: 0,
    prompt_details: input.prompt_details.as_ref().map(PromptDetails::from),
    completion_details: None,
    cache_read_tokens: input.cache_read_tokens,
    numeric_extras: BTreeMap::new(),
});

copy_fields!(&PromptExtrasInput => PromptExtras, |input| {
    cache_write_tokens: input.cache_write_tokens,
    cache_creation_tokens: input.cache_creation_tokens,
    cache_creation_token_details: input.cache_creation_token_details,
    cached_tokens_details: input.cached_tokens_details,
    character_count: input.character_count,
    image_count: input.image_count,
    video_length_seconds: input.video_length_seconds,
    audio_length_seconds: input.audio_length_seconds,
    query_count: input.query_count,
});

// Every batch case supplies its own prices, so the public table never answers here.
struct NoLookup;

impl PriceLookup for NoLookup {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        None
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }

    fn effective_priced_model(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        None
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Calculator(fields) => run_calculator(fields),
            Input::Prompt(fields) => run_prompt(fields),
        }
        .map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "BatchCostError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}

fn run_calculator(input: &CalculatorInput) -> Result<serde_json::Value, BatchCostError> {
    let usage = CostUsage::from(&input.usage);
    let extras = input
        .prompt_extras
        .as_ref()
        .map_or_else(PromptExtras::default, PromptExtras::from);
    let catalog = ModelCatalog::default();
    let names = BTreeSet::new();
    let environment = BTreeMap::new();
    let resolution = ResolutionContext {
        catalog: &catalog,
        known_model_names: &names,
        custom_providers: &names,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let request = BatchCostRequest {
        model: &input.model,
        custom_llm_provider: input.custom_llm_provider.as_deref(),
        usage: &usage,
        prompt_extras: &extras,
        supplied_model_info: input.model_info.as_ref(),
        data_residency: input.data_residency.as_deref(),
    };
    let result = batch_cost_per_token(&request, &NoLookup, &resolution)?;
    Ok(json!({
        "prompt_cost": result.prompt_cost.normalize().to_string(),
        "completion_cost": result.completion_cost.normalize().to_string(),
    }))
}

fn run_prompt(input: &PromptInput) -> Result<serde_json::Value, BatchCostError> {
    let usage = CostUsage::from(&input.usage);
    let extras = input
        .prompt_extras
        .as_ref()
        .map_or_else(PromptExtras::default, PromptExtras::from);
    let cache_rates = PromptCacheRates {
        read: input.cache_read_rate,
        creation: input.cache_creation_rate,
    };
    let cost = batch_prompt_cost(
        &usage,
        &extras,
        &input.model_info,
        input.input_rate,
        cache_rates,
    )?;
    Ok(json!({ "prompt_cost": cost.normalize().to_string() }))
}
