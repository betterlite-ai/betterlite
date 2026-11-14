use betterlite_prompts::arguments::json::{Json, Object};

use super::error::GeminiImagePolicyError;
use super::tables::{aspect_ratio, gemini_image_size, parse_dimensions};

const HANDLED_PARAMS: [&str; 5] = ["n", "size", "imageConfig", "tools", "web_search_options"];

#[derive(Clone, Copy, Debug)]
pub struct ImageParamMapping<'request> {
    pub params: &'request Object,
    pub model: &'request str,
    pub supported_params: &'request [&'request str],
    pub optional_params: &'request Object,
    pub image_size_support: Option<bool>,
    pub parse_image_config_string: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageConfig {
    pub aspect_ratio: String,
    pub image_size: Option<String>,
}

#[must_use]
pub fn supports_gemini_image_size(model: &str, catalog_supports: Option<bool>) -> bool {
    if let Some(supported) = catalog_supports {
        return supported;
    }
    !model.contains("2.5-flash")
}

#[must_use]
pub fn is_gemini_image_model(model: &str) -> bool {
    model
        .split_once('/')
        .map_or(model, |(_, base_model)| base_model)
        .contains("gemini")
}

#[must_use]
pub fn map_size_to_image_config(
    size: &str,
    model: &str,
    image_size_support: Option<bool>,
) -> Option<ImageConfig> {
    let (width, height) = parse_dimensions(size)?;
    let image_size = if is_gemini_image_model(model) {
        supports_gemini_image_size(model, image_size_support)
            .then(|| gemini_image_size(width, height))
    } else {
        Some(gemini_image_size(width, height))
    };
    Some(ImageConfig {
        aspect_ratio: aspect_ratio(width, height).to_owned(),
        image_size: image_size.map(str::to_owned),
    })
}

pub fn map_image_params(mapping: &ImageParamMapping<'_>) -> Result<Object, GeminiImagePolicyError> {
    let filtered: Vec<(&String, &Json)> = mapping
        .params
        .iter()
        .filter(|(key, _)| mapping.supported_params.contains(&key.as_str()))
        .collect();
    let mut mapped = Object::new();
    if let Some((_, value)) = filtered.iter().find(|(key, _)| key.as_str() == "n")
        && !mapping.optional_params.contains_key("n")
    {
        mapped.insert("sampleCount".to_owned(), (*value).clone());
    }
    map_size(&filtered, mapping, &mut mapped);
    map_explicit_image_config(&filtered, mapping.parse_image_config_string, &mut mapped)?;
    for (key, value) in &filtered {
        if HANDLED_PARAMS.contains(&key.as_str())
            || mapping.optional_params.contains_key(key.as_str())
        {
            continue;
        }
        mapped.insert((*key).clone(), (*value).clone());
    }
    Ok(mapped)
}

fn map_size(filtered: &[(&String, &Json)], mapping: &ImageParamMapping<'_>, mapped: &mut Object) {
    let Some((_, Json::Text(size))) = filtered.iter().find(|(key, _)| key.as_str() == "size")
    else {
        return;
    };
    if mapping.optional_params.contains_key("size") {
        return;
    }
    let Some(config) = map_size_to_image_config(size, mapping.model, mapping.image_size_support)
    else {
        return;
    };
    if is_gemini_image_model(mapping.model) {
        let mut image_config =
            Object::from([("aspectRatio".to_owned(), Json::Text(config.aspect_ratio))]);
        if let Some(image_size) = config.image_size {
            image_config.insert("imageSize".to_owned(), Json::Text(image_size));
        }
        mapped.insert("imageConfig".to_owned(), Json::Object(image_config));
    } else {
        mapped.insert("aspectRatio".to_owned(), Json::Text(config.aspect_ratio));
        if let Some(image_size) = config.image_size {
            mapped.insert("imageSize".to_owned(), Json::Text(image_size));
        }
    }
}

fn map_explicit_image_config(
    filtered: &[(&String, &Json)],
    parse_string: bool,
    mapped: &mut Object,
) -> Result<(), GeminiImagePolicyError> {
    let Some((_, raw)) = filtered
        .iter()
        .find(|(key, _)| key.as_str() == "imageConfig")
    else {
        return Ok(());
    };
    let parsed = match *raw {
        Json::Text(text) if parse_string => {
            serde_json::from_str::<Json>(text).map_err(GeminiImagePolicyError::ImageConfigJson)?
        }
        Json::Null
        | Json::Bool(_)
        | Json::Number(_)
        | Json::Text(_)
        | Json::List(_)
        | Json::Object(_) => (*raw).clone(),
    };
    if let Json::Object(config) = parsed {
        mapped.insert("imageConfig".to_owned(), Json::Object(config));
    }
    Ok(())
}
