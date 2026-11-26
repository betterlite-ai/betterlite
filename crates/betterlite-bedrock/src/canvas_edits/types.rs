use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageFiles;
use indexmap::IndexMap;

pub type CanvasEditOptions = IndexMap<String, CanvasEditValue>;

// A Python option value: JSON, bytes already read from a file or SDK object, or a tuple FileTypes.
#[derive(Clone, Debug, PartialEq)]
pub enum CanvasEditValue {
    Json(Json),
    Bytes(Vec<u8>),
    Tuple,
}

// The Python wheel reads a file-like image at its current position, so bytes arrive already read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CanvasImageInput {
    Bytes(Vec<u8>),
    Text(String),
    Tuple,
}

#[derive(Debug)]
pub struct CanvasEditRequest {
    pub body: Json,
    pub files: ImageFiles,
}

impl From<Json> for CanvasEditValue {
    fn from(value: Json) -> Self {
        Self::Json(value)
    }
}

impl CanvasEditValue {
    // None when the value holds bytes or a tuple, which no JSON body can carry
    #[must_use]
    pub fn to_json(&self) -> Option<Json> {
        match self {
            Self::Json(value) => Some(value.clone()),
            Self::Bytes(_) | Self::Tuple => None,
        }
    }
}
