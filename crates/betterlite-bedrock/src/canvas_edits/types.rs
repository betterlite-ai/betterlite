use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageFiles, ImageParams};
use indexmap::IndexMap;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::error::CanvasEditError;

// The options LiteLLM's mapper and transform read by name; every other key stays in passthrough.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct CanvasEditOptions {
    #[serde(rename = "n", skip_serializing_if = "Option::is_none")]
    pub image_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask: Option<CanvasImageInput>,
    #[serde(rename = "negativeText", skip_serializing_if = "Option::is_none")]
    pub negative_text: Option<String>,
    #[serde(rename = "similarityStrength", skip_serializing_if = "Option::is_none")]
    pub similarity_strength: Option<f64>,
    #[serde(rename = "cfgScale", skip_serializing_if = "Option::is_none")]
    pub cfg_scale: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(rename = "taskType", skip_serializing_if = "Option::is_none")]
    pub task_type: Option<String>,
    #[serde(rename = "maskPrompt", skip_serializing_if = "Option::is_none")]
    pub mask_prompt: Option<String>,
    #[serde(rename = "outPaintingMode", skip_serializing_if = "Option::is_none")]
    pub out_painting_mode: Option<String>,
    #[serde(
        rename = "imageGenerationConfig",
        skip_serializing_if = "Option::is_none"
    )]
    pub image_generation_config: Option<ImageGenerationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<i64>,
    #[serde(rename = "numberOfImages", skip_serializing_if = "Option::is_none")]
    pub number_of_images: Option<i64>,
    #[serde(flatten)]
    pub passthrough: IndexMap<String, Json>,
}

// A Python image value: a supplied string, bytes already read from a file or SDK object, or a tuple FileTypes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
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
    pub mask_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask_image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub negative_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out_painting_mode: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VariationParams {
    pub images: Vec<String>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub negative_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity_strength: Option<f64>,
}

// The nested config keeps the caller's other keys; the six top-level values overwrite their own key.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageGenerationConfig {
    #[serde(flatten)]
    pub passthrough: IndexMap<String, Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number_of_images: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cfg_scale: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
}

#[derive(Debug)]
pub struct CanvasEditRequest {
    pub body: CanvasEditBody,
    pub files: ImageFiles,
}

impl CanvasEditOptions {
    // The request's option dict by its OpenAI and Nova Canvas names; a key neither names is kept aside.
    pub fn from_params(params: &ImageParams) -> Result<Self, CanvasEditError> {
        let mut remaining = params.clone();
        let options = Self {
            image_count: field(&mut remaining, "n")?,
            size: field(&mut remaining, "size")?,
            response_format: field(&mut remaining, "response_format")?,
            mask: mask_input(&mut remaining)?,
            negative_text: field(&mut remaining, "negativeText")?,
            similarity_strength: field(&mut remaining, "similarityStrength")?,
            cfg_scale: field(&mut remaining, "cfgScale")?,
            seed: field(&mut remaining, "seed")?,
            quality: field(&mut remaining, "quality")?,
            task_type: field(&mut remaining, "taskType")?,
            mask_prompt: field(&mut remaining, "maskPrompt")?,
            out_painting_mode: field(&mut remaining, "outPaintingMode")?,
            image_generation_config: generation_input(&mut remaining)?,
            width: field(&mut remaining, "width")?,
            height: field(&mut remaining, "height")?,
            number_of_images: field(&mut remaining, "numberOfImages")?,
            passthrough: remaining,
        };
        Ok(options)
    }
}

// Each named option is parsed here with its declared type; a value the type cannot hold names the key.
fn field<T>(params: &mut ImageParams, name: &str) -> Result<Option<T>, CanvasEditError>
where
    T: DeserializeOwned,
{
    let Some(value) = params.shift_remove(name) else {
        return Ok(None);
    };
    typed(&value, name)
}

fn typed<T>(value: &Json, name: &str) -> Result<Option<T>, CanvasEditError>
where
    T: DeserializeOwned,
{
    serde_json::to_string(value)
        .ok()
        .and_then(|text| serde_json::from_str::<Option<T>>(&text).ok())
        .ok_or_else(|| CanvasEditError::Field {
            field: name.to_owned(),
        })
}

// A JSON mask is a base64 string or null; bytes and tuples arrive from the SDK-side input instead.
fn mask_input(params: &mut ImageParams) -> Result<Option<CanvasImageInput>, CanvasEditError> {
    match params.shift_remove("mask") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(CanvasImageInput::Text(text))),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(CanvasEditError::Field {
                field: "mask".to_owned(),
            })
        }
    }
}

// A nested config is read only when it is an object; any other value is ignored, as LiteLLM's isinstance check does.
fn generation_input(
    params: &mut ImageParams,
) -> Result<Option<ImageGenerationConfig>, CanvasEditError> {
    let Some(value) = params.shift_remove("imageGenerationConfig") else {
        return Ok(None);
    };
    if matches!(value, Json::Object(_)) {
        typed(&value, "imageGenerationConfig")
    } else {
        Ok(None)
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
