use std::error::Error;

use betterlite_logging::langfuse_assembly::usage::cost_value;
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use betterlite_openai::api::image_contracts::records::{ImageHttpResponse, ImageResponse};
use rust_decimal::Decimal;

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

// a missing price is zero, an explicit null suppresses the header, and anything else is read as a Decimal
fn cost_header(value: Option<&Json>) -> Result<Option<Json>, CostValueError> {
    let price = match value {
        None => Decimal::ZERO,
        Some(Json::Null) => return Ok(None),
        Some(Json::Bool(flag)) => Decimal::from(u8::from(*flag)),
        Some(Json::Number(number)) => parse_decimal_lexeme(&number.to_string())
            .map_err(|source| CostValueError::Parse { source })?,
        Some(Json::Text(text)) => {
            parse_decimal_lexeme(text.trim()).map_err(|source| CostValueError::Parse { source })?
        }
        Some(Json::List(_) | Json::Object(_)) => return Err(CostValueError::NotNumber),
    };
    Ok(Some(cost_value(price)))
}
