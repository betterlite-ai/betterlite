use betterlite_models::decisions::Json;
use betterlite_models::image_cost::default_calculator::{
    DefaultImageCostRequest, default_image_cost_calculator,
};
use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::price_lookup::PriceLookup;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

mod tests;

const OPENAI_PROVIDER: &str = "openai";
const DEFAULT_SIZE: &str = "1024x1024";

pub struct ImageCostRequest<'request> {
    pub model: &'request str,
    pub request_body: &'request IndexMap<String, Json>,
}

#[derive(Debug, thiserror::Error)]
enum ImageCostFailure {
    #[error("the request field {field} is not a string")]
    NotText { field: &'static str },
    #[error("the image cost could not be calculated: {0}")]
    Calculator(#[from] ImageCostError),
}

#[must_use]
pub fn image_generation_cost(request: &ImageCostRequest<'_>, lookup: &dyn PriceLookup) -> Decimal {
    let quality = request.request_body.get("quality");
    match calculated_cost(request, quality, lookup) {
        Ok(cost) => cost,
        Err(failure) => warn_and_zero(request.model, &failure, "image generation"),
    }
}

#[must_use]
pub fn image_editing_cost(request: &ImageCostRequest<'_>, lookup: &dyn PriceLookup) -> Decimal {
    // The editing request carries no quality parameter, so the source passes none to the calculator
    match calculated_cost(request, None, lookup) {
        Ok(cost) => cost,
        Err(failure) => warn_and_zero(request.model, &failure, "image editing"),
    }
}

fn warn_and_zero(model: &str, failure: &ImageCostFailure, endpoint: &str) -> Decimal {
    tracing::warn!(model, error = %failure, "Error calculating {endpoint} cost");
    Decimal::ZERO
}

fn calculated_cost(
    request: &ImageCostRequest<'_>,
    quality: Option<&Json>,
    lookup: &dyn PriceLookup,
) -> Result<Decimal, ImageCostFailure> {
    // The body reads int() of a number as truncated toward zero, of a bool as 0 or 1, and of anything unparsable as the source's fallback of 1
    let image_count = match request.request_body.get("n") {
        Some(Json::Number(number)) => number
            .as_i64()
            .or_else(|| {
                let float = number.as_f64()?;
                float.trunc().to_i64()
            })
            .unwrap_or(1),
        Some(Json::Bool(false)) => 0,
        Some(Json::Text(text)) => text.trim().parse::<i64>().unwrap_or(1),
        None | Some(Json::Bool(true) | Json::Null | Json::List(_) | Json::Object(_)) => 1,
    };
    // The calculator treats an empty or absent quality as no quality, and any other non-string is a type mismatch the source would only fail on later
    let quality_text = match quality {
        None | Some(Json::Null) => None,
        Some(Json::Text(text)) if text.is_empty() => None,
        Some(Json::Text(text)) => Some(text.as_str()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(ImageCostFailure::NotText { field: "quality" });
        }
    };
    // The source's body lookup keeps any non-string size, and the calculator then raises, so a number is a failed cost rather than the default size
    let raw_size = match request.request_body.get("size") {
        None | Some(Json::Null) => DEFAULT_SIZE,
        Some(Json::Text(text)) => text.as_str(),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(ImageCostFailure::NotText { field: "size" });
        }
    };
    // LiteLLM rewrites an x separator to "-x-" before building catalog keys; the models calculator rewrites it only for the dimension parse, so the key lookup needs the rewritten spelling here
    let price_size = if raw_size.contains("-x-") {
        raw_size.to_owned()
    } else {
        raw_size.replace('x', "-x-")
    };
    let calculation = DefaultImageCostRequest {
        model: request.model,
        custom_llm_provider: Some(OPENAI_PROVIDER),
        quality: quality_text,
        n_images: Some(image_count),
        size: Some(price_size.as_str()),
        model_info: None,
    };
    Ok(default_image_cost_calculator(&calculation, lookup)?)
}
