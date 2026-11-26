use betterlite_logging::langfuse_assembly::usage::cost_value;
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::tiered_pricing::Scalar;
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageHttpResponse, ImageResponse, ImageUsage,
};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Error as JsonError;
use thiserror::Error;

use crate::canvas_edits::environment::get_error_class;
use crate::canvas_edits::error::CanvasEditError;
use crate::stability_edits::types::text_of;

pub const COST_HEADER: &str = "llm_provider-x-litellm-response-cost";

const OUTPUT_COST_PER_IMAGE: &str = "output_cost_per_image";
const ERROR_PREFIX: &str = "Nova Canvas image edit error";
const PARSE_PREFIX: &str = "Error parsing Nova Canvas image edit response";
const NO_IMAGES: &str = "Nova Canvas image edit returned no images";
const IMAGE_FIELD: &str = "images";

// B068 get_model_info hands over the model's bedrock metadata record; an unmapped model has none
pub trait ModelPriceLookup {
    fn price_record(&self, model: &str) -> Option<PriceRecord>;
}

// The keys a Nova Canvas reply body declares; other keys carry nothing the transform reads.
#[derive(Debug, Deserialize)]
pub struct CanvasReplyFields {
    pub images: Option<Json>,
    pub errors: Option<Json>,
    pub message: Option<Json>,
    pub error: Option<Json>,
}

// A parsed body: the object shape the transform reads, or a value of another shape kept as the text a non-200 message prints.
#[derive(Debug)]
pub enum CanvasReplyBody {
    Object(Box<CanvasReplyFields>),
    NotObject { text: String },
}

#[derive(Debug, Error)]
pub enum CanvasReplyError {
    #[error("the response body is not JSON: {source}")]
    Json {
        #[source]
        source: JsonError,
    },
    #[error("the response body is not a JSON object")]
    NotAnObject,
    #[error("the {field} value has a type the Nova Canvas reply does not declare")]
    Field { field: String },
}

#[derive(Debug, Error)]
pub enum CanvasResultError {
    #[error(transparent)]
    Edit(#[from] CanvasEditError),
    #[error(transparent)]
    Reply(#[from] CanvasReplyError),
    // LiteLLM reports a body that is not JSON as its provider error; the decoder failure stays in the chain.
    #[error("{message}")]
    Parse {
        message: String,
        #[source]
        source: CanvasReplyError,
        provider: CanvasEditError,
    },
}

impl CanvasResultError {
    // The status LiteLLM's BedrockError carries; None is what a Python AttributeError or TypeError reports
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Edit(error) => error.status(),
            Self::Parse { provider, .. } => provider.status(),
            Self::Reply(_) => None,
        }
    }
}

#[derive(Debug)]
pub struct CanvasEditReply {
    pub response: ImageResponse,
    pub additional_headers: AdditionalHeaders,
}

// Only raw JSON decoding fails here; a parsed body of another shape keeps its Python error type.
pub fn decode_canvas_reply(body: &[u8]) -> Result<CanvasReplyBody, CanvasReplyError> {
    if let Ok(fields) = serde_json::from_slice::<CanvasReplyFields>(body) {
        return Ok(CanvasReplyBody::Object(Box::new(fields)));
    }
    match serde_json::from_slice(body) {
        Ok(value) => Ok(CanvasReplyBody::NotObject {
            text: text_of(&value),
        }),
        Err(source) => Err(CanvasReplyError::Json { source }),
    }
}

pub fn transform_image_edit_response<Lookup>(
    model: &str,
    upstream: &ImageHttpResponse,
    now_seconds: i64,
    lookup: &Lookup,
) -> Result<CanvasEditReply, CanvasResultError>
where
    Lookup: ModelPriceLookup,
{
    let body = reply_or_error(upstream)?;
    status_or_error(upstream, &body)?;
    let CanvasReplyBody::Object(mut fields) = body else {
        return Err(CanvasResultError::Reply(CanvasReplyError::NotAnObject));
    };
    let entries = image_entries(fields.images.take())?;
    reject_error_fields(upstream, &fields, entries.is_empty())?;
    let data = image_data(entries)?;
    if data.is_empty() {
        return Err(provider_reply_error(upstream, NO_IMAGES.to_owned()));
    }
    let mut additional_headers = AdditionalHeaders::new();
    if let Some(cost) = cost_header(model, data.len(), lookup) {
        additional_headers.insert(COST_HEADER.to_owned(), cost);
    }
    Ok(CanvasEditReply {
        response: ImageResponse {
            created: now_seconds,
            data: Some(data),
            usage: Some(ImageUsage::default()),
        },
        additional_headers,
    })
}

