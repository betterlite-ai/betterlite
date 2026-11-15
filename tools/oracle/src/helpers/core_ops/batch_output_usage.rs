use std::collections::{BTreeMap, BTreeSet};

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::batch_output_usage::error::BatchOutputUsageError;
use betterlite_core::api::batch_output_usage::pricing::BatchRowPricer;
use betterlite_core::api::batch_output_usage::{
    BatchOutputProviders, BatchOutputUsage, BatchOutputUsageInput, BatchOutputUsageResult,
    ConverseUsageOutcome, VertexRowStats,
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
use betterlite_models::usage_sum::Usage as SummedUsage;
use betterlite_vertex::gemini_usage::usage::calculate_usage;
use betterlite_vertex::vertex_batch_records::rows::{
    RowCostCalculator, native_vertex_batch_row_stats,
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

// The pinned provider literal excludes bedrock, so the adapter's bedrock seams answer nothing.
struct Providers;

impl BatchOutputProviders for Providers {
    fn bedrock_embedding_usage(
        &self,
        _response_body: &IndexMap<String, Json>,
    ) -> Option<SummedUsage> {
        None
    }

    fn bedrock_converse_usage(&self, _usage: &Json) -> ConverseUsageOutcome {
        ConverseUsageOutcome::NotConverse
    }

    fn vertex_row_stats(
        &self,
        row: &Json,
        model_name: Option<&str>,
        model_info: Option<&ModelInfo>,
        pricer: &dyn BatchRowPricer,
    ) -> Option<VertexRowStats> {
        native_vertex_batch_row_stats(
            row,
            model_name,
            model_info,
            calculate_usage,
            &PricerBridge(pricer),
        )
        .ok()
        .flatten()
        .map(|native| VertexRowStats {
            usage: native.usage,
            total_tokens: native.total_tokens,
            model: native.model,
            prompt_cost: native.prompt_cost,
            completion_cost: native.completion_cost,
        })
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
    Ok(result_value(&service.run(input)?))
}

fn result_value(result: &BatchOutputUsageResult) -> Value {
    json!({
        "cost": result.cost.normalize().to_string(),
        "prompt_cost": result.prompt_cost.normalize().to_string(),
        "completion_cost": result.completion_cost.normalize().to_string(),
        "usage": {
            "prompt_tokens": result.usage.prompt_tokens,
            "completion_tokens": result.usage.completion_tokens,
            "total_tokens": result.usage.total_tokens,
            "reasoning_tokens": result.usage.reasoning_tokens,
            "cache_read_tokens": result.usage.cache_read_tokens,
            "cache_creation_tokens": result.usage.cache_creation_tokens,
        },
        "models": result.models,
        "successful_requests": result.successful_requests,
        "failed_requests": result.failed_requests,
        "diagnostics": result.diagnostics,
    })
}
