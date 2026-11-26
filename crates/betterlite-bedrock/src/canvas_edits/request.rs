use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageFiles;
use indexmap::IndexMap;

use super::error::CanvasEditError;
use super::file_types::file_types_to_b64;
use super::params::size_pair;
use super::task_body::{INPAINTING, OUTPAINTING, TaskBodyInput, canvas_task_body};
use super::types::{
    CanvasEditOptions, CanvasEditRequest, CanvasEditValue, CanvasImageInput, ImageGenerationConfig,
};

pub struct CanvasEditRequestInput<'request> {
    pub prompt: Option<&'request str>,
    pub image: Option<&'request CanvasImageInput>,
    pub options: &'request CanvasEditOptions,
}

// LiteLLM converts the image first, then the mask, then reads every option before judging the prompt.
pub fn transform_image_edit_request(
    input: &CanvasEditRequestInput<'_>,
) -> Result<CanvasEditRequest, CanvasEditError> {
    let image = file_types_to_b64(input.image)?;
    let mut options = input.options.clone();
    let mask = take_mask(&mut options)?;
    let (width, height) = take_dimensions(&mut options)?;
    let generation = take_generation(&mut options, width, height)?;
    let task_type = match take_json(options.task_type.take(), "taskType")? {
        None => None,
        Some(Json::Text(name)) => Some(name),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(CanvasEditError::Field {
                field: "taskType".to_owned(),
            });
        }
    };
    let text = task_text(input.prompt, task_type.as_deref())?;
    let negative_text = take_json(options.negative_text.take(), "negativeText")?;
    let similarity_strength = take_json(options.similarity_strength.take(), "similarityStrength")?;
    let mask_prompt = take_json(options.mask_prompt.take(), "maskPrompt")?;
    let out_painting_mode = take_json(options.out_painting_mode.take(), "outPaintingMode")?;
    let body = canvas_task_body(
        &TaskBodyInput {
            image: &image,
            mask: mask.as_deref(),
            text: &text,
            negative_text: negative_text.as_ref(),
            similarity_strength: similarity_strength.as_ref(),
            task_type: task_type.as_deref(),
            mask_prompt: mask_prompt.as_ref(),
            out_painting_mode: out_painting_mode.as_ref(),
        },
        generation,
    )?;
    Ok(CanvasEditRequest {
        body,
        files: ImageFiles::new(),
    })
}

fn take_mask(options: &mut CanvasEditOptions) -> Result<Option<String>, CanvasEditError> {
    let Some(value) = options.mask.take() else {
        return Ok(None);
    };
    let image = match value {
        CanvasEditValue::Json(Json::Null) => return Ok(None),
        CanvasEditValue::Json(Json::Text(text)) => CanvasImageInput::Text(text),
        CanvasEditValue::Bytes(content) => CanvasImageInput::Bytes(content),
        CanvasEditValue::Tuple => CanvasImageInput::Tuple,
        CanvasEditValue::Json(
            Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
        ) => {
            return Err(CanvasEditError::Field {
                field: "mask".to_owned(),
            });
        }
    };
    file_types_to_b64(Some(&image)).map(Some)
}

// The size is split on the first "x", and only reached when neither width nor height was given.
fn take_dimensions(
    options: &mut CanvasEditOptions,
) -> Result<(Option<Json>, Option<Json>), CanvasEditError> {
    let size = options.size.take();
    let width = take_json(options.width.take(), "width")?;
    let height = take_json(options.height.take(), "height")?;
    if width.is_none()
        && height.is_none()
        && let Some((left, right)) = size.as_ref().and_then(size_pair)
    {
        return Ok((Some(left), Some(right)));
    }
    Ok((width, height))
}

// A nested config is copied only when it is an object, and each non-None top-level value overwrites it.
fn take_generation(
    options: &mut CanvasEditOptions,
    width: Option<Json>,
    height: Option<Json>,
) -> Result<Option<ImageGenerationConfig>, CanvasEditError> {
    let mut config = ImageGenerationConfig::default();
    for (name, value) in take_nested(options) {
        match name.as_str() {
            "width" => config.width = Some(value),
            "height" => config.height = Some(value),
            "numberOfImages" => config.number_of_images = Some(value),
            "quality" => config.quality = Some(value),
            "cfgScale" => config.cfg_scale = Some(value),
            "seed" => config.seed = Some(value),
            other => {
                config.passthrough.insert(other.to_owned(), value);
            }
        }
    }
    if let Some(value) = width {
        config.width = Some(value);
    }
    if let Some(value) = height {
        config.height = Some(value);
    }
    if let Some(value) = take_json(options.number_of_images.take(), "numberOfImages")? {
        config.number_of_images = Some(value);
    }
    if let Some(value) = take_json(options.quality.take(), "quality")? {
        config.quality = Some(value);
    }
    if let Some(value) = take_json(options.cfg_scale.take(), "cfgScale")? {
        config.cfg_scale = Some(value);
    }
    if let Some(value) = take_json(options.seed.take(), "seed")? {
        config.seed = Some(value);
    }
    Ok((!config.is_empty()).then_some(config))
}

fn take_nested(options: &mut CanvasEditOptions) -> IndexMap<String, Json> {
    match options.image_generation_config.take() {
        Some(CanvasEditValue::Json(Json::Object(fields))) => fields,
        None
        | Some(
            CanvasEditValue::Json(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_),
            )
            | CanvasEditValue::Bytes(_)
            | CanvasEditValue::Tuple,
        ) => IndexMap::new(),
    }
}

fn take_json(value: Option<CanvasEditValue>, name: &str) -> Result<Option<Json>, CanvasEditError> {
    let Some(value) = value.filter(|taken| !matches!(taken, CanvasEditValue::Json(Json::Null)))
    else {
        return Ok(None);
    };
    value
        .to_json()
        .map(Some)
        .ok_or_else(|| CanvasEditError::Field {
            field: name.to_owned(),
        })
}

fn task_text(prompt: Option<&str>, task_type: Option<&str>) -> Result<String, CanvasEditError> {
    if let Some(name) = task_type.filter(|name| *name == INPAINTING || *name == OUTPAINTING)
        && prompt.is_none_or(str::is_empty)
    {
        return Err(CanvasEditError::MissingPrompt {
            task_type: name.to_owned(),
        });
    }
    Ok(match prompt {
        Some(text) if !text.is_empty() => text.to_owned(),
        None | Some(_) => " ".to_owned(),
    })
}
