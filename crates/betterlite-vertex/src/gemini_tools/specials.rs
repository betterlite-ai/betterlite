use betterlite_prompts::arguments::json::{Json, Object};

use super::records::{LatLng, RetrievalConfig};

#[must_use]
pub fn transform_type(tool: &Object) -> Object {
    match tool.get("type") {
        Some(Json::Text(kind)) if kind == "computer_use" => {
            Object::from([("computerUse".to_owned(), Json::Object(without_type(tool)))])
        }
        Some(Json::Text(kind)) if kind == "web_search" || kind == "web_search_preview" => {
            Object::from([("googleSearch".to_owned(), Json::Object(Object::new()))])
        }
        Some(_) => without_type(tool),
        None => tool.clone(),
    }
}
fn without_type(tool: &Object) -> Object {
    tool.iter()
        .filter(|(key, _)| key.as_str() != "type")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
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
    let mut cleaned = config.clone();
    cleaned.retain(|key, _| !matches!(key.as_str(), "latitude" | "longitude" | "languageCode"));
    (cleaned, retrieval)
}
#[must_use]
pub fn transform_computer_use(config: &Object) -> Object {
    let mut transformed = Object::new();
    if let Some(environment) = config.get("environment") {
        if let Some(value) = environment.as_str().and_then(environment_value) {
            transformed.insert("environment".to_owned(), Json::Text(value.to_owned()));
        } else {
            tracing::info!(
                "computer_use environment is not browser, unspecified, ENVIRONMENT_BROWSER or ENVIRONMENT_UNSPECIFIED; omitting it"
            );
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
fn environment_value(value: &str) -> Option<&'static str> {
    match value {
        "browser" | "ENVIRONMENT_BROWSER" => Some("ENVIRONMENT_BROWSER"),
        "unspecified" | "ENVIRONMENT_UNSPECIFIED" => Some("ENVIRONMENT_UNSPECIFIED"),
        _ => None,
    }
}
