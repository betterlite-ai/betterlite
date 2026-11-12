use betterlite_foundry::pricing::response::{ResponseCostRequest, cost_per_token};
use betterlite_models::anthropic_pricing::{
    AnthropicPriceRequest, BedrockPriceRequest, TokenPriceRequest, token_cost,
};
use betterlite_models::duration_cost::error::DurationCostError;
use betterlite_models::tiered_pricing::{Scalar, coerce_cost_per_token};
use betterlite_models::token_cost::{
    GenericCostRequest, GenericCostResult, generic_cost_per_token,
};
use rust_decimal::Decimal;

use super::Call;
use super::error::CallPriceError;
use super::policy::{CharacterPriceRequest, TokenPriceRequest as PolicyTokenRequest, VertexRoute};

pub fn generic(
    call: &Call<'_>,
    model: &str,
    provider: &str,
    service_tier: Option<&str>,
    data_residency: Option<&str>,
) -> Result<GenericCostResult, CallPriceError> {
    Ok(generic_cost_per_token(
        &GenericCostRequest {
            model,
            custom_llm_provider: Some(provider),
            usage: &call.view.cost,
            prompt_extras: &call.view.extras,
            supplied_pricing: None,
            service_tier,
            data_residency,
            vertex_location: None,
            moment: call.env.moment,
        },
        call.env.lookup,
        call.env.database,
    )?)
}

pub fn by_provider(call: &Call<'_>) -> Result<GenericCostResult, CallPriceError> {
    let input = call.input;
    let provider = call.names.provider.as_str();
    let model = call.names.model.as_str();
    match provider {
        "vertex_ai" => vertex(call),
        "anthropic" | "bedrock" => anthropic_family(call),
        "openai" => generic(
            call,
            model,
            provider,
            input.service_tier,
            input.data_residency,
        ),
        "azure" => generic(call, model, provider, input.service_tier, None),
        "azure_ai" => Ok(cost_per_token(
            &ResponseCostRequest {
                model,
                usage: &call.view.cost,
                prompt_extras: &call.view.extras,
                response_time_ms: input.response_time_ms,
                request_model: input.request_model,
                service_tier: input.service_tier,
                moment: call.env.moment,
            },
            call.env.lookup,
            call.env.database,
        )?),
        "databricks" | "gemini" => policy_tokens(call, input.service_tier),
        "fireworks_ai" | "deepseek" | "tencent" | "perplexity" | "xai" | "lemonade"
        | "dashscope" | "qwencloud" | "qwen_ai_platform" => policy_tokens(call, None),
        _ => other(call),
    }
}

fn anthropic_family(call: &Call<'_>) -> Result<GenericCostResult, CallPriceError> {
    let input = call.input;
    let model = call.names.model.as_str();
    let projection = input.server_tool_projection.cloned().unwrap_or_default();
    let request = if call.names.provider == "anthropic" {
        TokenPriceRequest::Anthropic(AnthropicPriceRequest {
            model,
            usage: &call.view.cost,
            prompt_extras: &call.view.extras,
            supplied_model_info: None,
            provider_specific_entry: None,
            service_tier: input.service_tier,
            projection: &projection,
            moment: call.env.moment,
        })
    } else {
        TokenPriceRequest::Bedrock(BedrockPriceRequest {
            model,
            usage: &call.view.cost,
            prompt_extras: &call.view.extras,
            service_tier: input.service_tier,
            moment: call.env.moment,
        })
    };
    Ok(token_cost(&request, call.env.lookup, call.env.database)?)
}

fn policy_tokens(
    call: &Call<'_>,
    service_tier: Option<&str>,
) -> Result<GenericCostResult, CallPriceError> {
    Ok(call.env.policy.token_cost(&PolicyTokenRequest {
        provider: &call.names.provider,
        model: &call.names.model,
        usage: &call.view.cost,
        prompt_extras: &call.view.extras,
        service_tier,
        vertex_location: None,
    })?)
}

fn vertex(call: &Call<'_>) -> Result<GenericCostResult, CallPriceError> {
    let input = call.input;
    let model = call.names.without_prefix.as_str();
    let route = call.env.policy.vertex_route(model, input.call_type)?;
    if route == VertexRoute::Character {
        return Ok(call.env.policy.character_cost(&CharacterPriceRequest {
            provider: &call.names.provider,
            model,
            prompt_characters: input.prompt_characters,
            completion_characters: input.completion_characters,
            usage: &call.view.cost,
            prompt_extras: &call.view.extras,
            service_tier: input.service_tier,
            vertex_location: input.vertex_location,
        })?);
    }
    Ok(call.env.policy.token_cost(&PolicyTokenRequest {
        provider: &call.names.provider,
        model,
        usage: &call.view.cost,
        prompt_extras: &call.view.extras,
        service_tier: input.service_tier,
        vertex_location: input.vertex_location,
    })?)
}

// Providers without a native formula bill generic token prices only when the catalog declares token or tiered rates.
fn other(call: &Call<'_>) -> Result<GenericCostResult, CallPriceError> {
    let model = call.names.model.as_str();
    let provider = call.names.provider.as_str();
    let record = call
        .env
        .lookup
        .effective_model_info(model, Some(provider))
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
    generic(
        call,
        model,
        provider,
        call.input.service_tier,
        call.input.data_residency,
    )
}
