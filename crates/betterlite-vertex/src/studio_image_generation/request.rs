use betterlite_prompts::arguments::json::{Json, Object};

use super::error::StudioImageError;
use crate::gemini_image_policy::generation::image_generation_config;
use crate::gemini_image_policy::policy::is_gemini_image_model;

// the optional fields of GeminiImageGenerationParameters besides sampleCount, in the order the class declares them
const TEXT_PARAMETERS: [&str; 12] = [
    "aspectRatio",
    "imageSize",
    "personGeneration",
    "background",
    "input_fidelity",
    "moderation",
    "output_compression",
    "output_format",
    "quality",
    "response_format",
    "style",
    "user",
];

#[derive(Clone, Copy)]
enum ParameterKind {
    Integer,
    Text,
}

// ast-grep-ignore: no-long-names
pub fn transform_image_generation_request(
    model: &str,
    prompt: &str,
    optional_params: &Object,
    image_size_support: Option<bool>,
) -> Result<Object, StudioImageError> {
    if is_gemini_image_model(model) {
        return Ok(gemini_request(
            model,
            prompt,
            optional_params,
            image_size_support,
        ));
    }
    imagen_request(prompt, optional_params)
}

fn gemini_request(
    model: &str,
    prompt: &str,
    optional_params: &Object,
    image_size_support: Option<bool>,
) -> Object {
    let generation = image_generation_config(model, optional_params, image_size_support);
    let part = Object::from([("text".to_owned(), Json::Text(prompt.to_owned()))]);
    let turn = Object::from([("parts".to_owned(), Json::List(vec![Json::Object(part)]))]);
    let mut body = Object::from([
        ("contents".to_owned(), Json::List(vec![Json::Object(turn)])),
        ("generationConfig".to_owned(), Json::Object(generation)),
    ]);
    // LiteLLM copies only a truthy tools or toolConfig into the body
    for key in ["tools", "toolConfig"] {
        if let Some(value) = optional_params
            .get(key)
            .filter(|candidate| candidate.is_truthy())
        {
            body.insert(key.to_owned(), value.clone());
        }
    }
    body
}

fn imagen_request(prompt: &str, optional_params: &Object) -> Result<Object, StudioImageError> {
    let instance = Object::from([("prompt".to_owned(), Json::Text(prompt.to_owned()))]);
    let parameters = imagen_parameters(optional_params)?;
    Ok(Object::from([
        (
            "instances".to_owned(),
            Json::List(vec![Json::Object(instance)]),
        ),
        ("parameters".to_owned(), Json::Object(parameters)),
    ]))
}

// GeminiImageGenerationParameters ignores unknown keys, keeps an explicit zero or empty string and omits a null
fn imagen_parameters(optional_params: &Object) -> Result<Object, StudioImageError> {
    let mut parameters = Object::new();
    if let Some(value) = optional_params.get("sampleCount") {
        insert_parameter(
            &mut parameters,
            "sampleCount",
            value,
            ParameterKind::Integer,
        )?;
    }
    for name in TEXT_PARAMETERS {
        if let Some(value) = optional_params.get(name) {
            insert_parameter(&mut parameters, name, value, ParameterKind::Text)?;
        }
    }
    Ok(parameters)
}

fn insert_parameter(
    parameters: &mut Object,
    name: &'static str,
    value: &Json,
    kind: ParameterKind,
) -> Result<(), StudioImageError> {
    if matches!(value, Json::Null) {
        return Ok(());
    }
    if !kind.fits(value) {
        return Err(StudioImageError::FieldType {
            param: name,
            expected: kind.expected(),
        });
    }
    parameters.insert(name.to_owned(), value.clone());
    Ok(())
}

impl ParameterKind {
    fn fits(self, value: &Json) -> bool {
        match (self, value) {
            (Self::Integer, Json::Number(number)) => number.as_i64().is_some(),
            (Self::Text, Json::Text(_)) => true,
            (Self::Integer | Self::Text, _) => false,
        }
    }

    const fn expected(self) -> &'static str {
        match self {
            Self::Integer => "an integer",
            Self::Text => "a string",
        }
    }
}
