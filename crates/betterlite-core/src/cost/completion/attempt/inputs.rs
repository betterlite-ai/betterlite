use std::error::Error;

use betterlite_models::ocr_pricing::OcrPricing;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord};
use betterlite_models::tiered_pricing::Scalar;
use betterlite_models::usage_sum::Usage as PricingUsage;
use betterlite_openai::api::rerank_contracts::records::RerankBilledUnits;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::cost::call_price::CallPriceInput;
use crate::cost::completion::args::CompletionArgs;
use crate::cost::completion::error::{ChargeInputError, CompletionCostError};
use crate::cost::completion::usage::pricing_usage;
use crate::cost::context::ocr::ocr_info;
use crate::cost::context::pricing::PricingQuery;
use crate::cost::prepare::{AttemptBilling, PreparedBilling};

// One candidate being priced: the call's inputs, the name with the provider and region it resolved to, the timing and the token counts.
pub(super) struct Attempt<'attempt> {
    pub args: &'attempt CompletionArgs<'attempt>,
    pub prepared: &'attempt PreparedBilling,
    pub moment: DateTime<Utc>,
    pub model: &'attempt str,
    pub provider: Option<&'attempt str>,
    pub region: Option<&'attempt str>,
    pub request_model_for_cost: Option<&'attempt str>,
    pub response_time_ms: Decimal,
    pub billing: &'attempt AttemptBilling,
}

// The values a CallPriceInput borrows, converted once per attempt.
pub(super) struct Owned {
    usage: Option<PricingUsage>,
    ocr_prices: Option<PriceRecord>,
    rerank: Option<RerankBilledUnits>,
    prompt_characters: Option<i64>,
    completion_characters: Option<i64>,
}

// _ocr_model_info returns the three OCR rates as a record; an absent rate is left out so the next layer supplies it.
fn ocr_record(pricing: &OcrPricing) -> Result<PriceRecord, ChargeInputError> {
    let mut record = PriceRecord::new();
    for (key, price) in [
        ("ocr_cost_per_page", pricing.ocr_cost_per_page),
        ("ocr_cost_per_credit", pricing.ocr_cost_per_credit),
        ("annotation_cost_per_page", pricing.annotation_cost_per_page),
    ] {
        if let Some(rate) = price {
            let number = rate
                .to_string()
                .parse::<serde_json::Number>()
                .map_err(ChargeInputError::OcrRate)?;
            record.insert(key.to_owned(), Scalar::Number(number));
        }
    }
    Ok(record)
}

fn character_count(
    field: &'static str,
    count: Option<u64>,
) -> Result<Option<i64>, ChargeInputError> {
    count
        .map(i64::try_from)
        .transpose()
        .map_err(|source| ChargeInputError::CharacterCount { field, source })
}

impl Owned {
    pub(super) fn new<ImageError: Error + 'static>(
        attempt: &Attempt<'_>,
        lookup: &dyn PriceLookup,
    ) -> Result<Self, CompletionCostError<ImageError>> {
        let (args, prepared, billing) = (attempt.args, attempt.prepared, attempt.billing);
        let query = PricingQuery {
            custom_pricing: args.billing.custom_pricing,
            router_model_id: args.billing.router_model_id,
            logging_params: args.charge.logging.params,
        };
        let ocr = ocr_info(&query, lookup)?;
        Ok(Self {
            usage: prepared
                .cost_usage
                .as_ref()
                .map(pricing_usage)
                .transpose()?,
            ocr_prices: ocr.as_ref().map(ocr_record).transpose()?,
            rerank: billing
                .rerank_units
                .as_ref()
                .map(|units| RerankBilledUnits {
                    search_units: Some(units.search_units),
                    total_tokens: Some(units.total_tokens),
                }),
            prompt_characters: character_count("prompt_characters", billing.prompt_characters)?,
            completion_characters: character_count(
                "completion_characters",
                billing.completion_characters,
            )?,
        })
    }
}

// The cost_per_token arguments of the token stage: region_name is dropped under explicit pricing, and request_model is never forwarded.
pub(super) fn price_input<'input>(
    attempt: &Attempt<'input>,
    owned: &'input Owned,
) -> CallPriceInput<'input> {
    let (prepared, billing) = (attempt.prepared, attempt.billing);
    let charge = &attempt.args.charge;
    CallPriceInput {
        model: Some(attempt.model),
        prompt_tokens: billing.prompt_tokens,
        completion_tokens: billing.completion_tokens,
        response_time_ms: Some(attempt.response_time_ms),
        custom_llm_provider: attempt.provider,
        region_name: if prepared.explicit_pricing {
            None
        } else {
            attempt.region
        },
        prompt_characters: owned.prompt_characters,
        completion_characters: owned.completion_characters,
        cache_creation_input_tokens: Some(billing.cache_creation_input_tokens),
        cache_read_input_tokens: Some(billing.cache_read_input_tokens),
        custom_cost_per_token: charge.custom_cost_per_token,
        custom_cost_per_second: charge.custom_cost_per_second,
        number_of_queries: None,
        usage: owned.usage.as_ref(),
        rerank_billed_units: owned.rerank.as_ref(),
        call_type: prepared.call_type,
        audio_seconds: billing.transcription_duration,
        service_tier: prepared.service_tier.as_deref(),
        data_residency: charge.data_residency,
        vertex_location: charge.vertex_location,
        vector_response: charge.vector_response,
        ocr_billing: charge.ocr_billing,
        search_hidden_params: charge.search_hidden_params,
        request_model: None,
        custom_model_info: owned.ocr_prices.as_ref(),
        server_tool_projection: charge.server_tool_projection,
    }
}

mod tests;
