use betterlite_config::sections::compatible_image_credentials::Host;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use super::error::ImageOptionError;
use crate::api::image_contracts::records::ImageParams;

pub const OPENAI_STYLE_PREFIX: &str = "openai/";
pub const AIML_OPENAI_STYLE_PARAMS: &[&str] = &[
    "n",
    "size",
    "quality",
    "response_format",
    "output_format",
    "background",
    "moderation",
    "output_compression",
];
pub const AIML_FLUX_PARAMS: &[&str] = &["n", "response_format", "size"];
pub const COMETAPI_PARAMS: &[&str] = &["n", "quality", "response_format", "size", "style"];

// the prefix test is case-sensitive, so only a lowercase openai/ picks the OpenAI request schema
#[must_use]
pub fn supported_params(host: Host, model: &str) -> &'static [&'static str] {
    match host {
        Host::Aiml if model.starts_with(OPENAI_STYLE_PREFIX) => AIML_OPENAI_STYLE_PARAMS,
        Host::Aiml => AIML_FLUX_PARAMS,
        Host::CometApi => COMETAPI_PARAMS,
    }
}

// a key the caller already put in optional_params is skipped before support is checked
pub fn map_openai_params(
    host: Host,
    non_default_params: &ImageParams,
    optional_params: &mut ImageParams,
    model: &str,
    drop_params: bool,
) -> Result<(), ImageOptionError> {
    let supported = supported_params(host, model);
    let openai_style = host == Host::Aiml && model.starts_with(OPENAI_STYLE_PREFIX);
    for (name, value) in non_default_params {
        if optional_params.contains_key(name) {
            continue;
        }
        if !supported.contains(&name.as_str()) {
            if drop_params {
                continue;
            }
            return Err(ImageOptionError::Unsupported {
                name: name.clone(),
                model: model.to_owned(),
            });
        }
        if openai_style {
            optional_params.insert(name.clone(), value.clone());
            continue;
        }
        match host {
            Host::Aiml => map_aiml_option(name, value, optional_params)?,
            Host::CometApi => {
                optional_params.insert(name.clone(), value.clone());
            }
        }
    }
    Ok(())
}

fn map_aiml_option(
    name: &str,
    value: &Json,
    optional_params: &mut ImageParams,
) -> Result<(), ImageOptionError> {
    let renamed = match name {
        "n" => (String::from("num_images"), value.clone()),
        "response_format" => (String::from("output_format"), value.clone()),
        "size" => (String::from("image_size"), image_size(value)?),
        _ => (name.to_owned(), value.clone()),
    };
    optional_params.insert(renamed.0, renamed.1);
    Ok(())
}

// only a string holding x is split, and it must give exactly two integers; every other value passes through
fn image_size(value: &Json) -> Result<Json, ImageOptionError> {
    let Json::Text(text) = value else {
        return Ok(value.clone());
    };
    if !text.contains('x') {
        return Ok(value.clone());
    }
    let parts: Vec<&str> = text.split('x').collect();
    let [width, height] = parts.as_slice() else {
        return Err(ImageOptionError::Size { size: text.clone() });
    };
    let Some(parsed_width) = width.trim().parse::<i64>().ok() else {
        return Err(ImageOptionError::Size { size: text.clone() });
    };
    let Some(parsed_height) = height.trim().parse::<i64>().ok() else {
        return Err(ImageOptionError::Size { size: text.clone() });
    };
    let mut image_size = IndexMap::new();
    image_size.insert(
        String::from("width"),
        Json::Number(Number::from(parsed_width)),
    );
    image_size.insert(
        String::from("height"),
        Json::Number(Number::from(parsed_height)),
    );
    Ok(Json::Object(image_size))
}

// Flux-style AIML builds its TypedDict around prompt and model, so an option with either name fails instead of winning
pub fn request_body(
    host: Host,
    model: &str,
    prompt: &str,
    optional_params: &ImageParams,
) -> Result<IndexMap<String, Json>, ImageOptionError> {
    if host == Host::Aiml && !model.starts_with(OPENAI_STYLE_PREFIX) {
        let duplicate = ["model", "prompt"]
            .into_iter()
            .find(|name| optional_params.contains_key(*name));
        if let Some(name) = duplicate {
            return Err(ImageOptionError::Duplicate {
                name: name.to_owned(),
            });
        }
    }
    let mut body = IndexMap::from([
        (String::from("model"), Json::Text(model.to_owned())),
        (String::from("prompt"), Json::Text(prompt.to_owned())),
    ]);
    body.extend(optional_params.clone());
    Ok(body)
}
