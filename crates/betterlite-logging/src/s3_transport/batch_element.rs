use serde::{Deserialize, Serialize};

mod tests;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub enum ContentType {
    #[default]
    #[serde(rename = "application/json")]
    Json,
    #[serde(rename = "application/x-ndjson")]
    Ndjson,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct S3BatchLoggingElement<Payload> {
    pub payload: Payload,
    pub s3_object_key: String,
    pub s3_object_download_filename: String,
    // None means the upload serializes the payload itself
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub content_type: ContentType,
    // unix seconds of the first failed upload, None until one fails
    #[serde(default)]
    pub retrying_since: Option<f64>,
}
