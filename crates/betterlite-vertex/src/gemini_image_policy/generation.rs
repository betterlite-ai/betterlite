use betterlite_prompts::arguments::json::{Json, Object};

use super::policy::supports_gemini_image_size;
use crate::grounding_requests::calculate_grounding_requests;

const CANDIDATE_KEYS: [&str; 4] = ["candidateCount", "candidate_count", "sampleCount", "n"];

#[must_use]
pub fn get_gemini_image_generation_config(
    model: &str,
    optional_params: &Object,
    image_size_support: Option<bool>,
) -> Object {
    let mut generation = Object::from([(
        "response_modalities".to_owned(),
        Json::List(vec![
            Json::Text("IMAGE".to_owned()),
            Json::Text("TEXT".to_owned()),
        ]),
    )]);
    let mut image_config = match optional_params.get("imageConfig") {
        Some(Json::Object(config)) => config.clone(),
        _ => Object::new(),
    };
    if !supports_gemini_image_size(model, image_size_support) {
        image_config.shift_remove("imageSize");
    }
    if !image_config.is_empty() {
        generation.insert("imageConfig".to_owned(), Json::Object(image_config));
    }
    if let Some(count) = CANDIDATE_KEYS.iter().find_map(|name| {
        optional_params
            .get(*name)
            .filter(|value| !matches!(value, Json::Null))
    }) {
        generation.insert("candidateCount".to_owned(), count.clone());
    }
    generation
}

#[must_use]
pub fn get_gemini_image_web_search_requests(response: &Object) -> Option<i64> {
    let mut grounding_metadata = Vec::new();
    if let Some(Json::List(candidates)) = response.get("candidates") {
        for candidate in candidates {
            let Json::Object(fields) = candidate else {
                continue;
            };
            match fields.get("groundingMetadata") {
                Some(Json::List(entries)) => grounding_metadata.extend(entries.iter().cloned()),
                Some(metadata @ Json::Object(_)) => grounding_metadata.push(metadata.clone()),
                _ => {}
            }
        }
    }
    calculate_grounding_requests(&grounding_metadata).web_search_requests
}
