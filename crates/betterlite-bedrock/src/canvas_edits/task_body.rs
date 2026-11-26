use super::error::CanvasEditError;
use super::types::{
    BackgroundRemovalParams, CanvasEditBody, ImageGenerationConfig, PaintedParams, VariationParams,
};

pub const BACKGROUND_REMOVAL: &str = "BACKGROUND_REMOVAL";
pub const OUTPAINTING: &str = "OUTPAINTING";
pub const IMAGE_VARIATION: &str = "IMAGE_VARIATION";
pub const INPAINTING: &str = "INPAINTING";

pub struct TaskBodyInput<'body> {
    pub image: &'body str,
    pub mask: Option<&'body str>,
    pub text: &'body str,
    pub negative_text: Option<&'body str>,
    pub similarity_strength: Option<f64>,
    pub task_type: Option<&'body str>,
    pub mask_prompt: Option<&'body str>,
    pub out_painting_mode: Option<&'body str>,
}

// Every explicit task name has its own branch; only INPAINTING reaches the tail, and the rest route by mask.
pub fn canvas_task_body(
    input: &TaskBodyInput<'_>,
    generation: Option<ImageGenerationConfig>,
) -> Result<CanvasEditBody, CanvasEditError> {
    if input.task_type == Some(BACKGROUND_REMOVAL) {
        return Ok(CanvasEditBody::BackgroundRemoval {
            params: Box::new(BackgroundRemovalParams {
                image: input.image.to_owned(),
            }),
        });
    }
    let painted = input.task_type == Some(INPAINTING) || input.task_type == Some(OUTPAINTING);
    if painted && input.mask.is_none() && input.mask_prompt.is_none() {
        return Err(if input.task_type == Some(OUTPAINTING) {
            CanvasEditError::OutPaintingMask
        } else {
            CanvasEditError::InPaintingMask
        });
    }
    if input.task_type == Some(OUTPAINTING) {
        return Ok(painted_body(input, generation, true));
    }
    let variation = input.task_type == Some(IMAGE_VARIATION);
    if !variation {
        reject_unsupported_task(input.task_type)?;
        if input.mask.is_some()
            || input.mask_prompt.is_some()
            || input.task_type == Some(INPAINTING)
        {
            return Ok(painted_body(input, generation, false));
        }
    }
    Ok(CanvasEditBody::ImageVariation {
        params: Box::new(VariationParams {
            images: vec![input.image.to_owned()],
            text: input.text.to_owned(),
            negative_text: input.negative_text.map(str::to_owned),
            similarity_strength: input.similarity_strength,
        }),
        image_generation_config: generation.map(Box::new),
    })
}

// A maskPrompt wins over a mask image wherever LiteLLM lets both through; only OUTPAINTING carries the mode.
fn painted_body(
    input: &TaskBodyInput<'_>,
    generation: Option<ImageGenerationConfig>,
    out_painting: bool,
) -> CanvasEditBody {
    let params = Box::new(PaintedParams {
        image: input.image.to_owned(),
        text: input.text.to_owned(),
        mask_prompt: input.mask_prompt.map(str::to_owned),
        mask_image: input
            .mask
            .filter(|_mask| input.mask_prompt.is_none())
            .map(str::to_owned),
        negative_text: input.negative_text.map(str::to_owned),
        out_painting_mode: input
            .out_painting_mode
            .filter(|_mode| out_painting)
            .map(str::to_owned),
    });
    let image_generation_config = generation.map(Box::new);
    if out_painting {
        CanvasEditBody::OutPainting {
            params,
            image_generation_config,
        }
    } else {
        CanvasEditBody::InPainting {
            params,
            image_generation_config,
        }
    }
}

fn reject_unsupported_task(task_type: Option<&str>) -> Result<(), CanvasEditError> {
    match task_type {
        Some(name) if !name.trim().is_empty() && name != INPAINTING => {
            Err(CanvasEditError::UnsupportedTaskType {
                task_type: name.to_owned(),
            })
        }
        Some(_) | None => Ok(()),
    }
}
