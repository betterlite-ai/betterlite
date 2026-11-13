use betterlite_prompts::arguments::json::{Json, Object};

use super::error::GeminiImagePolicyError;
use super::tables::{aspect_ratio, gemini_image_size, parse_dimensions};

const HANDLED_PARAMS: [&str; 5] = ["n", "size", "imageConfig", "tools", "web_search_options"];

#[derive(Clone, Copy, Debug)]
pub struct ImageParamMapping<'a> {
    pub params: &'a Object,
    pub model: &'a str,
    pub supported_params: &'a [&'a str],
    pub optional_params: &'a Object,
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
    catalog_supports.unwrap_or_else(|| !model.contains("2.5-flash"))
}

#[must_use]
pub fn is_gemini_image_model(model: &str) -> bool {
    model
        .split_once('/')
        .map_or(model, |(_, base_model)| base_model)
        .contains("gemini")
}

#[must_use]
pub fn map_openai_size_to_gemini_image_config(
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

pub fn map_openai_image_params_to_gemini(
    mapping: &ImageParamMapping<'_>,
) -> Result<Object, GeminiImagePolicyError> {
    let filtered = filtered_params(mapping.params, mapping.supported_params);
    let mut mapped = Object::new();
    map_sample_count(&filtered, mapping.optional_params, &mut mapped);
    map_size(&filtered, mapping, &mut mapped);
    map_explicit_image_config(&filtered, mapping.parse_image_config_string, &mut mapped)?;
    copy_remaining(&filtered, mapping.optional_params, &mut mapped);
    Ok(mapped)
}

fn filtered_params<'params>(
    params: &'params Object,
    supported_params: &[&str],
) -> Vec<(&'params String, &'params Json)> {
    params
        .iter()
        .filter(|(key, _)| supported_params.iter().any(|name| *name == key.as_str()))
        .collect()
}

fn map_sample_count(filtered: &[(&String, &Json)], optional: &Object, mapped: &mut Object) {
    let Some((_, value)) = filtered.iter().find(|(key, _)| key.as_str() == "n") else {
        return;
    };
    if !optional.contains_key("n") {
        mapped.insert("sampleCount".to_owned(), (*value).clone());
    }
}

fn map_size(filtered: &[(&String, &Json)], mapping: &ImageParamMapping<'_>, mapped: &mut Object) {
    let Some((_, Json::Text(size))) = filtered.iter().find(|(key, _)| key.as_str() == "size")
    else {
        return;
    };
    if mapping.optional_params.contains_key("size") {
        return;
    }
    let Some(config) =
        map_openai_size_to_gemini_image_config(size, mapping.model, mapping.image_size_support)
    else {
        return;
    };
    if is_gemini_image_model(mapping.model) {
        mapped.insert("imageConfig".to_owned(), image_config_json(&config));
    } else {
        mapped.insert("aspectRatio".to_owned(), Json::Text(config.aspect_ratio));
        if let Some(image_size) = config.image_size {
            mapped.insert("imageSize".to_owned(), Json::Text(image_size));
        }
    }
}

fn image_config_json(config: &ImageConfig) -> Json {
    let mut object = Object::from([(
        "aspectRatio".to_owned(),
        Json::Text(config.aspect_ratio.clone()),
    )]);
    if let Some(image_size) = &config.image_size {
        object.insert("imageSize".to_owned(), Json::Text(image_size.clone()));
    }
    Json::Object(object)
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
        Json::Text(text) if parse_string => serde_json::from_str::<Json>(text)
            .map_err(|source| GeminiImagePolicyError::ImageConfigJson { source })?,
        value => value.clone(),
    };
    if let Json::Object(config) = parsed {
        mapped.insert("imageConfig".to_owned(), Json::Object(config));
    }
    Ok(())
}

fn copy_remaining(filtered: &[(&String, &Json)], optional: &Object, mapped: &mut Object) {
    for (key, value) in filtered {
        if HANDLED_PARAMS.contains(&key.as_str()) || optional.contains_key(key.as_str()) {
            continue;
        }
        mapped.insert((*key).clone(), (*value).clone());
    }
}
