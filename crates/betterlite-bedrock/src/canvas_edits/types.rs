use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageFiles, ImageParams};
use indexmap::IndexMap;
use serde::Serialize;

// The options LiteLLM's mapper and transform read by name; every other key stays in passthrough.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CanvasEditOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<CanvasEditValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<CanvasEditValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<CanvasEditValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask: Option<CanvasEditValue>,
    #[serde(rename = "negativeText", skip_serializing_if = "Option::is_none")]
    pub negative_text: Option<CanvasEditValue>,
    #[serde(rename = "similarityStrength", skip_serializing_if = "Option::is_none")]
    pub similarity_strength: Option<CanvasEditValue>,
    #[serde(rename = "cfgScale", skip_serializing_if = "Option::is_none")]
    pub cfg_scale: Option<CanvasEditValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<CanvasEditValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<CanvasEditValue>,
    #[serde(rename = "taskType", skip_serializing_if = "Option::is_none")]
    pub task_type: Option<CanvasEditValue>,
    #[serde(rename = "maskPrompt", skip_serializing_if = "Option::is_none")]
    pub mask_prompt: Option<CanvasEditValue>,
    #[serde(rename = "outPaintingMode", skip_serializing_if = "Option::is_none")]
    pub out_painting_mode: Option<CanvasEditValue>,
    #[serde(
        rename = "imageGenerationConfig",
        skip_serializing_if = "Option::is_none"
    )]
    pub image_generation_config: Option<CanvasEditValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<CanvasEditValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<CanvasEditValue>,
    #[serde(rename = "numberOfImages", skip_serializing_if = "Option::is_none")]
    pub number_of_images: Option<CanvasEditValue>,
    #[serde(flatten)]
    pub passthrough: IndexMap<String, CanvasEditValue>,
}

// A Python option value: JSON, bytes already read from a file or SDK object, or a tuple FileTypes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum CanvasEditValue {
    Json(Json),
    Bytes(Vec<u8>),
    Tuple,
}

// The Python wheel reads a file-like image at its current position, so bytes arrive already read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CanvasImageInput {
    Bytes(Vec<u8>),
    Text(String),
    Tuple,
}

// The InvokeModel body: the taskType tag, its params record, and the optional generation config.
#[derive(Debug, Serialize)]
#[serde(tag = "taskType")]
pub enum CanvasEditBody {
    #[serde(rename = "BACKGROUND_REMOVAL")]
    BackgroundRemoval {
        #[serde(rename = "backgroundRemovalParams")]
        params: Box<BackgroundRemovalParams>,
    },
    #[serde(rename = "OUTPAINTING")]
    OutPainting {
        #[serde(rename = "outPaintingParams")]
        params: Box<PaintedParams>,
        #[serde(
            rename = "imageGenerationConfig",
            skip_serializing_if = "Option::is_none"
        )]
        image_generation_config: Option<Box<ImageGenerationConfig>>,
    },
    #[serde(rename = "INPAINTING")]
    InPainting {
        #[serde(rename = "inPaintingParams")]
        params: Box<PaintedParams>,
        #[serde(
            rename = "imageGenerationConfig",
            skip_serializing_if = "Option::is_none"
        )]
        image_generation_config: Option<Box<ImageGenerationConfig>>,
    },
    #[serde(rename = "IMAGE_VARIATION")]
    ImageVariation {
        #[serde(rename = "imageVariationParams")]
        params: Box<VariationParams>,
        #[serde(
            rename = "imageGenerationConfig",
            skip_serializing_if = "Option::is_none"
        )]
        image_generation_config: Option<Box<ImageGenerationConfig>>,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundRemovalParams {
    pub image: String,
}

// INPAINTING and OUTPAINTING share every field; only OUTPAINTING ever carries outPaintingMode.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaintedParams {
    pub image: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask_prompt: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask_image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub negative_text: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out_painting_mode: Option<Json>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VariationParams {
    pub images: Vec<String>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub negative_text: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity_strength: Option<Json>,
}

// The nested config keeps the caller's other keys; the six top-level values overwrite their own key.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageGenerationConfig {
    #[serde(flatten)]
    pub passthrough: IndexMap<String, Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number_of_images: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cfg_scale: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<Json>,
}

#[derive(Debug)]
pub struct CanvasEditRequest {
    pub body: CanvasEditBody,
    pub files: ImageFiles,
}

impl CanvasEditOptions {
    // The request's option dict by its OpenAI and Nova Canvas names; a key neither names is kept aside.
    #[must_use]
    pub fn from_params(params: &ImageParams) -> Self {
        let mut options = Self::default();
        for (name, value) in params {
            let value = CanvasEditValue::Json(value.clone());
            match name.as_str() {
                "n" => options.n = Some(value),
                "size" => options.size = Some(value),
                "response_format" => options.response_format = Some(value),
                "mask" => options.mask = Some(value),
                "negativeText" => options.negative_text = Some(value),
                "similarityStrength" => options.similarity_strength = Some(value),
                "cfgScale" => options.cfg_scale = Some(value),
                "seed" => options.seed = Some(value),
                "quality" => options.quality = Some(value),
                "taskType" => options.task_type = Some(value),
                "maskPrompt" => options.mask_prompt = Some(value),
                "outPaintingMode" => options.out_painting_mode = Some(value),
                "imageGenerationConfig" => options.image_generation_config = Some(value),
                "width" => options.width = Some(value),
                "height" => options.height = Some(value),
                "numberOfImages" => options.number_of_images = Some(value),
                other => {
                    options.passthrough.insert(other.to_owned(), value);
                }
            }
        }
        options
    }
}

impl CanvasEditValue {
    // None when the value holds bytes or a tuple, which no JSON body can carry
    #[must_use]
    pub fn to_json(&self) -> Option<Json> {
        match self {
            Self::Json(value) => Some(value.clone()),
            Self::Bytes(_) | Self::Tuple => None,
        }
    }
}

impl ImageGenerationConfig {
    // LiteLLM includes the config only when its dict is non-empty; an explicit null still counts.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.passthrough.is_empty()
            && self.width.is_none()
            && self.height.is_none()
            && self.number_of_images.is_none()
            && self.quality.is_none()
            && self.cfg_scale.is_none()
            && self.seed.is_none()
    }
}
