use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use decode::line_code_points;
use error::RecordJsonError;
use parse::parse;
use text::RecordText;

pub mod decode;
pub mod dumps;
pub mod error;
pub mod parse;
pub mod text;

// json.loads keeps the first position of a repeated key and its last value, as IndexMap::insert does
pub type RecordObject = IndexMap<RecordText, RecordJson>;

// What json.loads gives a batch record: a number keeps its literal, NaN and Infinity included, and text keeps lone surrogates
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordJson {
    Null,
    Bool(bool),
    Number(String),
    Text(RecordText),
    List(Vec<Self>),
    Object(RecordObject),
}

impl RecordJson {
    #[must_use]
    pub const fn as_text(&self) -> Option<&RecordText> {
        match self {
            Self::Text(text) => Some(text),
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::List(_) | Self::Object(_) => None,
        }
    }

    #[must_use]
    pub const fn as_object(&self) -> Option<&RecordObject> {
        match self {
            Self::Object(members) => Some(members),
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::Text(_) | Self::List(_) => None,
        }
    }
}

impl From<&Json> for RecordJson {
    fn from(value: &Json) -> Self {
        match value {
            Json::Null => Self::Null,
            Json::Bool(flag) => Self::Bool(*flag),
            Json::Number(number) => Self::Number(number.to_string()),
            Json::Text(text) => Self::Text(RecordText::from(text.as_str())),
            Json::List(items) => Self::List(items.iter().map(Self::from).collect()),
            Json::Object(members) => Self::Object(
                members
                    .iter()
                    .map(|(key, item)| (RecordText::from(key.as_str()), Self::from(item)))
                    .collect(),
            ),
        }
    }
}

// json.loads(raw_line) on the bytes the upload validation already accepted as one JSON object
pub fn load_record(raw_line: &[u8]) -> Result<RecordObject, RecordJsonError> {
    let code_points = line_code_points(raw_line).ok_or(RecordJsonError::Encoding)?;
    match parse(&code_points)? {
        RecordJson::Object(payload) => Ok(payload),
        RecordJson::Null
        | RecordJson::Bool(_)
        | RecordJson::Number(_)
        | RecordJson::Text(_)
        | RecordJson::List(_) => Err(RecordJsonError::NotObject),
    }
}
