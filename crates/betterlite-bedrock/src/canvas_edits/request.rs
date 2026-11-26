use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageFiles;
use indexmap::IndexMap;

use super::error::CanvasEditError;
use super::file_types::file_types_to_b64;
use super::params::{size_pair, take_value};
use super::task_body::{
    BACKGROUND_REMOVAL, BuiltTaskBody, INPAINTING, OUTPAINTING, TaskBodyInput, canvas_task_body,
};
use super::types::{CanvasEditOptions, CanvasEditRequest, CanvasEditValue, CanvasImageInput};

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
    let task_type = take_task_type(&mut options)?;
    let text = task_text(input.prompt, task_type.as_deref())?;
    let negative_text = take_json(&mut options, "negativeText")?;
    let similarity_strength = take_json(&mut options, "similarityStrength")?;
    let mask_prompt = take_json(&mut options, "maskPrompt")?;
    let out_painting_mode = take_json(&mut options, "outPaintingMode")?;
    let body = canvas_task_body(&TaskBodyInput {
        image: &image,
        mask: mask.as_deref(),
        text: &text,
        negative_text: negative_text.as_ref(),
        similarity_strength: similarity_strength.as_ref(),
        task_type: task_type.as_deref(),
        mask_prompt: mask_prompt.as_ref(),
        out_painting_mode: out_painting_mode.as_ref(),
    })?;
    Ok(CanvasEditRequest {
        body: body_json(body.build(), generation),
        files: ImageFiles::new(),
    })
}

// AWS rejects imageGenerationConfig on BACKGROUND_REMOVAL, and an empty one is left out too.
fn body_json(body: BuiltTaskBody, generation: IndexMap<String, Json>) -> Json {
    let mut json = IndexMap::new();
    json.insert("taskType".to_owned(), Json::Text(body.task_type.to_owned()));
    json.insert(body.params_key.to_owned(), Json::Object(body.params));
    if body.task_type != BACKGROUND_REMOVAL && !generation.is_empty() {
        json.insert("imageGenerationConfig".to_owned(), Json::Object(generation));
    }
    Json::Object(json)
}

fn take_mask(options: &mut CanvasEditOptions) -> Result<Option<String>, CanvasEditError> {
    let Some(value) = take_value(options, "mask") else {
        return Ok(None);
    };
    let image = match value {
        CanvasEditValue::Json(Json::Text(text)) => CanvasImageInput::Text(text),
        CanvasEditValue::Bytes(content) => CanvasImageInput::Bytes(content),
        CanvasEditValue::Tuple => CanvasImageInput::Tuple,
        CanvasEditValue::Json(
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
        ) => {
            return Err(field_error("mask"));
        }
    };
    file_types_to_b64(Some(&image)).map(Some)
}

// The size is split on the first "x", and only reached when neither width nor height was given.
fn take_dimensions(
    options: &mut CanvasEditOptions,
) -> Result<(Option<Json>, Option<Json>), CanvasEditError> {
    let size = options.shift_remove("size");
    let width = take_json(options, "width")?;
    let height = take_json(options, "height")?;
    if width.is_none()
        && height.is_none()
        && let Some((left, right)) = size.as_ref().and_then(size_pair)
    {
        return Ok((Some(left), Some(right)));
    }
    Ok((width, height))
}

fn take_generation(
    options: &mut CanvasEditOptions,
    width: Option<Json>,
    height: Option<Json>,
) -> Result<IndexMap<String, Json>, CanvasEditError> {
    let overrides = [
        ("width", width),
        ("height", height),
        ("numberOfImages", take_json(options, "numberOfImages")?),
        ("quality", take_json(options, "quality")?),
        ("cfgScale", take_json(options, "cfgScale")?),
        ("seed", take_json(options, "seed")?),
    ];
    // The nested config's keys keep their order and a top-level value overwrites its own key.
    let mut config = take_nested(options);
    for (name, value) in overrides {
        if let Some(json) = value {
            config.insert(name.to_owned(), json);
        }
    }
    Ok(config)
}

fn take_nested(options: &mut CanvasEditOptions) -> IndexMap<String, Json> {
    match take_value(options, "imageGenerationConfig") {
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

fn take_task_type(options: &mut CanvasEditOptions) -> Result<Option<String>, CanvasEditError> {
    match take_value(options, "taskType") {
        None => Ok(None),
        Some(CanvasEditValue::Json(Json::Text(text))) => Ok(Some(text)),
        Some(
            CanvasEditValue::Json(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | CanvasEditValue::Bytes(_)
            | CanvasEditValue::Tuple,
        ) => Err(field_error("taskType")),
    }
}

fn take_json(options: &mut CanvasEditOptions, name: &str) -> Result<Option<Json>, CanvasEditError> {
    match take_value(options, name) {
        None => Ok(None),
        Some(value) => value.to_json().map(Some).ok_or_else(|| field_error(name)),
    }
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

fn field_error(name: &str) -> CanvasEditError {
    CanvasEditError::Field {
        field: name.to_owned(),
    }
}
