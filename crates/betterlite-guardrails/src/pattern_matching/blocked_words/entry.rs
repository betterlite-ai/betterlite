use std::collections::BTreeMap;

use betterlite_config::sections::content_filter_records::{BlockedWord, ContentFilterAction};
use serde::Deserialize;
use serde::de::IgnoredAny;

use super::WordsError;

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum Entry {
    Mapping(BTreeMap<String, Field>),
    Other(IgnoredAny),
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum Field {
    Text(String),
    Null(()),
    Other(IgnoredAny),
}

pub(super) fn into_word(entry: Entry) -> Result<Option<BlockedWord>, WordsError> {
    let Entry::Mapping(mut fields) = entry else {
        return Ok(None);
    };
    let (Some(keyword), Some(action)) = (fields.remove("keyword"), fields.remove("action")) else {
        return Ok(None);
    };
    let Field::Text(text) = keyword else {
        return Err(WordsError::Field("keyword"));
    };
    let Field::Text(decision) = action else {
        return Err(WordsError::Field("action"));
    };
    let selected = match decision.as_str() {
        "BLOCK" => ContentFilterAction::Block,
        "MASK" => ContentFilterAction::Mask,
        _ => return Err(WordsError::Field("action")),
    };
    let description = match fields.remove("description") {
        Some(Field::Text(value)) => Some(value),
        Some(Field::Null(())) | None => None,
        Some(Field::Other(IgnoredAny)) => return Err(WordsError::Field("description")),
    };
    Ok(Some(BlockedWord {
        keyword: text,
        action: selected,
        description,
    }))
}
