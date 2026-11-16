use betterlite_models::decisions::Extra;
use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::image_cost::usage::{DetailMap, ImageUsageDetails};
use betterlite_models::price_lookup::PriceRecord;
use betterlite_openai::api::image_contracts::records::ImageData;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_vertex::vertex_image_policy::config::{
    EditConfig, GenerationConfig, edit_config, generation_config,
};
use betterlite_vertex::vertex_image_policy::cost::{
    BilledUsage, EditResponse, GeneratedImages, edit_cost, generation_cost,
};
use betterlite_vertex::vertex_image_policy::error::PredictionError;
use betterlite_vertex::vertex_image_policy::prediction::{
    is_image_generation_response,
    // ast-grep-ignore: no-long-names
    process_image_generation_response,
};
use betterlite_vertex::vertex_imagen_generation::response::ImagenGenerationResponse;
use serde::Deserialize;
use serde_json::{Value, json};

use super::pricing::CatalogPricing;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) models: Vec<String>,
    pub(super) billing: Vec<Billing>,
    pub(super) replies: Vec<Reply>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum BillingKind {
    Generation,
    Edit,
}

// one cost call: the catalog entry the lookup answers with, and the ImageResponse it prices
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Billing {
    kind: BillingKind,
    model: String,
    catalog: PriceRecord,
    // an edit given null gets something that is not an ImageResponse
    response: Option<Images>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Images {
    images: usize,
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Usage {
    input_tokens: i64,
    output_tokens: i64,
    total_tokens: i64,
    web_search_requests: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ReplyKind {
    Detect,
    Convert,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reply {
    kind: ReplyKind,
    body: Extra,
    // the encoded images the response already held before the conversion
    existing: Vec<Option<String>>,
}

pub(super) fn run_cases(input: &Input) -> HelperResult {
    let mut models = Vec::with_capacity(input.models.len());
    for model in &input.models {
        models.push(json!({
            "model": model,
            "generation": match generation_config(model) {
                GenerationConfig::Gemini => "VertexAIGeminiImageGenerationConfig",
                GenerationConfig::Imagen => "VertexAIImagenImageGenerationConfig",
            },
            "edit": match edit_config(model) {
                EditConfig::Gemini => "VertexAIGeminiImageEditConfig",
                EditConfig::Imagen => "VertexAIImagenImageEditConfig",
            },
        }));
    }
    let mut billing = Vec::with_capacity(input.billing.len());
    for case in &input.billing {
        billing.push(billing_value(case)?);
    }
    let mut replies = Vec::with_capacity(input.replies.len());
    for case in &input.replies {
        replies.push(reply_value(case)?);
    }
    Ok(json!({"models": models, "billing": billing, "replies": replies}))
}

// the usage as the shared image usage cost reads it, with the zero input details ImageUsage always carries
fn usage_details(usage: &Usage) -> Result<ImageUsageDetails, Box<HelperError>> {
    let input_tokens_details: DetailMap =
        serde_json::from_str(r#"{"image_tokens": 0, "text_tokens": 0}"#).or_config_error()?;
    Ok(ImageUsageDetails {
        input_tokens: Some(usage.input_tokens),
        output_tokens: Some(usage.output_tokens),
        total_tokens: Some(usage.total_tokens),
        input_tokens_details: Some(input_tokens_details),
        completion_tokens_details: None,
        output_tokens_details: None,
    })
}

fn billing_value(case: &Billing) -> Result<Value, Box<HelperError>> {
    let pricing = CatalogPricing {
        catalog: &case.catalog,
    };
    let cost = match case.kind {
        BillingKind::Generation => {
            let images = case
                .response
                .as_ref()
                .ok_or("a generation cost needs a response")
                .or_config_error()?;
            let details = images.usage.as_ref().map(usage_details).transpose()?;
            let usage = images
                .usage
                .as_ref()
                .zip(details.as_ref())
                .map(|(fields, tokens)| BilledUsage {
                    tokens,
                    web_search_requests: fields.web_search_requests,
                });
            let generated = GeneratedImages {
                image_count: images.images,
                usage,
            };
            generation_cost(&pricing, &case.model, None, &generated)
        }
        BillingKind::Edit => {
            let response = case
                .response
                .as_ref()
                .map_or(EditResponse::Other, |images| EditResponse::Image {
                    image_count: images.images,
                });
            edit_cost(&pricing, &case.model, response)
        }
    }
    .map_err(|error| cost_failure(&error))?;
    Ok(json!({"cost": cost.normalize().to_string()}))
}

fn cost_failure(error: &ImageCostError) -> HelperError {
    let class = match error {
        ImageCostError::NotImageResponse => "ValueError",
        ImageCostError::UnknownModel { .. } => "Exception",
        ImageCostError::NoPrice { .. }
        | ImageCostError::WrongFieldType { .. }
        | ImageCostError::MalformedSize { .. }
        | ImageCostError::DimensionOverflow
        | ImageCostError::CounterOverflow
        | ImageCostError::Overflow
        | ImageCostError::Rate(_)
        | ImageCostError::Tier(_)
        | ImageCostError::TokenCost(_) => "ImageCostError",
    };
    HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    }
}

fn prediction_failure(error: &PredictionError) -> Box<HelperError> {
    let (class, status) = match error {
        PredictionError::MissingPredictions => ("InternalServerError", Some(500_u16)),
        PredictionError::EmptyPredictions => ("IndexError", None),
        PredictionError::MissingImageKey { .. } => ("KeyError", None),
        PredictionError::Shape { .. } | PredictionError::ImageType { .. } => ("TypeError", None),
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn reply_value(case: &Reply) -> Result<Value, Box<HelperError>> {
    match case.kind {
        ReplyKind::Detect => detect_value(&case.body),
        ReplyKind::Convert => convert_value(case),
    }
}

fn detect_value(body: &Extra) -> Result<Value, Box<HelperError>> {
    let detected =
        is_image_generation_response(body).map_err(|error| prediction_failure(&error))?;
    Ok(json!({"detected": detected}))
}

fn convert_value(case: &Reply) -> Result<Value, Box<HelperError>> {
    let mut response = ImagenGenerationResponse::default();
    for encoded in &case.existing {
        response.data.push(ImageData {
            b64_json: encoded.clone(),
            revised_prompt: None,
            url: None,
        });
    }
    // ast-grep-ignore: no-long-names
    process_image_generation_response(&case.body, &mut response)
        .map_err(|error| prediction_failure(&error))?;
    let mut images: Vec<Option<&str>> = Vec::with_capacity(response.data.len());
    for image in &response.data {
        images.push(image.b64_json.as_deref());
    }
    Ok(json!({"images": images}))
}
