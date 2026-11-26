use betterlite_openai::api::image_contracts::records::ImageFiles;

use super::error::CanvasEditError;
use super::file_types::file_types_to_b64;
use super::params::size_pair;
use super::task_body::{INPAINTING, OUTPAINTING, TaskBodyInput, canvas_task_body};
use super::types::{CanvasEditOptions, CanvasEditRequest, CanvasImageInput, ImageGenerationConfig};

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
    let (width, height) = take_dimensions(&mut options);
    let generation = take_generation(&mut options, width, height);
    let task_type = options.task_type.take();
    let text = task_text(input.prompt, task_type.as_deref())?;
    let negative_text = options.negative_text.take();
    let similarity_strength = options.similarity_strength;
    let mask_prompt = options.mask_prompt.take();
    let out_painting_mode = options.out_painting_mode.take();
    let body = canvas_task_body(
        &TaskBodyInput {
            image: &image,
            mask: mask.as_deref(),
            text: &text,
            negative_text: negative_text.as_deref(),
            similarity_strength,
            task_type: task_type.as_deref(),
            mask_prompt: mask_prompt.as_deref(),
            out_painting_mode: out_painting_mode.as_deref(),
        },
        generation,
    )?;
    Ok(CanvasEditRequest {
        body,
        files: ImageFiles::new(),
    })
}

fn take_mask(options: &mut CanvasEditOptions) -> Result<Option<String>, CanvasEditError> {
    let Some(mask) = options.mask.take() else {
        return Ok(None);
    };
    file_types_to_b64(Some(&mask)).map(Some)
}

// The size is split on the first "x", and only reached when neither width nor height was given.
fn take_dimensions(options: &mut CanvasEditOptions) -> (Option<i64>, Option<i64>) {
    let size = options.size.take();
    let width = options.width.take();
    let height = options.height.take();
    if width.is_none()
        && height.is_none()
        && let Some((left, right)) = size.as_deref().and_then(size_pair)
    {
        return (Some(left), Some(right));
    }
    (width, height)
}

// A nested config is kept only when it is non-empty, and each non-None top-level value overwrites it.
fn take_generation(
    options: &mut CanvasEditOptions,
    width: Option<i64>,
    height: Option<i64>,
) -> Option<ImageGenerationConfig> {
    let mut config = options.image_generation_config.take().unwrap_or_default();
    config.width = width.or(config.width);
    config.height = height.or(config.height);
    config.number_of_images = options.number_of_images.take().or(config.number_of_images);
    config.quality = options.quality.take().or(config.quality);
    config.cfg_scale = options.cfg_scale.take().or(config.cfg_scale);
    config.seed = options.seed.take().or(config.seed);
    (!config.is_empty()).then_some(config)
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
