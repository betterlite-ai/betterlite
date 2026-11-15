use std::error::Error;

use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::api::image_contracts::records::{ImageHttpResponse, ImageResponse};
use serde_json::Number;

use super::error::{CostValueError, StabilityEditError};
use crate::api::stability::generation::reply::decode_stability_image;

pub const COST_HEADER: &str = "llm_provider-x-litellm-response-cost";

const OUTPUT_COST_PER_IMAGE: &str = "output_cost_per_image";

// B2225 get_model_info hands over the model's metadata record; a failed lookup keeps its own error type
pub trait ModelInfoLookup {
    type Error: Error + 'static;

    fn model_info(&self, model: &str) -> Result<JsonObject, Self::Error>;
}

#[derive(Debug)]
pub struct ImageEditReply {
    pub response: ImageResponse,
    pub additional_headers: AdditionalHeaders,
}

// LiteLLM's transform_image_edit_response: a fresh response, then the catalog price as a hidden header
pub fn transform_image_edit_response<Lookup>(
    model: &str,
    upstream: &ImageHttpResponse,
    now_seconds: i64,
    lookup: &Lookup,
) -> Result<ImageEditReply, StabilityEditError<Lookup::Error>>
where
    Lookup: ModelInfoLookup,
{
    let mut response = ImageResponse {
        created: now_seconds,
        data: None,
        usage: None,
    };
    decode_stability_image(upstream, &mut response)?;
    let info = lookup
        .model_info(model)
        .map_err(StabilityEditError::ModelInfo)?;
    let mut additional_headers = AdditionalHeaders::new();
    if let Some(value) = cost_header(info.get(OUTPUT_COST_PER_IMAGE))? {
        additional_headers.insert(COST_HEADER.to_owned(), value);
    }
    Ok(ImageEditReply {
        response,
        additional_headers,
    })
}

// a missing price is zero, an explicit null suppresses the header, and anything else is converted
fn cost_header(value: Option<&Json>) -> Result<Option<Json>, CostValueError> {
    match value {
        None => Ok(Some(cost_number(0.0)?)),
        Some(Json::Null) => Ok(None),
        Some(value) => Ok(Some(cost_number(cost_float(value)?)?)),
    }
}

fn cost_number(value: f64) -> Result<Json, CostValueError> {
    match Number::from_f64(value) {
        Some(number) => Ok(Json::Number(number)),
        None => Err(CostValueError),
    }
}

// Python's float(): a bool counts as 0 or 1, a number keeps its value, and text is trimmed then parsed
fn cost_float(value: &Json) -> Result<f64, CostValueError> {
    match value {
        Json::Bool(flag) => Ok(if *flag { 1.0 } else { 0.0 }),
        Json::Number(number) => number.as_f64().ok_or(CostValueError),
        Json::Text(text) => text.trim().parse::<f64>().map_err(|_source| CostValueError),
        Json::Null | Json::List(_) | Json::Object(_) => Err(CostValueError),
    }
}
