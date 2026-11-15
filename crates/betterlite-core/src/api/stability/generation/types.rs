use betterlite_models::decisions::Json;
use serde::Serialize;
use serde_json::Number;

pub const DEFAULT_OUTPUT_FORMAT: &str = "png";
pub const DEFAULT_GENERATION_ENDPOINT: &str = "/v2beta/stable-image/generate/sd3";

// ast-grep-ignore: no-long-names
pub const OPENAI_SIZE_TO_STABILITY_ASPECT_RATIO: [(&str, &str); 5] = [
    ("1024x1024", "1:1"),
    ("1792x1024", "16:9"),
    ("1024x1792", "9:16"),
    ("512x512", "1:1"),
    ("256x256", "1:1"),
];

pub const STABILITY_GENERATION_MODELS: [(&str, &str); 9] = [
    ("sd3", "/v2beta/stable-image/generate/sd3"),
    ("sd3.5-large", "/v2beta/stable-image/generate/sd3"),
    ("sd3.5-large-turbo", "/v2beta/stable-image/generate/sd3"),
    ("sd3.5-medium", "/v2beta/stable-image/generate/sd3"),
    ("sd3-large", "/v2beta/stable-image/generate/sd3"),
    ("sd3-large-turbo", "/v2beta/stable-image/generate/sd3"),
    ("sd3-medium", "/v2beta/stable-image/generate/sd3"),
    ("stable-image-ultra", "/v2beta/stable-image/generate/ultra"),
    ("stable-image-core", "/v2beta/stable-image/generate/core"),
];

// LiteLLM's optional provider parameter: absent from the body, an explicit null, or a value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Nullable<T> {
    Null(()),
    Value(T),
    Missing,
}

impl<T> Nullable<T> {
    #[must_use]
    pub const fn is_missing(&self) -> bool {
        if matches!(self, Self::Missing) {
            return true;
        }
        false
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
// ast-grep-ignore: no-long-names
pub struct StabilityImageGenerationRequest {
    pub prompt: String,
    #[serde(skip_serializing_if = "Nullable::is_missing")]
    pub negative_prompt: Nullable<String>,
    #[serde(skip_serializing_if = "Nullable::is_missing")]
    pub aspect_ratio: Nullable<String>,
    #[serde(skip_serializing_if = "Nullable::is_missing")]
    pub seed: Nullable<i64>,
    #[serde(skip_serializing_if = "Nullable::is_missing")]
    pub output_format: Nullable<String>,
    #[serde(skip_serializing_if = "Nullable::is_missing")]
    pub model: Nullable<String>,
    #[serde(skip_serializing_if = "Nullable::is_missing")]
    pub mode: Nullable<String>,
    // the generation transform forwards no image; the edit configuration owns the file part
    #[serde(skip_serializing_if = "Nullable::is_missing")]
    pub image: Nullable<String>,
    #[serde(skip_serializing_if = "Nullable::is_missing")]
    pub strength: Nullable<Number>,
    #[serde(skip_serializing_if = "Nullable::is_missing")]
    pub style_preset: Nullable<String>,
}

#[must_use]
pub fn model_endpoint(model: &str) -> &'static str {
    let lowered = model.to_lowercase();
    let name = lowered.strip_prefix("stability/").unwrap_or(&lowered);
    STABILITY_GENERATION_MODELS
        .iter()
        .find_map(|(prefix, endpoint)| name.contains(prefix).then_some(*endpoint))
        .unwrap_or(DEFAULT_GENERATION_ENDPOINT)
}

#[must_use]
pub fn aspect_ratio(value: &Json) -> Option<&'static str> {
    let Json::Text(size) = value else {
        return None;
    };
    // ast-grep-ignore: no-long-names
    OPENAI_SIZE_TO_STABILITY_ASPECT_RATIO
        .iter()
        .find_map(|(key, ratio)| (*key == size.as_str()).then_some(*ratio))
}
