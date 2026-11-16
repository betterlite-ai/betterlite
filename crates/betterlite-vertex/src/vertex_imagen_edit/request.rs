use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageFiles, ImageParams};
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

// transform_image_edit_request: the predict body as serialized JSON, and no multipart files
pub fn transform_image_edit_request(
    prompt: Option<&str>,
    images: &mut [Option<ImagenFileInput<'_>>],
    mask: Option<&mut ImagenFileInput<'_>>,
    mapped: &ImageParams,
    max_depth: i64,
) -> Result<(String, ImageFiles), ImagenEditError> {
    // every reference is read before the prompt is checked, so a bad image wins over an absent prompt
    let references = prepare_references(images, mask, max_depth)?;
    let Some(edit_prompt) = prompt else {
        return Err(ImagenEditError::MissingPrompt);
    };
    let mut instance = ImageParams::new();
    instance.insert("prompt".to_owned(), Json::Text(edit_prompt.to_owned()));
    instance.insert("referenceImages".to_owned(), Json::List(references));
    let mut parameters = ImageParams::new();
    parameters.insert("sampleCount".to_owned(), sample_count(mapped)?);
    parameters.insert(
        "editMode".to_owned(),
        Json::Text(EDIT_MODE_INPAINT_INSERTION.to_owned()),
    );
    let mut edit_config = ImageParams::new();
    edit_config.insert(
        "baseSteps".to_owned(),
        Json::Number(Number::from(BASE_STEPS)),
    );
    parameters.insert("editConfig".to_owned(), Json::Object(edit_config));
    parameters.insert(
        "guidanceScale".to_owned(),
        Json::Number(
            Number::from_f64(GUIDANCE_SCALE).ok_or(ImagenEditError::FieldType {
                param: "guidanceScale",
                expected: "a finite number",
            })?,
        ),
    );
    parameters.insert("seed".to_owned(), Json::Null);
    let mut body = ImageParams::new();
    body.insert(
        "instances".to_owned(),
        Json::List(vec![Json::Object(instance)]),
    );
    body.insert("parameters".to_owned(), Json::Object(parameters));
    let payload = serde_json::to_string(&body).map_err(ImagenEditError::Serialize)?;
    Ok((payload, ImageFiles::new()))
}

// _prepare_reference_images: raw references keep their one-based list position, and the mask comes last
fn prepare_references(
    images: &mut [Option<ImagenFileInput<'_>>],
    mask: Option<&mut ImagenFileInput<'_>>,
    max_depth: i64,
) -> Result<Vec<Json>, ImagenEditError> {
    // an absent image is refused before anything is read
    if images.is_empty() {
        return Err(ImagenEditError::MissingImage);
    }
    let mut references = Vec::new();
    let mut position = 0_u64;
    let mut raw_count = 0_u64;
    for image in images.iter_mut() {
        position = position.saturating_add(1);
        let Some(stored) = image else {
            continue;
        };
        let content = read_image_bytes(stored, 0, max_depth)?;
        references.push(Json::Object(reference_object(
            REFERENCE_TYPE_RAW,
            position,
            &content,
        )));
        raw_count = raw_count.saturating_add(1);
    }
    if let Some(mask) = mask {
        references.push(mask_reference(
            mask,
            raw_count.saturating_add(1),
            max_depth,
        )?);
    }
    // the list the request sends holds at least one reference, a mask alone included
    if references.is_empty() {
        return Err(ImagenEditError::MissingImage);
    }
    Ok(references)
}

// len(reference_images) + 1 may repeat an earlier raw id, and LiteLLM keeps the collision
fn mask_reference(
    mask: &mut ImagenFileInput<'_>,
    reference_id: u64,
    max_depth: i64,
) -> Result<Json, ImagenEditError> {
    let content = read_image_bytes(mask, 0, max_depth)?;
    let mut config = ImageParams::new();
    config.insert(
        "maskMode".to_owned(),
        Json::Text(MASK_MODE_USER_PROVIDED.to_owned()),
    );
    config.insert(
        "dilation".to_owned(),
        Json::Number(
            Number::from_f64(DILATION).ok_or(ImagenEditError::FieldType {
                param: "dilation",
                expected: "a finite number",
            })?,
        ),
    );
    let mut reference = reference_object(REFERENCE_TYPE_MASK, reference_id, &content);
    reference.insert("maskImageConfig".to_owned(), Json::Object(config));
    Ok(Json::Object(reference))
}

fn reference_object(kind: &str, reference_id: u64, content: &[u8]) -> ImageParams {
    let mut encoded = ImageParams::new();
    encoded.insert(
        "bytesBase64Encoded".to_owned(),
        Json::Text(STANDARD.encode(content)),
    );
    let mut reference = ImageParams::new();
    reference.insert("referenceType".to_owned(), Json::Text(kind.to_owned()));
    reference.insert(
        "referenceId".to_owned(),
        Json::Number(Number::from(reference_id)),
    );
    reference.insert("referenceImage".to_owned(), Json::Object(encoded));
    reference
}

// the mapped params hold the sampleCount LiteLLM reads, and an absent or null one takes one sample
fn sample_count(mapped: &ImageParams) -> Result<Json, ImagenEditError> {
    match mapped.get("sampleCount") {
        None | Some(Json::Null) => Ok(Json::Number(Number::from(DEFAULT_SAMPLE_COUNT))),
        Some(Json::Number(number)) => Ok(Json::Number(number.clone())),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(ImagenEditError::FieldType {
                param: "sampleCount",
                expected: "a number",
            })
        }
    }
}
