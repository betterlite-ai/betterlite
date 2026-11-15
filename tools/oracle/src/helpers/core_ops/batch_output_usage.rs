use std::collections::{BTreeMap, BTreeSet};

use betterlite_bedrock::batch_create::titan::titan_batch_token_usage;
use betterlite_bedrock::converse_usage::Usage as ConverseUsage;
use betterlite_bedrock::converse_usage::transform::usage_from_batch_output;
use betterlite_bedrock::converse_usage::wire::{ConverseTokenUsage, is_converse_usage_shape};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::batch_output_usage::{
    BatchOutputProviders, BatchOutputUsage, BatchOutputUsageError, BatchOutputUsageInput,
    BatchOutputUsageResult, BatchRowPricer, BatchUsageTotals, ConverseUsageOutcome, VertexRowStats,
};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_models::token_cost::GenericCostResult;
use betterlite_models::token_cost::error::TokenCostError;
use betterlite_models::token_rates::ModelInfo;
use betterlite_models::usage_sum::{
    CompletionTokensDetails, PromptTokensDetails, Usage as SummedUsage,
};
use betterlite_vertex::gemini_usage::usage::calculate_usage;
use betterlite_vertex::vertex_batch_records::rows::{
    NativeVertexBatchRowStats, RowCostCalculator, native_vertex_batch_row_stats,
};
use indexmap::IndexMap;
use serde_json::{Value, json};

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

struct PricerBridge<'pricer>(&'pricer dyn BatchRowPricer);

impl RowCostCalculator for PricerBridge<'_> {
    fn calculate(
        &self,
        usage: &SummedUsage,
        model: &str,
        custom_llm_provider: &str,
        model_info: Option<&ModelInfo>,
    ) -> Result<GenericCostResult, TokenCostError> {
        let Some(cost) = self
            .0
            .price(usage, model, custom_llm_provider, model_info)
            .ok()
        else {
            return Err(TokenCostError::UnknownModel);
        };
        Ok(GenericCostResult {
            prompt_cost: cost.prompt_cost,
            completion_cost: cost.completion_cost,
        })
    }
}

struct Providers;

impl BatchOutputProviders for Providers {
    fn bedrock_embedding_usage(
        &self,
        response_body: &IndexMap<String, Json>,
    ) -> Option<SummedUsage> {
        let value = Json::Object(response_body.clone());
        let usage = titan_batch_token_usage(&value)?;
        Some(SummedUsage {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
            prompt_tokens_details: None,
            completion_tokens_details: None,
            numeric_extras: BTreeMap::new(),
        })
    }

    fn bedrock_converse_usage(&self, usage: &Json) -> ConverseUsageOutcome {
        if !is_converse_usage_shape(usage) {
            return ConverseUsageOutcome::NotConverse;
        }
        let Ok(decoded) = ConverseTokenUsage::decode(usage) else {
            return ConverseUsageOutcome::Invalid;
        };
        let Some(converted) = usage_from_batch_output(&decoded).ok() else {
            return ConverseUsageOutcome::Invalid;
        };
        ConverseUsageOutcome::Usage(converse_usage(&converted))
    }

    fn vertex_row_stats(
        &self,
        row: &Json,
        model_name: Option<&str>,
        model_info: Option<&ModelInfo>,
        pricer: &dyn BatchRowPricer,
    ) -> Option<VertexRowStats> {
        let Ok(Some(NativeVertexBatchRowStats {
            usage,
            total_tokens,
            model,
            prompt_cost,
            completion_cost,
        })) = native_vertex_batch_row_stats(
            row,
            model_name,
            model_info,
            calculate_usage,
            &PricerBridge(pricer),
        )
        else {
            return None;
        };
        Some(VertexRowStats {
            usage,
            total_tokens,
            model,
            prompt_cost,
            completion_cost,
        })
    }
}

fn converse_usage(usage: &ConverseUsage) -> SummedUsage {
    SummedUsage {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
        prompt_tokens_details: Some(PromptTokensDetails {
            text_tokens: Some(usage.prompt_tokens_details.text_tokens),
            audio_tokens: usage.prompt_tokens_details.audio_tokens,
            image_tokens: usage.prompt_tokens_details.image_tokens,
            video_tokens: usage.prompt_tokens_details.video_tokens,
            web_search_requests: None,
            google_maps_grounding_requests: None,
            tool_use_tokens: None,
            character_count: None,
            image_count: None,
            video_length_seconds: None,
            audio_length_seconds: None,
            query_count: None,
            cache_write_tokens: Some(usage.prompt_tokens_details.cache_write_tokens),
            cache_creation_tokens: Some(usage.prompt_tokens_details.cache_creation_tokens),
            cached_tokens: Some(usage.prompt_tokens_details.cached_tokens),
            cached_tokens_details: None,
        }),
        completion_tokens_details: Some(CompletionTokensDetails {
            text_tokens: usage.completion_tokens_details.text_tokens,
            audio_tokens: usage.completion_tokens_details.audio_tokens,
            image_tokens: usage.completion_tokens_details.image_tokens,
            video_tokens: usage.completion_tokens_details.video_tokens,
            reasoning_tokens: usage.completion_tokens_details.reasoning_tokens,
            accepted_prediction_tokens: usage.completion_tokens_details.accepted_prediction_tokens,
            rejected_prediction_tokens: usage.completion_tokens_details.rejected_prediction_tokens,
        }),
        numeric_extras: BTreeMap::new(),
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = BatchOutputUsageInput;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        run_batch(input).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "BatchOutputUsageError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}

fn run_batch(input: &BatchOutputUsageInput) -> Result<Value, BatchOutputUsageError> {
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
    let service = BatchOutputUsage {
        limits: ApiLimits::default(),
        lookup: &NoLookup,
        resolution: &resolution,
        providers: &Providers,
    };
    let result = service.run(input)?;
    Ok(result_value(&result))
}

fn result_value(result: &BatchOutputUsageResult) -> Value {
    json!({
        "cost": result.cost.normalize().to_string(),
        "prompt_cost": result.prompt_cost.normalize().to_string(),
        "completion_cost": result.completion_cost.normalize().to_string(),
        "usage": usage_value(&result.usage),
        "models": result.models,
        "successful_requests": result.successful_requests,
        "failed_requests": result.failed_requests,
        "diagnostics": result.diagnostics,
    })
}

fn usage_value(usage: &BatchUsageTotals) -> Value {
    json!({
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "reasoning_tokens": usage.reasoning_tokens,
        "cache_read_tokens": usage.cache_read_tokens,
        "cache_creation_tokens": usage.cache_creation_tokens,
    })
}
