use betterlite_prompts::arguments::json::{Json, Object};

use super::error::StudioVeoError;
use super::image::gemini_image;
use super::params::MappedParams;
use crate::vertex_veo_request::image::{ImageInput, MimeReader};

pub struct CreateInput<'media> {
    pub prompt: &'media str,
    pub api_base: &'media str,
    pub mapped: MappedParams<'media>,
}

#[derive(Debug)]
pub struct CreateForm {
    pub body: Object,
    pub url: String,
}

// GeminiVideoGenerationParameters, in declaration order
const PARAMETER_FIELDS: [&str; 8] = [
    "aspectRatio",
    "durationSeconds",
    "resolution",
    "negativePrompt",
    "lastFrame",
    "referenceImages",
    "video",
    "personGeneration",
];

// transform_video_create_request: the image leaves the options, and the parameter model keeps its eight declared fields
pub fn create_request(
    input: CreateInput<'_>,
    mime: &dyn MimeReader,
) -> Result<CreateForm, StudioVeoError> {
    let CreateInput {
        prompt,
        api_base,
        mapped,
    } = input;
    let MappedParams { image, options } = mapped;
    let mut instance = Object::new();
    instance.insert("prompt".to_owned(), Json::Text(prompt.to_owned()));
    if let Some(chosen) = image
        && !matches!(&chosen, ImageInput::Json(Json::Null))
    {
        instance.insert(
            "image".to_owned(),
            Json::Object(gemini_image(chosen, mime)?),
        );
    }
    let mut body = Object::new();
    body.insert(
        "instances".to_owned(),
        Json::List(vec![Json::Object(instance)]),
    );
    body.insert(
        "parameters".to_owned(),
        Json::Object(typed_parameters(&options)?),
    );
    Ok(CreateForm {
        body,
        url: api_base.to_owned(),
    })
}

fn typed_parameters(options: &Object) -> Result<Object, StudioVeoError> {
    let mut parameters = Object::new();
    for name in PARAMETER_FIELDS {
        let Some(value) = options.get(name) else {
            continue;
        };
        if matches!(value, Json::Null) {
            continue;
        }
        if let Some(kept) = parameter_value(name, value)? {
            parameters.insert(name.to_owned(), kept);
        }
    }
    Ok(parameters)
}

fn parameter_value(name: &'static str, value: &Json) -> Result<Option<Json>, StudioVeoError> {
    let kept = match name {
        "aspectRatio" | "resolution" | "negativePrompt" | "personGeneration" => {
            text_parameter(name, value)?
        }
        "durationSeconds" => integer_parameter(name, value)?,
        "referenceImages" => list_parameter(name, value)?,
        "lastFrame" | "video" => value.clone(),
        _ => return Ok(None),
    };
    Ok(Some(kept))
}

fn text_parameter(name: &'static str, value: &Json) -> Result<Json, StudioVeoError> {
    match value {
        Json::Text(_) => Ok(value.clone()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(StudioVeoError::ParameterType {
                field: name,
                expected: "a string",
            })
        }
    }
}

fn integer_parameter(name: &'static str, value: &Json) -> Result<Json, StudioVeoError> {
    match value {
        Json::Number(number) if number.is_i64() || number.is_u64() => Ok(value.clone()),
        Json::Null
        | Json::Bool(_)
        | Json::Number(_)
        | Json::Text(_)
        | Json::List(_)
        | Json::Object(_) => Err(StudioVeoError::ParameterType {
            field: name,
            expected: "an integer",
        }),
    }
}

fn list_parameter(name: &'static str, value: &Json) -> Result<Json, StudioVeoError> {
    match value {
        Json::List(_) => Ok(value.clone()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            Err(StudioVeoError::ParameterType {
                field: name,
                expected: "a list",
            })
        }
    }
}
