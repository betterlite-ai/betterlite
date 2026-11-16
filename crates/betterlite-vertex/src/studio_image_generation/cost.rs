use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::image_cost::pricing::flat_image_cost;
use betterlite_models::image_cost::usage::{DetailMap, ImageUsageDetails};
use betterlite_models::price_lookup::PriceRecord;
use betterlite_openai::api::image_contracts::records::ImageInputTokens;
use rust_decimal::Decimal;
use serde_json::value::RawValue;

use super::error::StudioImageError;
use super::response::ImageGenerationResponse;
use super::usage::ImageGenerationUsage;
use crate::vertex_image_policy::cost::ImagePricing;

const PROVIDER: &str = "gemini";

// The response the Studio cost prices: LiteLLM resolves the model first, then rejects anything that is not an ImageResponse.
pub enum ImageCostResponse<'response> {
    Image(&'response ImageGenerationResponse),
    Other,
}

// cost_calculator: the model lookup comes first, then the response type, then token cost, per-image cost and search.
pub fn generation_cost(
    pricing: &dyn ImagePricing,
    model: &str,
    supplied: Option<&PriceRecord>,
    response: ImageCostResponse<'_>,
) -> Result<Decimal, StudioImageError> {
    let model_info = pricing.model_info(model, PROVIDER, supplied)?;
    let ImageCostResponse::Image(response) = response else {
        return Err(StudioImageError::from(ImageCostError::NotImageResponse));
    };
    let search = pricing.web_search_cost(
        PROVIDER,
        response
            .usage
            .as_ref()
            .and_then(|usage| usage.web_search_requests),
        &model_info,
    );
    let base = billed_total(pricing, model, &model_info, response)?;
    base.checked_add(search)
        .ok_or_else(|| StudioImageError::from(ImageCostError::Overflow))
}

// Token usage wins over the per-image price, and a missing usage or an unpriced token table bills the images.
fn billed_total(
    pricing: &dyn ImagePricing,
    model: &str,
    model_info: &PriceRecord,
    response: &ImageGenerationResponse,
) -> Result<Decimal, StudioImageError> {
    let token_cost = match &response.usage {
        Some(usage) => pricing.usage_cost(model, PROVIDER, &usage_details(usage)?, model_info)?,
        None => None,
    };
    if let Some(cost) = token_cost {
        return Ok(cost);
    }
    let count =
        i64::try_from(response.data.len()).map_err(|_error| ImageCostError::CounterOverflow)?;
    Ok(flat_image_cost(Some(model_info), count)?)
}

// The token details the shared usage cost reads, one itemized map per side of the usage.
fn usage_details(usage: &ImageGenerationUsage) -> Result<ImageUsageDetails, StudioImageError> {
    Ok(ImageUsageDetails {
        input_tokens: Some(usage.input_tokens),
        output_tokens: Some(usage.output_tokens),
        total_tokens: Some(usage.total_tokens),
        input_tokens_details: Some(token_details(&usage.input_tokens_details)?),
        completion_tokens_details: Some(token_details(&usage.completion_tokens_details)?),
        output_tokens_details: Some(token_details(&usage.output_tokens_details)?),
    })
}

fn token_details(tokens: &ImageInputTokens) -> Result<DetailMap, StudioImageError> {
    let mut details = DetailMap::new();
    for (name, count) in [
        ("text_tokens", tokens.text_tokens),
        ("image_tokens", tokens.image_tokens),
    ] {
        let raw = RawValue::from_string(count.to_string()).map_err(StudioImageError::Encode)?;
        details.insert(Box::<str>::from(name), raw);
    }
    Ok(details)
}
