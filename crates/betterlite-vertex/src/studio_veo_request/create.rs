use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};

use super::error::StudioVeoError;
use super::image::gemini_image;
use super::params::MappedParams;
use crate::vertex_veo_request::image::{ImageInput, MimeReader};

pub struct CreateInput<'media> {
    pub prompt: &'media str,
    pub api_base: &'media str,
    pub mapped: MappedParams<'media>,
}

// GeminiVideoGenerationRequest, after LiteLLM's model
#[derive(Debug, Serialize)]
pub struct CreateBody {
    pub instances: Vec<Instance>,
    pub parameters: Parameters,
}

// GeminiVideoGenerationInstance
#[derive(Debug, Serialize)]
pub struct Instance {
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<Object>,
}

// GeminiVideoGenerationParameters, in declaration order, with absent and null alike dropped
#[derive(Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Parameters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub negative_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_frame: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_images: Option<Vec<Json>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub person_generation: Option<String>,
}

#[derive(Debug)]
pub struct CreateForm {
    pub body: CreateBody,
    pub url: String,
}

// transform_video_create_request: the image leaves the options, and the parameters carry their declared types
pub fn create_request(
    input: CreateInput<'_>,
    mime: &dyn MimeReader,
) -> Result<CreateForm, StudioVeoError> {
    let CreateInput {
        prompt,
        api_base,
        mapped,
    } = input;
    let MappedParams {
        image: image_option,
        options,
    } = mapped;
    let image = match image_option {
        Some(ImageInput::Json(Json::Null)) | None => None,
        Some(chosen) => Some(gemini_image(chosen, mime)?),
    };
    Ok(CreateForm {
        body: CreateBody {
            instances: vec![Instance {
                prompt: prompt.to_owned(),
                image,
            }],
            parameters: parameters_from(options)?,
        },
        url: api_base.to_owned(),
    })
}

// GeminiVideoGenerationParameters(**params_copy): unknown keys are ignored, null becomes None and a wrong type names its field
fn parameters_from(options: Object) -> Result<Parameters, StudioVeoError> {
    let text = serde_json::to_string(&Json::Object(options)).map_err(|source| {
        StudioVeoError::Parameters {
            path: "parameters".to_owned(),
            source,
        }
    })?;
    let mut deserializer = serde_json::Deserializer::from_str(&text);
    serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
        StudioVeoError::Parameters {
            path: error.path().to_string(),
            source: error.into_inner(),
        }
    })
}
