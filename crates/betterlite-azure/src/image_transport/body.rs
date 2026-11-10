use url::Url;

use crate::image_transport::json::{JsonMap, JsonValue};

// dall-e-3 starts from 2023-12-01-preview, so LiteLLM polls only the five versions before it
pub(crate) const PREVIEW_API_VERSIONS: [&str; 5] = [
    "2023-06-01-preview",
    "2023-07-01-preview",
    "2023-08-01-preview",
    "2023-09-01-preview",
    "2023-10-01-preview",
];

// httpx's copy_with(path=...) keeps query, fragment, host, credentials and port and replaces the whole path
pub fn modify_url(original_url: &str, new_path: &str) -> Result<String, url::ParseError> {
    let mut url = Url::parse(original_url)?;
    url.set_path(new_path);
    Ok(url.to_string())
}

#[must_use]
pub fn image_generation_body(
    api_base: &str,
    data: &JsonMap,
    deployment_name: Option<&str>,
) -> JsonMap {
    if api_base.contains("images/generations") && api_base.contains("/openai/deployments/") {
        return without_model(data);
    }
    match deployment_name.filter(|name| !name.is_empty()) {
        Some(name) if api_base.contains("/openai/v1/images/") => with_model(data, name),
        _ => data.clone(),
    }
}

fn without_model(data: &JsonMap) -> JsonMap {
    data.iter()
        .filter(|(key, _)| key.as_str() != "model")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn with_model(data: &JsonMap, deployment_name: &str) -> JsonMap {
    let mut body = data.clone();
    body.insert(
        "model".to_owned(),
        JsonValue::String(deployment_name.to_owned()),
    );
    body
}
