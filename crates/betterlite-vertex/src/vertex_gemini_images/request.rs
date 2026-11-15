use betterlite_prompts::arguments::json::{Json, Object};

use super::error::GeminiImageError;

// ast-grep-ignore: no-long-names
pub fn transform_image_generation_request(
    prompt: &str,
    optional_params: &Object,
) -> Result<Object, GeminiImageError> {
    let mut generation_config = Object::new();
    generation_config.insert(
        "responseModalities".to_owned(),
        Json::List(vec![Json::Text("IMAGE".to_owned())]),
    );
    let image_config = merged_image_config(optional_params)?;
    if !image_config.is_empty() {
        generation_config.insert("imageConfig".to_owned(), Json::Object(image_config));
    }
    if let Some(count) = optional_params
        .get("candidate_count")
        .or_else(|| optional_params.get("n"))
    {
        generation_config.insert("candidateCount".to_owned(), count.clone());
    }
    let mut part = Object::new();
    part.insert("text".to_owned(), Json::Text(prompt.to_owned()));
    let mut turn = Object::new();
    turn.insert("role".to_owned(), Json::Text("user".to_owned()));
    turn.insert("parts".to_owned(), Json::List(vec![Json::Object(part)]));
    let mut body = Object::new();
    body.insert("contents".to_owned(), Json::List(vec![Json::Object(turn)]));
    body.insert(
        "generationConfig".to_owned(),
        Json::Object(generation_config),
    );
    // LiteLLM copies only a truthy tools or toolConfig into the body
    for key in ["tools", "toolConfig"] {
        if let Some(value) = optional_params
            .get(key)
            .filter(|candidate| candidate.is_truthy())
        {
            body.insert(key.to_owned(), value.clone());
        }
    }
    Ok(body)
}

// the user's imageConfig seeds the object and the flat params overwrite it for backward compat
fn merged_image_config(optional_params: &Object) -> Result<Object, GeminiImageError> {
    let mut image_config = match optional_params.get("imageConfig") {
        None | Some(Json::Null) => Object::new(),
        Some(Json::Object(config)) => config.clone(),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(GeminiImageError::FieldType {
                param: "imageConfig",
                expected: "an object",
            });
        }
    };
    if let Some(ratio) = optional_params
        .get("aspectRatio")
        .or_else(|| optional_params.get("aspect_ratio"))
    {
        image_config.insert("aspectRatio".to_owned(), ratio.clone());
    }
    if let Some(size) = optional_params
        .get("imageSize")
        .or_else(|| optional_params.get("image_size"))
    {
        image_config.insert("imageSize".to_owned(), size.clone());
    }
    Ok(image_config)
}
