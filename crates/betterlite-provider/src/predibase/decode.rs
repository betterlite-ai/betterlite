use std::collections::BTreeMap;
use std::str;

use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;
use serde_json::Number;
use serde_with::rust::double_option;

use super::error::PredibaseError;

const CHAT_TEMPLATE_TOKENS: [&str; 5] = ["<|assistant|>", "<|system|>", "<|user|>", "<s>", "</s>"];

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum TokenLogprob {
    Number(Number),
    Null,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Token {
    pub logprob: TokenLogprob,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum NullableString {
    Text(String),
    Null,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Sequence {
    pub generated_text: String,
    pub finish_reason: NullableString,
    pub tokens: Vec<Token>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Details {
    // a missing entry is a KeyError in LiteLLM and a null entry maps to stop
    #[serde(default, with = "double_option")]
    pub finish_reason: Option<Option<String>>,
    #[serde(default)]
    pub tokens: Option<Vec<Token>>,
    #[serde(default)]
    pub best_of_sequences: Option<Vec<Sequence>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextReply {
    pub generated_text: String,
    pub details: Option<Details>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedReply {
    Error(String),
    Text(TextReply),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct RawReply {
    #[serde(default)]
    details: Option<Details>,
}

pub fn decode_predibase_reply(bytes: &[u8]) -> Result<DecodedReply, PredibaseError> {
    let shaped: Json = serde_json::from_slice(bytes).map_err(|_error| PredibaseError {
        status_code: 422,
        message: body_text(bytes),
        headers: BTreeMap::new(),
    })?;
    let Json::Object(fields) = shaped else {
        return Err(PredibaseError {
            status_code: 422,
            message: format!(
                "'completion_response' is not a dictionary - {}",
                body_text(bytes)
            ),
            headers: BTreeMap::new(),
        });
    };
    read_fields(&fields, bytes)
}

fn read_fields(fields: &Object, bytes: &[u8]) -> Result<DecodedReply, PredibaseError> {
    if let Some(error) = fields.get("error") {
        return Ok(DecodedReply::Error(error_message(error)?));
    }
    let Some(generated_text) = fields.get("generated_text") else {
        return Err(PredibaseError {
            status_code: 422,
            message: format!(
                "'generated_text' is not a key response dictionary - {}",
                body_text(bytes)
            ),
            headers: BTreeMap::new(),
        });
    };
    let Json::Text(text) = generated_text else {
        return Err(PredibaseError {
            status_code: 500,
            message: String::from("the predibase reply generated_text is not a string"),
            headers: BTreeMap::new(),
        });
    };
    check_shape(fields)?;
    let reply: RawReply = serde_json::from_slice(bytes).map_err(|source| PredibaseError {
        status_code: 500,
        message: format!("the predibase reply has an invalid field - {source}"),
        headers: BTreeMap::new(),
    })?;
    Ok(DecodedReply::Text(TextReply {
        generated_text: text.clone(),
        details: reply.details,
    }))
}

// serde also builds a struct from a JSON array, so the shape pass keeps each nested container an object
fn check_shape(fields: &Object) -> Result<(), PredibaseError> {
    let details = match fields.get("details") {
        None | Some(Json::Null) => return Ok(()),
        Some(Json::Object(details)) => details,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(wrong_object_shape("details"));
        }
    };
    object_entries(details.get("tokens"), "tokens")?;
    for sequence in object_entries(details.get("best_of_sequences"), "best_of_sequences")? {
        object_entries(sequence.get("tokens"), "tokens")?;
    }
    Ok(())
}

fn object_entries<'json>(
    value: Option<&'json Json>,
    field: &'static str,
) -> Result<Vec<&'json Object>, PredibaseError> {
    match value {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::List(items)) => items
            .iter()
            .map(|item| match item {
                Json::Object(object) => Ok(object),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                    Err(wrong_object_shape(field))
                }
            })
            .collect(),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            Err(wrong_object_shape(field))
        }
    }
}

fn wrong_object_shape(field: &'static str) -> PredibaseError {
    PredibaseError {
        status_code: 500,
        message: format!("the predibase reply {field} is not an object or a list of objects"),
        headers: BTreeMap::new(),
    }
}

fn error_message(value: &Json) -> Result<String, PredibaseError> {
    match value {
        Json::Text(text) => Ok(text.clone()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            serde_json::to_string(value).map_err(|_error| PredibaseError {
                status_code: 500,
                message: String::from("the predibase reply error does not serialize"),
                headers: BTreeMap::new(),
            })
        }
    }
}

fn body_text(bytes: &[u8]) -> String {
    let Ok(text) = str::from_utf8(bytes) else {
        return format!("{} bytes that are not UTF-8", bytes.len());
    };
    text.to_owned()
}

#[must_use]
pub fn output_parser(generated_text: &str) -> String {
    let mut text = generated_text.to_owned();
    for token in CHAT_TEMPLATE_TOKENS {
        if text.trim().starts_with(token) {
            text = text.replacen(token, "", 1);
        }
        if let Some(stripped) = text.strip_suffix(token) {
            text = stripped.to_owned();
        }
    }
    text
}
