pub mod calls;
pub mod custom;
pub mod error;
pub mod model_names;
pub mod policy;
pub mod providers;
mod tests;
pub mod usage;

use betterlite_models::cost_usage::server_tool_projection::ServerToolProjection;
use betterlite_models::duration_cost::wall_clock::per_second_pricing_cost;
use betterlite_models::ocr_pricing::OcrBilling;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord};
use betterlite_models::resolution::ResolutionContext;
use betterlite_models::token_cost::GenericCostResult;
use betterlite_models::usage_sum::Usage;
use betterlite_openai::api::rerank_contracts::records::RerankBilledUnits;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResponse;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

use crate::routes::CallType;
use error::CallPriceError;
use model_names::ModelNames;
use policy::ProviderPricePolicy;
use usage::UsageView;

#[derive(Debug, Clone, Copy)]
pub struct CustomCostPerToken {
    pub input_cost_per_token: Decimal,
    pub output_cost_per_token: Decimal,
    pub cache_read_input_token_cost: Option<Decimal>,
    pub cache_creation_cost: Option<Decimal>,
}

// Every argument cost_per_token reads; absent counters stay absent so the cache normalization can tell them from zero.
pub struct CallPriceInput<'input> {
    pub model: Option<&'input str>,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub response_time_ms: Option<Decimal>,
    pub custom_llm_provider: Option<&'input str>,
    pub region_name: Option<&'input str>,
    pub prompt_characters: Option<i64>,
    pub completion_characters: Option<i64>,
    pub cache_creation_input_tokens: Option<i64>,
    pub cache_read_input_tokens: Option<i64>,
    pub custom_cost_per_token: Option<CustomCostPerToken>,
    pub custom_cost_per_second: Option<Decimal>,
    pub number_of_queries: Option<i64>,
    pub usage: Option<&'input Usage>,
    pub rerank_billed_units: Option<&'input RerankBilledUnits>,
    pub call_type: CallType,
    pub audio_seconds: Decimal,
    pub service_tier: Option<&'input str>,
    pub data_residency: Option<&'input str>,
    pub vertex_location: Option<&'input str>,
    pub vector_response: Option<&'input VectorStoreSearchResponse>,
    pub ocr_billing: Option<&'input OcrBilling>,
    pub search_hidden_params: Option<&'input PriceRecord>,
    pub request_model: Option<&'input str>,
    pub custom_model_info: Option<&'input PriceRecord>,
    pub server_tool_projection: Option<&'input ServerToolProjection>,
}

pub struct CallPriceEnv<'env> {
    pub lookup: &'env dyn PriceLookup,
    pub policy: &'env dyn ProviderPricePolicy,
    pub resolution: &'env ResolutionContext<'env>,
    pub database: &'env TimeZoneDatabase,
    pub moment: DateTime<Utc>,
}

// What each pricing branch reads once the model names and the usage projection are fixed.
pub struct Call<'call> {
    pub input: &'call CallPriceInput<'call>,
    pub env: &'call CallPriceEnv<'call>,
    pub names: ModelNames,
    pub view: UsageView,
}

// Ports cost_per_token: custom prices return first, then the call type, wall-clock seconds and the provider decide.
pub fn call_price(
    input: &CallPriceInput<'_>,
    env: &CallPriceEnv<'_>,
) -> Result<GenericCostResult, CallPriceError> {
    let Some(model) = input.model else {
        return Err(CallPriceError::NullModel);
    };
    if let Some(custom) = custom::custom_price(input)? {
        return Ok(custom);
    }
    let names = model_names::select(model, input, env)?;
    let view = usage::usage_view(input)?;
    let call = Call {
        input,
        env,
        names,
        view,
    };
    if let Some(priced) = calls::by_call_type(&call)? {
        return Ok(priced);
    }
    if let Some((prompt_cost, completion_cost)) = per_second_pricing_cost(
        &call.names.model,
        Some(&call.names.provider),
        input.response_time_ms,
        env.lookup,
    )? {
        return Ok(GenericCostResult {
            prompt_cost,
            completion_cost,
        });
    }
    providers::by_provider(&call)
}
