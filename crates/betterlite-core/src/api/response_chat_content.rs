pub mod annotations;
pub mod content;
pub mod debug;
pub mod image_file;
pub mod tool_choice;

use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Projection {
    pub value: Value,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatContent {
    Text(String),
    Known {
        kind: String,
        fields: Map<String, Value>,
    },
    Unknown(Value),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ContentError {
    #[error("Invalid image URL: {value}")]
    InvalidImageUrl { value: String },
}

mod tests;
