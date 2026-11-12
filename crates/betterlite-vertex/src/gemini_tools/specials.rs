use betterlite_prompts::arguments::json::{Json, Object};

use super::records::{LatLng, RetrievalConfig};

#[must_use]
pub fn transform_type(tool: &Object) -> Object {
    match tool.get("type") {
        Some(Json::Text(kind)) if kind == "computer_use" => {
            let mut transformed = Object::new();
            transformed.insert("computerUse".to_owned(), Json::Object(without_type(tool)));
            transformed
        }
        Some(Json::Text(kind)) if kind == "web_search" || kind == "web_search_preview" => {
            let mut transformed = Object::new();
            transformed.insert("googleSearch".to_owned(), Json::Object(Object::new()));
            transformed
        }
        Some(
            Json::Text(_)
            | Json::Bool(_)
            | Json::Number(_)
            | Json::List(_)
            | Json::Object(_)
            | Json::Null,
        ) => without_type(tool),
        None => tool.clone(),
    }
}

fn without_type(tool: &Object) -> Object {
    let mut stripped = Object::new();
    for (key, value) in tool {
        if key.as_str() != "type" {
            stripped.insert(key.clone(), value.clone());
        }
    }
    stripped
}

#[must_use]
pub fn extract_maps(config: &Object) -> (Object, Option<RetrievalConfig>) {
    let latitude = config
        .get("latitude")
        .filter(|value| !matches!(value, Json::Null));
    let longitude = config
        .get("longitude")
        .filter(|value| !matches!(value, Json::Null));
    let language_code = config
        .get("languageCode")
        .filter(|value| !matches!(value, Json::Null));
    let retrieval = latitude
        .zip(longitude)
        .map(|(latitude_json, longitude_json)| RetrievalConfig {
            lat_lng: LatLng {
                latitude: latitude_json.clone(),
                longitude: longitude_json.clone(),
            },
            language_code: language_code.cloned(),
        });
    let mut cleaned = Object::new();
    for (key, value) in config {
        if !matches!(key.as_str(), "latitude" | "longitude" | "languageCode") {
            cleaned.insert(key.clone(), value.clone());
        }
    }
    (cleaned, retrieval)
}

#[must_use]
pub fn transform_computer_use(config: &Object) -> Object {
    let mut transformed = Object::new();
    if let Some(environment) = config.get("environment") {
        match environment {
            Json::Text(text) if text == "browser" => {
                transformed.insert(
                    "environment".to_owned(),
                    Json::Text("ENVIRONMENT_BROWSER".to_owned()),
                );
            }
            Json::Text(text) if text == "unspecified" => {
                transformed.insert(
                    "environment".to_owned(),
                    Json::Text("ENVIRONMENT_UNSPECIFIED".to_owned()),
                );
            }
            Json::Text(text)
                if text == "ENVIRONMENT_BROWSER" || text == "ENVIRONMENT_UNSPECIFIED" =>
            {
                transformed.insert("environment".to_owned(), Json::Text(text.clone()));
            }
            Json::Text(_)
            | Json::Bool(_)
            | Json::Number(_)
            | Json::List(_)
            | Json::Object(_)
            | Json::Null => {
                tracing::info!(
                    "computer_use environment is not browser, unspecified, ENVIRONMENT_BROWSER or ENVIRONMENT_UNSPECIFIED; omitting it"
                );
            }
        }
    }
    if let Some(functions) = config
        .get("excluded_predefined_functions")
        .or_else(|| config.get("excludedPredefinedFunctions"))
    {
        transformed.insert("excludedPredefinedFunctions".to_owned(), functions.clone());
    }
    transformed
}
