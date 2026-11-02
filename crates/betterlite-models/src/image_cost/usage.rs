use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Number;
use serde_json::value::RawValue;

use super::error::ImageCostError;
use super::pricing::prices_tokens;
use crate::cost_usage::{CompletionDetails, CostUsage, PromptDetails};
use crate::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use crate::token_cost::prompt_overlap::PromptExtras;
use crate::token_cost::{GenericCostRequest, generic_cost_per_token};

pub type DetailMap = BTreeMap<Box<str>, Box<RawValue>>;

// The output side of a real ImageUsage object: input/output totals plus whichever detail breakdown (dict or attribute-bearing object, already normalized to a JSON object) the provider itemized.
#[derive(Debug, Clone, Default)]
pub struct ImageUsageDetails {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub input_tokens_details: Option<DetailMap>,
    pub completion_tokens_details: Option<DetailMap>,
    pub output_tokens_details: Option<DetailMap>,
}

pub struct ImageUsageCostRequest<'request> {
    pub model: &'request str,
    pub custom_llm_provider: Option<&'request str>,
    pub usage: &'request ImageUsageDetails,
    pub model_info: Option<&'request PriceRecord>,
    pub moment: DateTime<Utc>,
}

// Returns the summed cost from token usage, or None when usage metadata is missing, incomplete, or an all-zero placeholder, telling the caller to fall back to flat pricing.
pub fn image_usage_cost(
    request: &ImageUsageCostRequest<'_>,
    lookup: &dyn PriceLookup,
    database: &TimeZoneDatabase,
) -> Result<Option<Decimal>, ImageCostError> {
    let usage = request.usage;
    let (Some(input_tokens), Some(output_tokens), Some(total_tokens)) =
        (usage.input_tokens, usage.output_tokens, usage.total_tokens)
    else {
        return Ok(None);
    };
    if input_tokens == 0 && output_tokens == 0 && total_tokens == 0 {
        return Ok(None);
    }
    if let Some(model_info) = request.model_info
        && !prices_tokens(model_info)
    {
        return Ok(None);
    }

    let normalized = normalized_cost_usage(usage, input_tokens, output_tokens, total_tokens)?;
    let cost = chat_usage_cost(
        &ChatUsageCostRequest {
            model: request.model,
            custom_llm_provider: request.custom_llm_provider,
            usage: &normalized,
            model_info: request.model_info,
            moment: request.moment,
        },
        lookup,
        database,
    )?;
    Ok(Some(cost))
}

fn normalized_cost_usage(
    usage: &ImageUsageDetails,
    input_tokens: i64,
    output_tokens: i64,
    total_tokens: i64,
) -> Result<CostUsage, ImageCostError> {
    Ok(CostUsage {
        prompt_tokens: input_tokens,
        completion_tokens: output_tokens,
        total_tokens,
        prompt_details: prompt_details_from(usage.input_tokens_details.as_ref())?,
        completion_details: Some(completion_details_from(usage, output_tokens)?),
        cache_read_tokens: Some(0),
        numeric_extras: BTreeMap::new(),
    })
}

pub struct ChatUsageCostRequest<'request> {
    pub model: &'request str,
    pub custom_llm_provider: Option<&'request str>,
    pub usage: &'request CostUsage,
    pub model_info: Option<&'request PriceRecord>,
    pub moment: DateTime<Utc>,
}

// A genuine chat Usage riding on an image response: priced exactly like any other generic completion, with its own token-detail breakdown standing as given.
pub(super) fn chat_usage_cost(
    request: &ChatUsageCostRequest<'_>,
    lookup: &dyn PriceLookup,
    database: &TimeZoneDatabase,
) -> Result<Decimal, ImageCostError> {
    // A flat image model_info table carries no tiered or off-peak schedule of its own.
    let supplied_pricing = request.model_info.cloned().map(|record| PricedModel {
        record,
        tiered_pricing: Vec::new(),
        off_peak_pricing: None,
        off_peak_cache_read_present: false,
    });
    let extras = PromptExtras::default();
    let result = generic_cost_per_token(
        &GenericCostRequest {
            model: request.model,
            custom_llm_provider: request.custom_llm_provider,
            usage: request.usage,
            prompt_extras: &extras,
            supplied_pricing: supplied_pricing.as_ref(),
            service_tier: None,
            data_residency: None,
            vertex_location: None,
            moment: request.moment,
        },
        lookup,
        database,
    )?;
    result
        .prompt_cost
        .checked_add(result.completion_cost)
        .ok_or(ImageCostError::Overflow)
}

