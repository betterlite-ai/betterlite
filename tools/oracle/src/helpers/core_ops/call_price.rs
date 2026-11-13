use std::collections::{BTreeMap, BTreeSet};

use betterlite_core::cost::call_price::{
    CallPriceEnv, CallPriceInput, CustomCostPerToken, call_price,
};
use betterlite_core::routes::CallType;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_models::usage_sum::{PromptTokensDetails, Usage, UsageNumber};
use betterlite_provider::pricing::ProviderPricing;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rates {
    input_cost_per_token: Decimal,
    output_cost_per_token: Decimal,
    cache_read_input_token_cost: Option<Decimal>,
    cache_creation_cost: Option<Decimal>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UsageInput {
    prompt_tokens: i64,
    completion_tokens: i64,
    detail_cache_read: Option<i64>,
    detail_cache_creation: Option<i64>,
    top_level_cache_read: Option<i64>,
    top_level_cache_creation: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model: Option<String>,
    prompt_tokens: i64,
    completion_tokens: i64,
    custom_llm_provider: Option<String>,
    custom_cost_per_token: Option<Rates>,
    custom_cost_per_second: Option<Decimal>,
    response_time_ms: Option<Decimal>,
    cache_creation_input_tokens: Option<i64>,
    cache_read_input_tokens: Option<i64>,
    usage: Option<UsageInput>,
}

// The cases price through custom rates only, so no catalog entry is ever read.
struct EmptyCatalog;

impl PriceLookup for EmptyCatalog {
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

fn usage_block(fields: &UsageInput) -> Usage {
    let mut numeric_extras = BTreeMap::new();
    if let Some(count) = fields.top_level_cache_read {
        numeric_extras.insert(
            "cache_read_input_tokens".to_owned(),
            UsageNumber::Integer(count),
        );
    }
    if let Some(count) = fields.top_level_cache_creation {
        numeric_extras.insert(
            "cache_creation_input_tokens".to_owned(),
            UsageNumber::Integer(count),
        );
    }
    Usage {
        prompt_tokens: fields.prompt_tokens,
        completion_tokens: fields.completion_tokens,
        total_tokens: fields
            .prompt_tokens
            .saturating_add(fields.completion_tokens),
        // LiteLLM mirrors cache_write_tokens and cache_creation_tokens into each other.
        prompt_tokens_details: Some(PromptTokensDetails {
            text_tokens: None,
            audio_tokens: None,
            image_tokens: None,
            video_tokens: None,
            web_search_requests: None,
            google_maps_grounding_requests: None,
            tool_use_tokens: None,
            character_count: None,
            image_count: None,
            video_length_seconds: None,
            audio_length_seconds: None,
            query_count: None,
            cache_write_tokens: fields.detail_cache_creation,
            cache_creation_tokens: fields.detail_cache_creation,
            cached_tokens: fields.detail_cache_read,
            cached_tokens_details: None,
        }),
        completion_tokens_details: None,
        numeric_extras,
    }
}

fn request<'input>(input: &'input Input, usage: Option<&'input Usage>) -> CallPriceInput<'input> {
    CallPriceInput {
        model: input.model.as_deref(),
        prompt_tokens: input.prompt_tokens,
        completion_tokens: input.completion_tokens,
        response_time_ms: input.response_time_ms,
        custom_llm_provider: input.custom_llm_provider.as_deref(),
        region_name: None,
        prompt_characters: None,
        completion_characters: None,
        cache_creation_input_tokens: input.cache_creation_input_tokens,
        cache_read_input_tokens: input.cache_read_input_tokens,
        custom_cost_per_token: input.custom_cost_per_token.as_ref().map(|rates| {
            CustomCostPerToken {
                input_cost_per_token: rates.input_cost_per_token,
                output_cost_per_token: rates.output_cost_per_token,
                cache_read_input_token_cost: rates.cache_read_input_token_cost,
                cache_creation_cost: rates.cache_creation_cost,
            }
        }),
        custom_cost_per_second: input.custom_cost_per_second,
        number_of_queries: None,
        usage,
        rerank_billed_units: None,
        call_type: CallType::Completion,
        audio_seconds: Decimal::ZERO,
        service_tier: None,
        data_residency: None,
        vertex_location: None,
        vector_response: None,
        ocr_billing: None,
        search_hidden_params: None,
        request_model: None,
        custom_model_info: None,
        server_tool_projection: None,
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let usage = input.usage.as_ref().map(usage_block);
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
        let priced = call_price(
            &request(input, usage.as_ref()),
            &CallPriceEnv {
                lookup: &EmptyCatalog,
                policy: &ProviderPricing,
                resolution: &resolution,
                database: &TimeZoneDatabase::from_env(),
                moment: DateTime::<Utc>::UNIX_EPOCH,
            },
        )
        .map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "CallPriceError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        Ok(json!({
            "prompt_cost": priced.prompt_cost.normalize().to_string(),
            "completion_cost": priced.completion_cost.normalize().to_string(),
        }))
    }
}
