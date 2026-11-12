use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_models::decisions::Extra;
use betterlite_models::decisions::Json;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;

use super::error::MultimodalEmbeddingError as Error;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 1] = ["dimensions"];
// Longer inputs stay plain text, so classifying request data never decodes without a bound
pub const MEDIA_TEXT_LIMIT: usize = 0x0200_0000;
const INSTANCES: &str = "instances";
const DIMENSIONALITY: &str = "outputDimensionality";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaKind {
    Base64Image,
    GcsImage,
    GcsVideo,
}

#[must_use]
pub fn classify_embedding_media(input: &str) -> Option<MediaKind> {
    if input.contains("gs://") {
        return Some(if input.contains("mp4") {
            MediaKind::GcsVideo
        } else {
            MediaKind::GcsImage
        });
    }
    (input.len() <= MEDIA_TEXT_LIMIT && is_data_base64(input)).then_some(MediaKind::Base64Image)
}

#[must_use]
pub fn element(caption: Option<&str>, media: &str, kind: Option<MediaKind>) -> Json {
    let mut fields = Extra::new();
    if let Some(text) = caption {
        fields.insert(String::from("text"), Json::Text(text.to_owned()));
    }
    let (name, payload) = match kind {
        Some(MediaKind::GcsVideo) => (
            "video",
            Json::Object(Extra::from([(
                String::from("gcsUri"),
                Json::Text(media.to_owned()),
            )])),
        ),
        Some(MediaKind::GcsImage) => (
            "image",
            Json::Object(Extra::from([(
                String::from("gcsUri"),
                Json::Text(media.to_owned()),
            )])),
        ),
        Some(MediaKind::Base64Image) => ("image", Json::Object(base64_fields(media))),
        None => ("text", Json::Text(media.to_owned())),
    };
    fields.insert(String::from(name), payload);
    Json::Object(fields)
}

fn base64_fields(input: &str) -> Extra {
    let payload = input.split(',').nth(1).unwrap_or(input);
    Extra::from([(
        String::from("bytesBase64Encoded"),
        Json::Text(payload.to_owned()),
    )])
}

// Python's is_base64_encoded: a data: prefix, the second comma segment, strict decoding and a canonical re-encoding
fn is_data_base64(input: &str) -> bool {
    if !input.starts_with("data:") {
        return false;
    }
    let Some(payload) = input.split(',').nth(1) else {
        return false;
    };
    let Ok(decoded) = STANDARD.decode(payload) else {
        return false;
    };
    STANDARD.encode(decoded) == payload
}

#[must_use]
pub fn map_openai_params(non_default_params: &Extra, mut optional_params: Extra) -> Extra {
    if let Some(dimensions) = non_default_params.get("dimensions") {
        optional_params.insert(String::from(DIMENSIONALITY), dimensions.clone());
    }
    optional_params
}

#[must_use]
pub fn validate_environment(
    mut headers: BTreeMap<String, String>,
    api_key: &SecretString,
) -> BTreeMap<String, String> {
    headers.insert(
        String::from("Content-Type"),
        String::from("application/json; charset=utf-8"),
    );
    headers.insert(
        String::from("Authorization"),
        format!("Bearer {}", api_key.expose_secret()),
    );
    headers
}

pub fn transform_request(input: &Json, optional_params: &Extra) -> Result<Json, Error> {
    let instances = match optional_params.get(INSTANCES) {
        Some(explicit) => explicit.clone(),
        None => match input {
            Json::List(elements) => Json::List(process_list(elements)?),
            Json::Text(text) => {
                Json::List(vec![element(None, text, classify_embedding_media(text))])
            }
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => {
                return Err(Error::UnsupportedInput);
            }
        },
    };
    let mut request = Extra::new();
    request.insert(String::from(INSTANCES), instances);
    if let Some(dimension) = optional_params.get(DIMENSIONALITY) {
        request.insert(
            String::from("parameters"),
            Json::Object(Extra::from([(
                String::from("dimension"),
                dimension.clone(),
            )])),
        );
    }
    Ok(Json::Object(request))
}

fn process_list(elements: &[Json]) -> Result<Vec<Json>, Error> {
    let mut instances = Vec::with_capacity(elements.len());
    let mut index = 0;
    while let Some(element) = elements.get(index) {
        let next = elements.get(index.saturating_add(1));
        match element {
            Json::Object(_) => {
                instances.push(element.clone());
                index = index.saturating_add(1);
            }
            Json::Text(text) => {
                let (instance, step) = text_instance(text, next);
                instances.push(instance);
                index = index.saturating_add(step);
            }
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
                return Err(Error::UnsupportedElement { index });
            }
        }
    }
    Ok(instances)
}

fn text_instance(text: &str, next: Option<&Json>) -> (Json, usize) {
    let current = classify_embedding_media(text);
    if current.is_some() {
        return (element(None, text, current), 1);
    }
    match next {
        Some(Json::Text(media)) if !media.is_empty() => classify_embedding_media(media)
            .map_or_else(
                || (element(None, text, None), 1),
                |kind| (element(Some(text), media, Some(kind)), 2),
            ),
        _ => (element(None, text, None), 1),
    }
}