fn prompt_details_from(
    input_tokens_details: Option<&DetailMap>,
) -> Result<Option<PromptDetails>, ImageCostError> {
    let Some(details) = input_tokens_details else {
        return Ok(None);
    };
    Ok(Some(PromptDetails {
        text_tokens: token_detail_value(details, "text_tokens")?,
        audio_tokens: None,
        image_tokens: token_detail_value(details, "image_tokens")?,
        video_tokens: None,
        web_search_requests: None,
    }))
}

// With neither completion_tokens_details nor output_tokens_details, every output token is an image token. With a record (completion_tokens_details preferred, even when empty), the itemized text/image/audio/reasoning counts stand, and any positive unreported remainder (output_tokens beyond their sum) is assigned to text.
fn completion_details_from(
    usage: &ImageUsageDetails,
    output_tokens: i64,
) -> Result<CompletionDetails, ImageCostError> {
    let Some(details) = usage
        .completion_tokens_details
        .as_ref()
        .or(usage.output_tokens_details.as_ref())
    else {
        return Ok(CompletionDetails {
            text_tokens: Some(0),
            image_tokens: Some(output_tokens),
            audio_tokens: Some(0),
            video_tokens: None,
            reasoning_tokens: Some(0),
        });
    };

    let text = token_detail_value(details, "text_tokens")?.unwrap_or(0);
    let image = token_detail_value(details, "image_tokens")?.unwrap_or(0);
    let audio = token_detail_value(details, "audio_tokens")?.unwrap_or(0);
    let reasoning = token_detail_value(details, "reasoning_tokens")?.unwrap_or(0);
    let overflow = || ImageCostError::CounterOverflow;
    let known = text
        .checked_add(image)
        .and_then(|value| value.checked_add(audio))
        .and_then(|value| value.checked_add(reasoning))
        .ok_or_else(overflow)?;
    let remainder = output_tokens.checked_sub(known).ok_or_else(overflow)?;
    let text_total = if remainder > 0 {
        text.checked_add(remainder).ok_or_else(overflow)?
    } else {
        text
    };
    Ok(CompletionDetails {
        text_tokens: Some(text_total),
        image_tokens: Some(image),
        audio_tokens: Some(audio),
        video_tokens: None,
        reasoning_tokens: Some(reasoning),
    })
}

// Shared with azure's exact-pixel-dimension check: a bool or an int-shaped JSON number counts, a float-shaped number, a string, an array, an object or a missing key does not.
#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum TokenDetailCandidate {
    Bool(bool),
    Number(Box<Number>),
    Other(Box<IgnoredAny>),
}

fn token_detail_value(details: &DetailMap, key: &str) -> Result<Option<i64>, ImageCostError> {
    let Some(raw) = details.get(key) else {
        return Ok(None);
    };
    let candidate = serde_json::from_str::<TokenDetailCandidate>(raw.get())
        .unwrap_or_else(|_error| TokenDetailCandidate::Other(Box::new(IgnoredAny)));
    match candidate {
        TokenDetailCandidate::Bool(flag) => Ok(Some(i64::from(flag))),
        TokenDetailCandidate::Number(number) => {
            if number.to_string().contains(['.', 'e', 'E']) {
                return Ok(None);
            }
            number
                .as_i64()
                .map(Some)
                .ok_or(ImageCostError::CounterOverflow)
        }
        TokenDetailCandidate::Other(_ignored) => Ok(None),
    }
}

mod tests;
