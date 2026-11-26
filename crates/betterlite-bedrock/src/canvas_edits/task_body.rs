use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::CanvasEditError;

pub const BACKGROUND_REMOVAL: &str = "BACKGROUND_REMOVAL";
pub const OUTPAINTING: &str = "OUTPAINTING";
pub const IMAGE_VARIATION: &str = "IMAGE_VARIATION";
pub const INPAINTING: &str = "INPAINTING";

const BACKGROUND_REMOVAL_PARAMS: &str = "backgroundRemovalParams";
const OUT_PAINTING_PARAMS: &str = "outPaintingParams";
const IMAGE_VARIATION_PARAMS: &str = "imageVariationParams";
const IN_PAINTING_PARAMS: &str = "inPaintingParams";

pub struct TaskBodyInput<'body> {
    pub image: &'body str,
    pub mask: Option<&'body str>,
    pub text: &'body str,
    pub negative_text: Option<&'body Json>,
    pub similarity_strength: Option<&'body Json>,
    pub task_type: Option<&'body str>,
    pub mask_prompt: Option<&'body Json>,
    pub out_painting_mode: Option<&'body Json>,
}

// The wire shape of one task body: the taskType tag, its params key, and the params object.
pub struct BuiltTaskBody {
    pub task_type: &'static str,
    pub params_key: &'static str,
    pub params: IndexMap<String, Json>,
}

// every variant is boxed so no payload is over 3x another, the limit variant_size_differences enforces
pub enum CanvasTaskBody {
    BackgroundRemoval(Box<BackgroundRemoval>),
    OutPainted(Box<PaintedTask>),
    ImageVariation(Box<ImageVariation>),
    InPainted(Box<PaintedTask>),
}

pub struct BackgroundRemoval {
    pub image: String,
}

// INPAINTING and OUTPAINTING share every field; only OUTPAINTING ever carries outPaintingMode.
pub struct PaintedTask {
    pub image: String,
    pub text: String,
    pub mask_prompt: Option<Json>,
    pub mask_image: Option<String>,
    pub negative_text: Option<Json>,
    pub out_painting_mode: Option<Json>,
}

pub struct ImageVariation {
    pub image: String,
    pub text: String,
    pub negative_text: Option<Json>,
    pub similarity_strength: Option<Json>,
}

impl CanvasTaskBody {
    #[must_use]
    pub fn build(self) -> BuiltTaskBody {
        match self {
            Self::BackgroundRemoval(body) => BuiltTaskBody {
                task_type: BACKGROUND_REMOVAL,
                params_key: BACKGROUND_REMOVAL_PARAMS,
                params: IndexMap::from([("image".to_owned(), Json::Text(body.image))]),
            },
            Self::OutPainted(body) => BuiltTaskBody {
                task_type: OUTPAINTING,
                params_key: OUT_PAINTING_PARAMS,
                params: painted_params(*body),
            },
            Self::ImageVariation(body) => BuiltTaskBody {
                task_type: IMAGE_VARIATION,
                params_key: IMAGE_VARIATION_PARAMS,
                params: variation_params(*body),
            },
            Self::InPainted(body) => BuiltTaskBody {
                task_type: INPAINTING,
                params_key: IN_PAINTING_PARAMS,
                params: painted_params(*body),
            },
        }
    }
}

// A taskType that is none, empty or whitespace routes automatically; any other name must be INPAINTING.
pub fn canvas_task_body(input: &TaskBodyInput<'_>) -> Result<CanvasTaskBody, CanvasEditError> {
    if input.task_type == Some(BACKGROUND_REMOVAL) {
        return Ok(CanvasTaskBody::BackgroundRemoval(Box::new(
            BackgroundRemoval {
                image: input.image.to_owned(),
            },
        )));
    }
    if input.task_type == Some(OUTPAINTING) {
        if !has_mask(input) {
            return Err(CanvasEditError::OutPaintingMask);
        }
        let body = painted_task(input, input.out_painting_mode.cloned());
        return Ok(CanvasTaskBody::OutPainted(Box::new(body)));
    }
    if input.task_type == Some(IMAGE_VARIATION) {
        return Ok(CanvasTaskBody::ImageVariation(Box::new(variation_task(
            input,
        ))));
    }
    reject_unsupported_task(input.task_type)?;
    if input.task_type == Some(INPAINTING) && !has_mask(input) {
        return Err(CanvasEditError::InPaintingMask);
    }
    if has_mask(input) || input.task_type == Some(INPAINTING) {
        let body = painted_task(input, None);
        return Ok(CanvasTaskBody::InPainted(Box::new(body)));
    }
    Ok(CanvasTaskBody::ImageVariation(Box::new(variation_task(
        input,
    ))))
}

const fn has_mask(input: &TaskBodyInput<'_>) -> bool {
    input.mask.is_some() || input.mask_prompt.is_some()
}

fn painted_task(input: &TaskBodyInput<'_>, out_painting_mode: Option<Json>) -> PaintedTask {
    PaintedTask {
        image: input.image.to_owned(),
        text: input.text.to_owned(),
        mask_prompt: input.mask_prompt.cloned(),
        mask_image: input.mask.map(str::to_owned),
        negative_text: input.negative_text.cloned(),
        out_painting_mode,
    }
}

fn variation_task(input: &TaskBodyInput<'_>) -> ImageVariation {
    ImageVariation {
        image: input.image.to_owned(),
        text: input.text.to_owned(),
        negative_text: input.negative_text.cloned(),
        similarity_strength: input.similarity_strength.cloned(),
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

// A maskPrompt wins over a mask image wherever LiteLLM lets both through.
fn painted_params(body: PaintedTask) -> IndexMap<String, Json> {
    let mut params = IndexMap::new();
    params.insert("image".to_owned(), Json::Text(body.image));
    params.insert("text".to_owned(), Json::Text(body.text));
    if let Some(prompt) = body.mask_prompt {
        params.insert("maskPrompt".to_owned(), prompt);
    } else if let Some(image) = body.mask_image {
        params.insert("maskImage".to_owned(), Json::Text(image));
    }
    if let Some(negative) = body.negative_text {
        params.insert("negativeText".to_owned(), negative);
    }
    if let Some(mode) = body.out_painting_mode {
        params.insert("outPaintingMode".to_owned(), mode);
    }
    params
}

fn variation_params(body: ImageVariation) -> IndexMap<String, Json> {
    let mut params = IndexMap::new();
    params.insert(
        "images".to_owned(),
        Json::List(vec![Json::Text(body.image)]),
    );
    params.insert("text".to_owned(), Json::Text(body.text));
    if let Some(negative) = body.negative_text {
        params.insert("negativeText".to_owned(), negative);
    }
    if let Some(strength) = body.similarity_strength {
        params.insert("similarityStrength".to_owned(), strength);
    }
    params
}
