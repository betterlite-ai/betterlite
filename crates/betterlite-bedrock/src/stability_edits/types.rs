use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageFiles;
use indexmap::IndexMap;

pub type EditOptions = IndexMap<String, EditValue>;

// A Python option value: JSON, bytes already read from a file or SDK object, or a list, which can hold bytes.
#[derive(Clone, Debug, PartialEq)]
pub enum EditValue {
    Json(Json),
    Bytes(Vec<u8>),
    List(Vec<Self>),
}

// The Python wheel reads a file-like image at its current position, so bytes arrive already read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageInput {
    Bytes(Vec<u8>),
    Text(String),
}

#[derive(Debug)]
pub struct EditedImageRequest {
    pub body: Json,
    pub files: ImageFiles,
}

impl From<Json> for EditValue {
    fn from(value: Json) -> Self {
        match value {
            Json::List(items) => Self::List(items.into_iter().map(Self::from).collect()),
            other => Self::Json(other),
        }
    }
}

impl EditValue {
    // None when the value holds bytes, which no JSON body can carry
    #[must_use]
    pub fn to_json(&self) -> Option<Json> {
        match self {
            Self::Json(value) => Some(value.clone()),
            Self::Bytes(_) => None,
            Self::List(items) => items
                .iter()
                .map(Self::to_json)
                .collect::<Option<Vec<Json>>>()
                .map(Json::List),
        }
    }
}

// Python's str() for the values a Stability message carries; a list or mapping prints as object.
#[must_use]
pub(crate) fn text_of(value: &Json) -> String {
    match value {
        Json::Null => "None".to_owned(),
        Json::Bool(flag) => (if *flag { "True" } else { "False" }).to_owned(),
        Json::Number(number) => number.to_string(),
        Json::Text(text) => text.clone(),
        Json::List(_) | Json::Object(_) => "object".to_owned(),
    }
}
