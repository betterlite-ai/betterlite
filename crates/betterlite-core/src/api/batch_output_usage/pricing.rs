use betterlite_models::cost_usage::CostUsage;
use betterlite_models::cost_usage::modality_projection::CachedTokenCounters;
use betterlite_models::decisions::Json;
use betterlite_models::ocr_pricing::{OcrBatchUsage, ocr_batch_cost};
use betterlite_models::price_lookup::PriceLookup;
use betterlite_models::resolution::ResolutionContext;
use betterlite_models::token_cost::prompt_overlap::{PromptExtras, prompt_tokens_details};
use betterlite_models::token_rates::ModelInfo;
use betterlite_models::usage_sum::Usage as SummedUsage;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::BatchOutputProvider;
use super::decode::{integer_field, number_field};
use super::error::BatchLineError;
use crate::cost::batch::error::BatchCostError;
use crate::cost::batch::{BatchCostRequest, batch_cost_per_token};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchRowCost {
    pub prompt_cost: Decimal,
    pub completion_cost: Decimal,
}

pub trait BatchRowPricer {
    fn price(
        &self,
        usage: &SummedUsage,
        model: &str,
        custom_llm_provider: &str,
        model_info: Option<&ModelInfo>,
    ) -> Result<BatchRowCost, BatchCostError>;
}

// B068's batch pricing over the injected catalog, as the provider row calculators and the line pass see it.
pub struct BatchCostPricer<'lookup> {
    pub lookup: &'lookup dyn PriceLookup,
    pub resolution: &'lookup ResolutionContext<'lookup>,
}

impl BatchRowPricer for BatchCostPricer<'_> {
    fn price(
        &self,
        usage: &SummedUsage,
        model: &str,
        custom_llm_provider: &str,
        model_info: Option<&ModelInfo>,
    ) -> Result<BatchRowCost, BatchCostError> {
        let cost_usage = CostUsage::from(usage);
        let extras = usage_prompt_extras(usage);
        let request = BatchCostRequest {
            model,
            custom_llm_provider: Some(custom_llm_provider),
            usage: &cost_usage,
            prompt_extras: &extras,
            supplied_model_info: model_info,
            data_residency: None,
        };
        let priced = batch_cost_per_token(&request, self.lookup, self.resolution)?;
        Ok(BatchRowCost {
            prompt_cost: priced.prompt_cost,
            completion_cost: priced.completion_cost,
        })
    }
}

pub(super) struct LineCostInput<'line> {
    pub body: Option<&'line IndexMap<String, Json>>,
    pub usage: &'line SummedUsage,
    pub provider: BatchOutputProvider,
    pub model_name: Option<&'line str>,
    pub response_model: Option<&'line str>,
    pub model_info: Option<&'line ModelInfo>,
}

pub(super) fn line_cost(
    input: &LineCostInput<'_>,
    lookup: &dyn PriceLookup,
    resolution: &ResolutionContext<'_>,
) -> Result<BatchRowCost, BatchLineError> {
    let cost_model = cost_model(input);
    if let Some(ocr_usage) = ocr_usage(input.body)? {
        let cost = ocr_batch_cost(
            cost_model,
            Some(input.provider.slug()),
            &ocr_usage,
            input.model_info,
            lookup,
        )?;
        return Ok(BatchRowCost {
            prompt_cost: cost,
            completion_cost: Decimal::ZERO,
        });
    }
    let cost_usage = CostUsage::from(input.usage);
    let extras = usage_prompt_extras(input.usage);
    let request = BatchCostRequest {
        model: cost_model,
        custom_llm_provider: Some(input.provider.slug()),
        usage: &cost_usage,
        prompt_extras: &extras,
        supplied_model_info: input.model_info,
        data_residency: None,
    };
    let priced = batch_cost_per_token(&request, lookup, resolution)?;
    Ok(BatchRowCost {
        prompt_cost: priced.prompt_cost,
        completion_cost: priced.completion_cost,
    })
}

pub(super) fn cache_counters(usage: &SummedUsage) -> Result<(i64, i64), BatchLineError> {
    let cost_usage = CostUsage::from(usage);
    let extras = usage_prompt_extras(usage);
    let details = prompt_tokens_details(&cost_usage, &extras)?;
    Ok((details.cache_hit_tokens, details.cache_creation_tokens))
}

fn cost_model<'line>(input: &LineCostInput<'line>) -> &'line str {
    if input.provider == BatchOutputProvider::Bedrock
        && let Some(model_name) = input.model_name
    {
        return model_name;
    }
    input
        .response_model
        .or(input.model_name)
        .unwrap_or_default()
}

fn ocr_usage(
    body: Option<&IndexMap<String, Json>>,
) -> Result<Option<OcrBatchUsage>, BatchLineError> {
    let Some(body) = body else {
        return Ok(None);
    };
    let Some(Json::Object(fields)) = body.get("usage_info") else {
        return Ok(None);
    };
    let pages_processed = integer_field(fields, "pages_processed")?.unwrap_or(0);
    let pages_processed_annotation =
        integer_field(fields, "pages_processed_annotation")?.unwrap_or(0);
    number_field(fields, "credits")?;
    integer_field(fields, "doc_size_bytes")?;
    Ok(Some(OcrBatchUsage {
        pages_processed,
        pages_processed_annotation,
    }))
}

fn usage_prompt_extras(usage: &SummedUsage) -> PromptExtras {
    let Some(details) = usage.prompt_tokens_details.as_ref() else {
        return PromptExtras::default();
    };
    PromptExtras {
        cache_write_tokens: details.cache_write_tokens,
        cache_creation_tokens: details.cache_creation_tokens,
        cache_creation_token_details: None,
        cached_tokens_details: details.cached_tokens_details.as_ref().map(|cached| {
            CachedTokenCounters {
                text_tokens: cached.text_tokens,
                audio_tokens: cached.audio_tokens,
                image_tokens: cached.image_tokens,
            }
        }),
        character_count: details.character_count,
        image_count: details.image_count,
        video_length_seconds: details.video_length_seconds,
        audio_length_seconds: details.audio_length_seconds,
        query_count: details.query_count,
    }
}
