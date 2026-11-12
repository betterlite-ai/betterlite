mod anthropic;
mod azure;
mod characters;
pub mod generic_price;
mod tokens;

use betterlite_models::duration_cost::error::DurationCostError;
use betterlite_models::tiered_pricing::{Scalar, coerce_cost_per_token};
use betterlite_models::token_cost::GenericCostResult;
use rust_decimal::Decimal;

use super::Call;
use super::error::CallPriceError;
use super::policy::VertexRoute;
use generic_price::generic;
use tokens::policy_tokens;

pub fn by_provider(call: &Call<'_>) -> Result<GenericCostResult, CallPriceError> {
    let model = call.names.model.as_str();
    match call.names.provider.as_str() {
        "vertex_ai" => vertex(call),
        "anthropic" | "bedrock" => anthropic::anthropic_family(call),
        "azure_ai" => azure::azure_ai(call),
        "openai" => generic(call, model, call.input.data_residency),
        "azure" => generic(call, model, None),
        "databricks" | "gemini" => policy_tokens(call, model, call.input.service_tier, None),
        "fireworks_ai" | "deepseek" | "tencent" | "perplexity" | "xai" | "lemonade"
        | "dashscope" | "qwencloud" | "qwen_ai_platform" => policy_tokens(call, model, None, None),
        _ => other(call),
    }
}

fn vertex(call: &Call<'_>) -> Result<GenericCostResult, CallPriceError> {
    let model = call.names.without_prefix.as_str();
    let input = call.input;
    match call.env.policy.vertex_route(model, input.call_type)? {
        VertexRoute::Character => characters::vertex_characters(call, model),
        VertexRoute::Token => policy_tokens(call, model, input.service_tier, input.vertex_location),
    }
}

// Providers without a native formula bill generic token prices only when the catalog declares token or tiered rates.
fn other(call: &Call<'_>) -> Result<GenericCostResult, CallPriceError> {
    let model = call.names.model.as_str();
    let record = call
        .env
        .lookup
        .effective_model_info(model, Some(&call.names.provider))
        .ok_or_else(|| DurationCostError::UnknownModel {
            model: model.to_owned(),
        })?;
    let mut declared = false;
    for rate in ["input_cost_per_token", "output_cost_per_token"] {
        if declared {
            break;
        }
        declared = match record.get(rate) {
            None | Some(Scalar::Null) => false,
            present => coerce_cost_per_token(present)? > Decimal::ZERO,
        };
    }
    declared = declared || !matches!(record.get("tiered_pricing"), None | Some(Scalar::Null));
    if !declared {
        return Ok(GenericCostResult {
            prompt_cost: Decimal::ZERO,
            completion_cost: Decimal::ZERO,
        });
    }
    generic(call, model, call.input.data_residency)
}
