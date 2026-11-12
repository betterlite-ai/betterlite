use betterlite_models::decisions::{Extra, Json};
use betterlite_tokenizer::count::error::CountError;
use serde::Serialize;

use crate::gemini_embedding_records::input::{EmbeddingInput, ResolvedFiles};
use crate::gemini_embedding_records::usage::TextTokenCounter;

pub(super) struct CharacterCount;

impl TextTokenCounter for CharacterCount {
    fn count_text(&self, _model: &str, text: &str) -> Result<u32, CountError> {
        u32::try_from(text.chars().count()).map_err(|_error| CountError::Overflow)
    }
}

pub(super) struct FailingCount;

impl TextTokenCounter for FailingCount {
    fn count_text(&self, _model: &str, _text: &str) -> Result<u32, CountError> {
        Err(CountError::Overflow)
    }
}

pub(super) fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

pub(super) fn object(source: &str) -> Extra {
    let Json::Object(fields) = json(source) else {
        panic!("an options fixture must be an object");
    };
    fields
}

pub(super) fn input(source: &str) -> EmbeddingInput {
    serde_json::from_str(source).unwrap()
}

pub(super) fn resolved_files(source: &str) -> ResolvedFiles {
    serde_json::from_str(source).unwrap()
}

pub(super) fn serialized<T: Serialize>(value: &T) -> Json {
    serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()
}
