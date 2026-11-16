use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::image_cost::pricing::raw_price;
use betterlite_models::image_cost::usage::ImageUsageDetails;
use betterlite_models::price_lookup::PriceRecord;
use rust_decimal::Decimal;

const PROVIDER: &str = "vertex_ai";

// The shared image pricing operations of the response cost service, injected so this crate never imports core.
pub trait ImagePricing {
    // resolve_image_model_info with the caller's table, or the catalog lookup alone when none is supplied
    fn model_info(
        &self,
        model: &str,
        provider: &str,
        supplied: Option<&PriceRecord>,
    ) -> Result<PriceRecord, ImageCostError>;

    // calculate_image_response_web_search_cost for the usage's grounding count, zero when it has none
    fn web_search_cost(
        &self,
        provider: &str,
        web_search_requests: Option<i64>,
        model_info: &PriceRecord,
    ) -> Decimal;

    // calculate_image_response_cost_from_usage: None tells the caller to bill per image instead
    fn usage_cost(
        &self,
        model: &str,
        provider: &str,
        usage: &ImageUsageDetails,
        model_info: &PriceRecord,
    ) -> Result<Option<Decimal>, ImageCostError>;
}

#[derive(Clone, Copy, Debug)]
pub struct BilledUsage<'response> {
    pub tokens: &'response ImageUsageDetails,
    // ImageUsage allows extra fields, and the search count rides in one of them rather than in the token details
    pub web_search_requests: Option<i64>,
}

// What the generation cost reads of an ImageResponse
#[derive(Clone, Copy, Debug)]
pub struct GeneratedImages<'response> {
    pub image_count: usize,
    pub usage: Option<BilledUsage<'response>>,
}

// The edit cost only checks that the response is an ImageResponse, then counts its data
#[derive(Clone, Copy, Debug)]
pub enum EditResponse {
    Image { image_count: usize },
    Other,
}

// output_cost_per_image times the returned images; an absent or null price is zero
fn per_image_total(
    model_info: &PriceRecord,
    image_count: usize,
) -> Result<Decimal, ImageCostError> {
    let count = i64::try_from(image_count).map_err(|_error| ImageCostError::CounterOverflow)?;
    let per_image = raw_price(model_info, "output_cost_per_image")?.unwrap_or(Decimal::ZERO);
    per_image
        .checked_mul(Decimal::from(count))
        .ok_or(ImageCostError::Overflow)
}

// Token usage wins over the per-image price, even a zero token price, and search is added on top.
pub fn generation_cost(
    pricing: &dyn ImagePricing,
    model: &str,
    supplied: Option<&PriceRecord>,
    response: &GeneratedImages<'_>,
) -> Result<Decimal, ImageCostError> {
    let model_info = pricing.model_info(model, PROVIDER, supplied)?;
    let search = pricing.web_search_cost(
        PROVIDER,
        response.usage.and_then(|usage| usage.web_search_requests),
        &model_info,
    );
    let token_cost = match response.usage {
        Some(usage) => pricing.usage_cost(model, PROVIDER, usage.tokens, &model_info)?,
        None => None,
    };
    let base = match token_cost {
        Some(cost) => cost,
        None => per_image_total(&model_info, response.image_count)?,
    };
    base.checked_add(search).ok_or(ImageCostError::Overflow)
}

// Editing resolves the catalog entry first, then charges the returned images only: no tokens, no search.
pub fn edit_cost(
    pricing: &dyn ImagePricing,
    model: &str,
    response: EditResponse,
) -> Result<Decimal, ImageCostError> {
    let model_info = pricing.model_info(model, PROVIDER, None)?;
    match response {
        EditResponse::Image { image_count } => per_image_total(&model_info, image_count),
        EditResponse::Other => Err(ImageCostError::NotImageResponse),
    }
}