fn reply_or_error(upstream: &ImageHttpResponse) -> Result<CanvasReplyBody, CanvasResultError> {
    match decode_canvas_reply(&upstream.body) {
        Ok(body) => Ok(body),
        Err(source) => Err(parse_reply_error(upstream, source)),
    }
}

fn parse_reply_error(upstream: &ImageHttpResponse, source: CanvasReplyError) -> CanvasResultError {
    let message = format!("{PARSE_PREFIX}: {source}");
    CanvasResultError::Parse {
        provider: get_error_class(
            message.clone(),
            upstream.status_code,
            upstream_headers(upstream),
        ),
        message,
        source,
    }
}

fn status_or_error(
    upstream: &ImageHttpResponse,
    body: &CanvasReplyBody,
) -> Result<(), CanvasResultError> {
    if upstream.status_code == 200 {
        return Ok(());
    }
    let text = match body {
        CanvasReplyBody::Object(_) => "object".to_owned(),
        CanvasReplyBody::NotObject { text } => text.clone(),
    };
    Err(provider_reply_error(
        upstream,
        format!("{ERROR_PREFIX}: {text}"),
    ))
}

fn provider_reply_error(upstream: &ImageHttpResponse, message: String) -> CanvasResultError {
    CanvasResultError::Edit(get_error_class(
        message,
        upstream.status_code,
        upstream_headers(upstream),
    ))
}

// get("images") or []: an absent or null container is empty, and a container of another type names the field.
fn image_entries(value: Option<Json>) -> Result<Vec<Json>, CanvasReplyError> {
    match value {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::List(entries)) => Ok(entries),
        Some(_) => Err(CanvasReplyError::Field {
            field: IMAGE_FIELD.to_owned(),
        }),
    }
}

// LiteLLM skips a falsey entry of any type; a truthy entry becomes an ImageObject, which only a string satisfies.
fn image_data(entries: Vec<Json>) -> Result<Vec<ImageData>, CanvasReplyError> {
    let mut data = Vec::new();
    for entry in entries {
        if !has_content(&entry) {
            continue;
        }
        let Json::Text(b64_json) = entry else {
            return Err(CanvasReplyError::Field {
                field: IMAGE_FIELD.to_owned(),
            });
        };
        data.push(ImageData {
            b64_json: Some(b64_json),
            revised_prompt: None,
            url: None,
        });
    }
    Ok(data)
}

// A truthy images container suppresses both error branches, even when every entry is skipped later.
fn reject_error_fields(
    upstream: &ImageHttpResponse,
    fields: &CanvasReplyFields,
    images_empty: bool,
) -> Result<(), CanvasResultError> {
    if !images_empty {
        return Ok(());
    }
    if let Some(errors) = &fields.errors {
        return Err(provider_reply_error(
            upstream,
            format!("{ERROR_PREFIX}: {}", text_of(errors)),
        ));
    }
    let chosen = fields
        .message
        .as_ref()
        .filter(|value| has_content(value))
        .or(fields.error.as_ref());
    if let Some(value) = chosen
        && has_content(value)
    {
        return Err(provider_reply_error(
            upstream,
            format!("{ERROR_PREFIX}: {}", text_of(value)),
        ));
    }
    Ok(())
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

// A missing price is zero, an explicit null suppresses the header, and anything else is read as a Decimal, as float() would.
fn cost_header<Lookup>(model: &str, image_count: usize, lookup: &Lookup) -> Option<Json>
where
    Lookup: ModelPriceLookup,
{
    let record = lookup.price_record(model)?;
    let price = match record.get(OUTPUT_COST_PER_IMAGE) {
        None => Decimal::ZERO,
        Some(Scalar::Null | Scalar::Array(_) | Scalar::Object(_)) => return None,
        Some(Scalar::Bool(flag)) => Decimal::from(u8::from(*flag)),
        Some(Scalar::Number(number)) => parse_decimal_lexeme(&number.to_string()).ok()?,
        Some(Scalar::Text(text)) => parse_decimal_lexeme(text.trim()).ok()?,
    };
    let count = Decimal::from(u64::try_from(image_count).ok()?);
    Some(cost_value(price.checked_mul(count)?))
}

fn upstream_headers(upstream: &ImageHttpResponse) -> Vec<(String, String)> {
    upstream
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

mod tests;
