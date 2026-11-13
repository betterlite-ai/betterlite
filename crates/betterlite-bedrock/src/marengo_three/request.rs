use serde::Serialize;
use serde_json::Number;

use super::fields::Segmentation;
use super::options::{EmbeddingOption, EmbeddingScope, EmbeddingType};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged, rename_all_fields = "camelCase")]
pub enum MediaSource {
    Inline { base64_string: String },
    Remote { s3_location: S3Location },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct S3Location {
    pub uri: String,
    pub bucket_owner: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextInput {
    pub input_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageInput {
    pub media_source: MediaSource,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimedMediaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_sec: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_sec: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segmentation: Option<Segmentation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_option: Option<Vec<EmbeddingOption>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_type: Option<Vec<EmbeddingType>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_scope: Option<Vec<EmbeddingScope>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimedMediaInput {
    pub media_source: MediaSource,
    #[serde(flatten)]
    pub options: TimedMediaOptions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextImageInput {
    pub input_text: String,
    pub media_source: MediaSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedMediaSource {
    pub name: String,
    pub media_type: &'static str,
    #[serde(flatten)]
    pub source: MediaSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_text: Option<String>,
    pub media_sources: Vec<NamedMediaSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "inputType", rename_all = "snake_case")]
pub enum RequestBody {
    Text { text: TextInput },
    Image { image: ImageInput },
    Video { video: TimedMediaInput },
    Audio { audio: TimedMediaInput },
    TextImage { text_image: TextImageInput },
    MultiInput { multi_input: MultiInput },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Marengo3Request {
    #[serde(rename = "inferenceId", skip_serializing_if = "Option::is_none")]
    pub inference_id: Option<String>,
    #[serde(flatten)]
    pub body: RequestBody,
}
