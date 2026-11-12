use betterlite_models::duration_cost::character::{
    CharacterCostRequest, CostMetric, generic_cost_per_character, select_cost_metric_for_model,
};
use betterlite_models::duration_cost::error::DurationCostError;
use betterlite_models::duration_cost::openai_seconds::cost_per_second;
use betterlite_models::ocr_pricing::ocr_cost;
use betterlite_models::resolution::{ResolutionRequest, resolve};
use betterlite_models::token_cost::GenericCostResult;
use betterlite_openai::api::rerank_contracts::records::RerankBilledUnits;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResponse;
use rust_decimal::Decimal;

use super::error::CallPriceError;
use super::policy::{RerankPriceRequest, SearchPriceRequest, VectorStorePriceRequest};
use super::providers::generic;
use super::{Call, CallPriceEnv};
use crate::cost::batch::{BatchCostRequest, batch_cost_per_token};
use crate::routes::CallType;

// A call type with its own price path answers here; None leaves the call to wall-clock and provider pricing.
pub fn by_call_type(call: &Call<'_>) -> Result<Option<GenericCostResult>, CallPriceError> {
    let input = call.input;
    let call_type = input.call_type;
    let provider = call.names.provider.as_str();
    let model = call.names.model.as_str();
    let price = if matches!(call_type, CallType::Speech | CallType::Aspeech) {
        speech(call)?
    } else if matches!(call_type, CallType::Rerank | CallType::Arerank) {
        rerank_cost(model, Some(provider), input.rerank_billed_units, call.env)?
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
        batch(call)?
    } else if matches!(
        call_type,
        CallType::Transcription | CallType::Atranscription
    ) {
        transcription(call)?
    } else if matches!(call_type, CallType::Search | CallType::Asearch) {
        search(call)?
    } else {
        return Ok(None);
    };
    Ok(Some(price))
}

fn vector(call: &Call<'_>) -> Result<GenericCostResult, CallPriceError> {
    let response = call
        .input
        .vector_response
        .ok_or(CallPriceError::MissingInput {
            call_type: "vector_store_search",
            field: "response",
        })?;
    vector_store_search_cost(
        Some(&call.names.model),
        Some(&call.names.provider),
        response,
        call.env,
    )
}

fn search(call: &Call<'_>) -> Result<GenericCostResult, CallPriceError> {
    let input = call.input;
    Ok(call.env.policy.search_cost(&SearchPriceRequest {
        provider: &call.names.provider,
        model: &call.names.model,
        number_of_queries: input
            .number_of_queries
            .filter(|count| *count != 0)
            .unwrap_or(1),
        hidden_params: input.search_hidden_params,
    })?)
}

fn ocr(call: &Call<'_>) -> Result<GenericCostResult, CallPriceError> {
    let billing = call.input.ocr_billing.ok_or(CallPriceError::MissingInput {
        call_type: "ocr",
        field: "response",
    })?;
    let cost = ocr_cost(
        &call.names.model,
        Some(&call.names.provider),
        billing,
        call.input.custom_model_info,
        call.env.lookup,
    )?;
    Ok(GenericCostResult {
        prompt_cost: cost,
        completion_cost: Decimal::ZERO,
    })
}

fn batch(call: &Call<'_>) -> Result<GenericCostResult, CallPriceError> {
    let priced = batch_cost_per_token(
        &BatchCostRequest {
            model: &call.names.model,
            custom_llm_provider: Some(&call.names.provider),
            usage: &call.view.cost,
            prompt_extras: &call.view.extras,
            supplied_model_info: None,
            data_residency: call.input.data_residency,
        },
        call.env.lookup,
        call.env.resolution,
    )?;
    Ok(GenericCostResult {
        prompt_cost: priced.prompt_cost,
        completion_cost: priced.completion_cost,
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
        return generic(
            call,
            model,
            provider,
            input.service_tier,
            input.data_residency,
        );
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

// Positive token evidence bills tokens; otherwise the audio duration bills.
fn transcription(call: &Call<'_>) -> Result<GenericCostResult, CallPriceError> {
    let input = call.input;
    let provider = call.names.provider.as_str();
    let model = call.names.without_prefix.as_str();
    let usage = &call.view.cost;
    let details_have_tokens = usage.prompt_details.is_some_and(|details| {
        details.audio_tokens.unwrap_or(0) > 0 || details.text_tokens.unwrap_or(0) > 0
    });
    if details_have_tokens || usage.prompt_tokens > 0 || usage.completion_tokens > 0 {
        return generic(
            call,
            model,
            provider,
            input.service_tier,
            input.data_residency,
        );
    }
    let (prompt_cost, completion_cost) =
        cost_per_second(model, Some(provider), input.audio_seconds, call.env.lookup)?;
    Ok(GenericCostResult {
        prompt_cost,
        completion_cost,
    })
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

// Ports vector_store_search_cost: a slash in the model re-resolves the provider, and no vector configuration prices at zero.
pub fn vector_store_search_cost(
    model: Option<&str>,
    custom_llm_provider: Option<&str>,
    response: &VectorStoreSearchResponse,
    env: &CallPriceEnv<'_>,
) -> Result<GenericCostResult, CallPriceError> {
    let resolved = model
        .filter(|name| name.contains('/'))
        .map(|slashed| {
            resolve(
                &ResolutionRequest {
                    model: slashed,
                    custom_llm_provider: None,
                    api_base: None,
                    api_key: None,
                },
                env.resolution,
            )
        })
        .transpose()
        .map_err(|source| CallPriceError::Resolution { source })?;
    let provider = resolved.as_ref().map_or_else(
        || custom_llm_provider.unwrap_or("openai"),
        |found| found.provider.slug(),
    );
    let priced = env.policy.vector_store_cost(&VectorStorePriceRequest {
        provider,
        api_type: resolved.as_ref().map(|found| found.model.as_str()),
        response,
    })?;
    Ok(priced.unwrap_or(GenericCostResult {
        prompt_cost: Decimal::ZERO,
        completion_cost: Decimal::ZERO,
    }))
}
