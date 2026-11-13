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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
// ast-grep-ignore: no-long-names
pub struct StabilityImageGenerationRequest {
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub negative_prompt: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<Option<i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_format: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<Option<String>>,
    // the generation transform forwards no image; the edit configuration owns the file part
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strength: Option<Option<Number>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style_preset: Option<Option<String>>,
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
