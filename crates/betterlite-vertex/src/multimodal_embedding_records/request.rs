use std::collections::BTreeMap;

use betterlite_models::decisions::Extra;
use betterlite_models::decisions::Json;

use super::error::MultimodalEmbeddingError;
use super::media::classify_embedding_media;
use super::media::merged_instance;
use super::media::process_element;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 1] = ["dimensions"];
const INSTANCES: &str = "instances";
const OUTPUT_DIMENSIONALITY: &str = "outputDimensionality";

#[must_use]
pub fn map_openai_params(non_default_params: &Extra, mut optional_params: Extra) -> Extra {
    if let Some(dimensions) = non_default_params.get("dimensions") {
        optional_params.insert(String::from(OUTPUT_DIMENSIONALITY), dimensions.clone());
    }
    optional_params
}

#[must_use]
pub fn validate_environment(
    mut headers: BTreeMap<String, String>,
    api_key: &str,
) -> BTreeMap<String, String> {
    headers.insert(
        String::from("Content-Type"),
        String::from("application/json; charset=utf-8"),
    );
    headers.insert(String::from("Authorization"), format!("Bearer {api_key}"));
    headers
}

pub fn transform_request(
    input: &Json,
    optional_params: &Extra,
) -> Result<Json, MultimodalEmbeddingError> {
    let instances = match optional_params.get(INSTANCES) {
        Some(explicit) => explicit.clone(),
        None => transformed_instances(input)?,
    };
    let mut request = Extra::new();
    request.insert(String::from(INSTANCES), instances);
    if let Some(dimension) = optional_params.get(OUTPUT_DIMENSIONALITY) {
        let mut parameters = Extra::new();
        parameters.insert(String::from("dimension"), dimension.clone());
        request.insert(String::from("parameters"), Json::Object(parameters));
    }
    Ok(Json::Object(request))
}

fn transformed_instances(input: &Json) -> Result<Json, MultimodalEmbeddingError> {
    match input {
        Json::List(elements) => Ok(Json::List(process_list(elements)?)),
        Json::Text(text) => Ok(Json::List(vec![process_element(text)])),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => {
            Err(MultimodalEmbeddingError::UnsupportedInput)
        }
    }
}

fn process_list(elements: &[Json]) -> Result<Vec<Json>, MultimodalEmbeddingError> {
    let mut instances = Vec::with_capacity(elements.len());
    let mut index = 0;
    while let Some(element) = elements.get(index) {
        let next = elements.get(index.saturating_add(1));
        let (instance, step) = element_instance(element, next, index)?;
        instances.push(instance);
        index = index.saturating_add(step);
    }
    Ok(instances)
}

fn element_instance(
    element: &Json,
    next: Option<&Json>,
    index: usize,
) -> Result<(Json, usize), MultimodalEmbeddingError> {
    match element {
        Json::Text(text) => Ok(text_instance_step(text, next)),
        Json::Object(_) => Ok((element.clone(), 1)),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
            Err(MultimodalEmbeddingError::UnsupportedElement { index })
        }
    }
}

fn text_instance_step(text: &str, next: Option<&Json>) -> (Json, usize) {
    if classify_embedding_media(text).is_some() {
        return (process_element(text), 1);
    }
    let Some(Json::Text(media)) = next else {
        return (process_element(text), 1);
    };
    if media.is_empty() {
        return (process_element(text), 1);
    }
    let Some(kind) = classify_embedding_media(media) else {
        return (process_element(text), 1);
    };
    (merged_instance(text, media, kind), 2)
}
