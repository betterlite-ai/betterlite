use betterlite_logging::langfuse_assembly::usage::cost_value;
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::tiered_pricing::Scalar;
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageHttpResponse, ImageResponse,
};
use rust_decimal::Decimal;

use super::environment::get_error_class;
use super::error::{StabilityEditError, StabilityReplyError};
use super::types::text_of;

pub const COST_HEADER: &str = "llm_provider-x-litellm-response-cost";

const OUTPUT_COST_PER_IMAGE: &str = "output_cost_per_image";

// B2225 get_model_info hands over the model's metadata record; an unmapped model has none
pub trait ModelPriceLookup {
    fn price_record(&self, model: &str) -> Option<PriceRecord>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StabilityArtifacts {
    pub errors: Option<Json>,
    pub finish_reason: Option<Json>,
    pub images: Vec<String>,
}

#[derive(Debug)]
pub struct ImageEditReply {
    pub response: ImageResponse,
    pub additional_headers: AdditionalHeaders,
}

// The Stability API answers artifacts under images; only a truthy string entry becomes an ImageObject.
pub fn decode_stability_reply(body: &[u8]) -> Result<StabilityArtifacts, StabilityReplyError> {
    let value: Json =
        serde_json::from_slice(body).map_err(|source| StabilityReplyError::Json { source })?;
    let Json::Object(mut fields) = value else {
        return Err(StabilityReplyError::NotAnObject);
    };
    let errors = fields.shift_remove("errors");
    let finish_reason = first_finish_reason(fields.shift_remove("finish_reasons"))?;
    let images = image_values(fields.shift_remove("images"))?;
    Ok(StabilityArtifacts {
        errors,
        finish_reason,
        images,
    })
}

// LiteLLM raises only for a truthy finish_reasons[0]; a string container answers its first character, as Python indexing does.
fn first_finish_reason(value: Option<Json>) -> Result<Option<Json>, StabilityReplyError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::List(reasons)) => Ok(reasons.into_iter().next().filter(has_content)),
        Some(Json::Text(text)) => Ok(text
            .chars()
            .next()
            .map(|character| Json::Text(character.to_string()))),
        Some(reason @ (Json::Bool(_) | Json::Number(_))) => {
            if has_content(&reason) {
                Err(StabilityReplyError::Field {
                    field: "finish_reasons".to_owned(),
                })
            } else {
                Ok(None)
            }
        }
        Some(Json::Object(reasons)) => {
            if reasons.is_empty() {
                Ok(None)
            } else {
                Err(StabilityReplyError::Field {
                    field: "finish_reasons".to_owned(),
                })
            }
        }
    }
}

fn image_values(value: Option<Json>) -> Result<Vec<String>, StabilityReplyError> {
    let entries = match value {
        None | Some(Json::Null) => return Ok(Vec::new()),
        Some(Json::List(entries)) => entries,
        Some(_) => {
            return Err(StabilityReplyError::Field {
                field: "images".to_owned(),
            });
        }
    };
    let mut images = Vec::new();
    for entry in entries {
        if !has_content(&entry) {
            continue;
        }
        let Json::Text(image) = entry else {
            return Err(StabilityReplyError::Field {
                field: "images".to_owned(),
            });
        };
        images.push(image);
    }
    Ok(images)
}

fn has_content(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number.as_f64().is_some_and(|float| float != 0.0_f64),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}

pub fn transform_image_edit_response<Lookup>(
    model: &str,
    upstream: &ImageHttpResponse,
    now_seconds: i64,
    lookup: &Lookup,
) -> Result<ImageEditReply, StabilityEditError>
where
    Lookup: ModelPriceLookup,
{
    let artifacts =
        decode_stability_reply(&upstream.body).map_err(|source| reply_error(upstream, source))?;
    reject_artifact_errors(upstream, &artifacts)?;
    let data = artifacts
        .images
        .into_iter()
        .map(|b64_json| ImageData {
            b64_json: Some(b64_json),
            revised_prompt: None,
            url: None,
        })
        .collect();
    let record = lookup
        .price_record(model)
        .ok_or_else(|| StabilityEditError::ModelInfo {
            model: model.to_owned(),
        })?;
    let mut additional_headers = AdditionalHeaders::new();
    if let Some(price) = cost_header(&record)? {
        additional_headers.insert(COST_HEADER.to_owned(), price);
    }
    Ok(ImageEditReply {
        response: ImageResponse {
            created: now_seconds,
            data: Some(data),
            usage: None,
        },
        additional_headers,
    })
}

// An errors key fails whatever the images hold; only the first finish reason is inspected, and it reports status 400.
fn reject_artifact_errors(
    upstream: &ImageHttpResponse,
    artifacts: &StabilityArtifacts,
) -> Result<(), StabilityEditError> {
    if let Some(errors) = &artifacts.errors {
        return Err(get_error_class(
            format!("Bedrock Stability error: {}", text_of(errors)),
            upstream.status_code,
            upstream_headers(upstream),
        ));
    }
    if let Some(reason) = &artifacts.finish_reason
        && has_content(reason)
    {
        return Err(get_error_class(
            format!("Bedrock Stability error: {}", text_of(reason)),
            400,
            upstream_headers(upstream),
        ));
    }
    Ok(())
}

// Only a JSON parse failure becomes BedrockError; a parsed record of the wrong shape keeps its Python error type.
fn reply_error(upstream: &ImageHttpResponse, source: StabilityReplyError) -> StabilityEditError {
    match source {
        StabilityReplyError::Json { .. } => get_error_class(
            format!("Error parsing Bedrock Stability response: {source}"),
            upstream.status_code,
            upstream_headers(upstream),
        ),
        other @ (StabilityReplyError::NotAnObject | StabilityReplyError::Field { .. }) => {
            StabilityEditError::Reply(other)
        }
    }
}

// A missing price is zero, an explicit null suppresses the header, and anything else is read as a Decimal, as float() would.
fn cost_header(record: &PriceRecord) -> Result<Option<Json>, StabilityEditError> {
    let price = match record.get(OUTPUT_COST_PER_IMAGE) {
        None => Decimal::ZERO,
        Some(Scalar::Null) => return Ok(None),
        Some(Scalar::Bool(flag)) => Decimal::from(u8::from(*flag)),
        Some(Scalar::Number(number)) => {
            parse_decimal_lexeme(&number.to_string()).map_err(ImageCostError::Rate)?
        }
        Some(Scalar::Text(text)) => {
            parse_decimal_lexeme(text.trim()).map_err(ImageCostError::Rate)?
        }
        Some(Scalar::Array(_) | Scalar::Object(_)) => {
            return Err(StabilityEditError::from(ImageCostError::WrongFieldType {
                field: OUTPUT_COST_PER_IMAGE.to_owned(),
            }));
        }
    };
    Ok(Some(cost_value(price)))
}

fn upstream_headers(upstream: &ImageHttpResponse) -> Vec<(String, String)> {
    upstream
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}
