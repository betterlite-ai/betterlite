use serde::Deserialize;

/// Titan's response body: a missing `images` key fails at deserialization, mirroring
/// Python's `response_dict["images"]` `KeyError`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TitanResponse {
    pub images: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Artifact {
    pub base64: String,
}

/// Stability 1's response body: missing `artifacts`, or an artifact missing `base64`,
/// both fail at deserialization the same way.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Stability1Response {
    pub artifacts: Vec<Artifact>,
}

/// Stability 3 and Nova default an absent `images` field to an empty list.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct ImagesOnlyResponse {
    #[serde(default)]
    pub images: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum FinishReason {
    Text(String),
    Boolean(bool),
    Other(i64),
}

impl FinishReason {
    #[must_use]
    pub const fn truthy(&self) -> bool {
        match self {
            Self::Text(text) => !text.is_empty(),
            Self::Boolean(flag) => *flag,
            Self::Other(value) => *value != 0,
        }
    }

    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            Self::Boolean(_) | Self::Other(_) => None,
        }
    }

    #[must_use]
    pub const fn python_type_name(&self) -> &'static str {
        match self {
            Self::Text(_) => "str",
            Self::Boolean(_) => "bool",
            Self::Other(_) => "int",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct Stability3Response {
    #[serde(default)]
    pub images: Vec<String>,
    #[serde(default)]
    pub finish_reasons: Vec<FinishReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageData {
    pub b64_json: String,
}

/// Stands in for the `OpenAI` `ImageResponse` object that the out-of-scope dispatcher
/// (B634) builds; `created` represents any other field that image mapping must not touch.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModelResponse {
    pub created: Option<i64>,
    pub data: Vec<ImageData>,
}

fn to_data(images: Vec<String>) -> Vec<ImageData> {
    images
        .into_iter()
        .map(|value| ImageData { b64_json: value })
        .collect()
}

#[must_use]
pub fn titan_images(response: &TitanResponse) -> Vec<ImageData> {
    to_data(response.images.clone())
}

#[must_use]
pub fn stability1_images(response: &Stability1Response) -> Vec<ImageData> {
    let values = response
        .artifacts
        .iter()
        .map(|artifact| artifact.base64.clone())
        .collect();
    to_data(values)
}

#[must_use]
pub fn optional_images(response: &ImagesOnlyResponse) -> Vec<ImageData> {
    to_data(response.images.clone())
}

pub fn apply_images(model_response: &mut ModelResponse, data: Vec<ImageData>) {
    model_response.data = data;
}
