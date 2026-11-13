mod batch;
mod search;
mod transcription;
pub mod vector_store;

use betterlite_models::duration_cost::character::{
    CharacterCostRequest, CostMetric, generic_cost_per_character, select_cost_metric_for_model,
};
use betterlite_models::duration_cost::error::DurationCostError;
use betterlite_models::ocr_pricing::ocr_cost;
use betterlite_models::resolution::{ResolutionRequest, resolve};
use betterlite_models::token_cost::GenericCostResult;
use betterlite_openai::api::rerank_contracts::records::RerankBilledUnits;
use rust_decimal::Decimal;

use super::error::CallPriceError;
use super::policy::RerankPriceRequest;
use super::providers::generic_price::generic;
use super::{Call, CallPriceEnv};
use crate::routes::CallType;

// A call type with its own price path answers here; None leaves the call to wall-clock and provider pricing.
pub fn by_call_type(call: &Call<'_>) -> Result<Option<GenericCostResult>, CallPriceError> {
    let call_type = call.input.call_type;
    let model = call.names.model.as_str();
    let units = call.input.rerank_billed_units;
    let price = if matches!(call_type, CallType::Speech | CallType::Aspeech) {
        speech(call)?
    } else if matches!(call_type, CallType::Rerank | CallType::Arerank) {
        rerank_cost(model, Some(&call.names.provider), units, call.env)?
    } else if matches!(
        call_type,
        CallType::VectorStoreSearch | CallType::AvectorStoreSearch
    ) {
        vector(call)?
    } else if matches!(call_type, CallType::Ocr | CallType::Aocr) {
        ocr(call)?
    } else if matches!(
        call_type,
        CallType::RetrieveBatch | CallType::AretrieveBatch
    ) {
        batch::batch(call)?
    } else if matches!(
        call_type,
        CallType::Transcription | CallType::Atranscription
    ) {
        transcription::transcription(call)?
    } else if matches!(call_type, CallType::Search | CallType::Asearch) {
        search::search(call)?
    } else {
        return Ok(None);
    };
    Ok(Some(price))
}

fn vector(call: &Call<'_>) -> Result<GenericCostResult, CallPriceError> {
    let Some(response) = call.input.vector_response else {
        return Err(CallPriceError::MissingInput {
            call_type: "vector_store_search",
            field: "response",
        });
    };
    let (model, provider) = (&call.names.model, &call.names.provider);
    vector_store::vector_store_search_cost(Some(model), Some(provider), response, call.env)
}

fn ocr(call: &Call<'_>) -> Result<GenericCostResult, CallPriceError> {
    let input = call.input;
    let Some(billing) = input.ocr_billing else {
        return Err(CallPriceError::MissingInput {
            call_type: "ocr",
            field: "response",
        });
    };
    let provider = Some(call.names.provider.as_str());
    let model = call.names.model.as_str();
    let cost = ocr_cost(
        model,
        provider,
        billing,
        input.custom_model_info,
        call.env.lookup,
    )?;
    Ok(GenericCostResult {
        prompt_cost: cost,
        completion_cost: Decimal::ZERO,
    })
}

fn speech(call: &Call<'_>) -> Result<GenericCostResult, CallPriceError> {
    let input = call.input;
    let provider = call.names.provider.as_str();
    let model = call.names.without_prefix.as_str();
    if matches!(provider, "vertex_ai" | "vertex_ai_beta")
        && let Some(cost) = call.env.policy.lyria_cost(provider, model)?
    {
        return Ok(GenericCostResult {
            prompt_cost: Decimal::ZERO,
            completion_cost: cost,
        });
    }
    let record = call
        .env
        .lookup
        .effective_model_info(model, Some(provider))
        .ok_or_else(|| DurationCostError::UnknownModel {
            model: model.to_owned(),
        })?;
    if select_cost_metric_for_model(model, &record)? == CostMetric::Token {
        return generic(call, model, input.data_residency);
    }
    speech_characters(call, model, provider)
}

// Character-priced speech bills the prompt characters; the completion side is fixed at zero.
fn speech_characters(
    call: &Call<'_>,
    model: &str,
    provider: &str,
) -> Result<GenericCostResult, CallPriceError> {
    let Some(characters) = call.input.prompt_characters else {
        return Err(CallPriceError::MissingPromptCharacters {
            model: model.to_owned(),
            provider: provider.to_owned(),
        });
    };
    let priced = generic_cost_per_character(
        &CharacterCostRequest {
            model,
            custom_llm_provider: provider,
            prompt_characters: Decimal::from(characters),
            completion_characters: Decimal::ZERO,
            custom_prompt_cost: None,
            custom_completion_cost: Some(Decimal::ZERO),
        },
        call.env.lookup,
    )?;
    match (priced.prompt_cost, priced.completion_cost) {
        (Some(prompt_cost), Some(completion_cost)) => Ok(GenericCostResult {
            prompt_cost,
            completion_cost,
        }),
        _ => Err(CallPriceError::MissingSpeechCost {
            model: model.to_owned(),
            provider: provider.to_owned(),
        }),
    }
}

// Ports rerank_cost: the provider is resolved again, model info is a soft lookup and the provider's own config prices the units.
pub fn rerank_cost(
    model: &str,
    custom_llm_provider: Option<&str>,
    billed_units: Option<&RerankBilledUnits>,
    env: &CallPriceEnv<'_>,
) -> Result<GenericCostResult, CallPriceError> {
    let resolved = resolve(
        &ResolutionRequest {
            model,
            custom_llm_provider,
            api_base: None,
            api_key: None,
        },
        env.resolution,
    )
    .map_err(|source| CallPriceError::Resolution { source })?;
    let provider = resolved.provider.slug();
    let model_info = env.lookup.effective_model_info(model, Some(provider));
    Ok(env.policy.rerank_cost(&RerankPriceRequest {
        provider,
        model,
        billed_units,
        model_info: model_info.as_ref(),
    })?)
}
