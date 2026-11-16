use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageFiles, ImageParams};
use serde::Serialize;
use serde_json::Number;

use super::error::ImagenEditError;
use super::input::{ImagenFileInput, read_image_bytes};

const REFERENCE_TYPE_RAW: &str = "REFERENCE_TYPE_RAW";
const REFERENCE_TYPE_MASK: &str = "REFERENCE_TYPE_MASK";
const MASK_MODE_USER_PROVIDED: &str = "MASK_MODE_USER_PROVIDED";
const EDIT_MODE_INPAINT_INSERTION: &str = "EDIT_MODE_INPAINT_INSERTION";
const DEFAULT_SAMPLE_COUNT: i64 = 1;
const BASE_STEPS: i64 = 50;
const GUIDANCE_SCALE: f64 = 7.5;
const DILATION: f64 = 0.03;

// the predict body, with fields in the order json.dumps writes LiteLLM's dicts
#[derive(Serialize)]
struct PredictBody<'prompt> {
    instances: [Instance<'prompt>; 1],
    parameters: Parameters,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Instance<'prompt> {
    prompt: &'prompt str,
    reference_images: Vec<ReferencePart>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReferencePart {
    reference_type: &'static str,
    reference_id: u64,
    reference_image: EncodedImage,
    #[serde(skip_serializing_if = "Option::is_none")]
    mask_image_config: Option<MaskImageConfig>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EncodedImage {
    bytes_base64_encoded: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MaskImageConfig {
    mask_mode: &'static str,
    dilation: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Parameters {
    sample_count: Number,
    edit_mode: &'static str,
    edit_config: EditConfig,
    guidance_scale: f64,
    // Vertex AI picks the seed, so the key goes out as null
    seed: Option<()>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EditConfig {
    base_steps: i64,
}

// transform_image_edit_request: the predict body as serialized JSON and no multipart files, with a missing image list refused before the prompt
pub fn transform_image_edit_request(
    prompt: Option<&str>,
    images: Option<&mut [Option<ImagenFileInput<'_>>]>,
    mask: Option<&mut ImagenFileInput<'_>>,
    mapped: &ImageParams,
    max_depth: i64,
) -> Result<(String, ImageFiles), ImagenEditError> {
    let Some(uploads) = images else {
        return Err(ImagenEditError::MissingImage);
    };
    // every reference is read before the prompt is checked, so a bad image wins over an absent prompt
    let references = prepare_references(uploads, mask, max_depth)?;
    let Some(edit_prompt) = prompt else {
        return Err(ImagenEditError::MissingPrompt);
    };
    let body = PredictBody {
        instances: [Instance {
            prompt: edit_prompt,
            reference_images: references,
        }],
        parameters: Parameters {
            sample_count: sample_count(mapped)?,
            edit_mode: EDIT_MODE_INPAINT_INSERTION,
            edit_config: EditConfig {
                base_steps: BASE_STEPS,
            },
            guidance_scale: GUIDANCE_SCALE,
            seed: None,
        },
    };
    let payload = serde_json::to_string(&body).map_err(ImagenEditError::Serialize)?;
    Ok((payload, ImageFiles::new()))
}

// _prepare_reference_images: raw references keep their one-based list position, and the mask comes last
fn prepare_references(
    images: &mut [Option<ImagenFileInput<'_>>],
    mask: Option<&mut ImagenFileInput<'_>>,
    max_depth: i64,
) -> Result<Vec<ReferencePart>, ImagenEditError> {
    let mut references = Vec::new();
    let mut position = 0_u64;
    for image in images.iter_mut() {
        position = position.saturating_add(1);
        let Some(stored) = image else {
            continue;
        };
        let content = read_image_bytes(stored, 0, max_depth)?;
        references.push(ReferencePart {
            reference_type: REFERENCE_TYPE_RAW,
            reference_id: position,
            reference_image: EncodedImage {
                bytes_base64_encoded: STANDARD.encode(content),
            },
            mask_image_config: None,
        });
    }
    // len(reference_images) + 1 may repeat an earlier raw id, and LiteLLM keeps the collision
    if let Some(mask_image) = mask {
        let content = read_image_bytes(mask_image, 0, max_depth)?;
        let mask_id = u64::try_from(references.len())
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        references.push(ReferencePart {
            reference_type: REFERENCE_TYPE_MASK,
            reference_id: mask_id,
            reference_image: EncodedImage {
                bytes_base64_encoded: STANDARD.encode(content),
            },
            mask_image_config: Some(MaskImageConfig {
                mask_mode: MASK_MODE_USER_PROVIDED,
                dilation: DILATION,
            }),
        });
    }
    // the list the request sends holds at least one reference, a mask alone included
    if references.is_empty() {
        return Err(ImagenEditError::MissingImage);
    }
    Ok(references)
}

// the mapped params hold the sampleCount LiteLLM reads, and an absent or null one takes one sample
fn sample_count(mapped: &ImageParams) -> Result<Number, ImagenEditError> {
    match mapped.get("sampleCount") {
        None | Some(Json::Null) => Ok(Number::from(DEFAULT_SAMPLE_COUNT)),
        Some(Json::Number(number)) => Ok(number.clone()),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(ImagenEditError::FieldType {
                param: "sampleCount",
                expected: "a number",
            })
        }
    }
}
