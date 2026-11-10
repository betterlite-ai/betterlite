use thiserror::Error;

use crate::image_transport::json::JsonValue;

#[derive(Debug, Error)]
pub enum ImageDecodeError {
    #[error("the image operation response is not JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImageOperation {
    pub json: JsonValue,
    pub status: Option<JsonValue>,
    pub result: Option<JsonValue>,
}

pub fn decode_image_operation(body: &[u8]) -> Result<ImageOperation, ImageDecodeError> {
    let json: JsonValue =
        serde_json::from_slice(body).map_err(|source| ImageDecodeError::Json { source })?;
    Ok(ImageOperation {
        status: json.get("status").cloned(),
        result: json.get("result").cloned(),
        json,
    })
}
