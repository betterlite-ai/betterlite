use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_models::decisions::Json;
use sha2::{Digest as _, Sha256};

use super::client::SendFailure;
use super::video_config::{Params, RequestFile};

// LiteLLM's DEFAULT_MAX_RECURSE_DEPTH, which its environment can change and a betterlite config cannot
pub(crate) const MAX_RECURSE_DEPTH: usize = 100;

// The parts httpx builds for data= and files=: the form fields in order, then one part per file tuple
pub(crate) fn with_files(
    data: &Params,
    files: &[RequestFile],
) -> Result<Vec<MultipartPart>, SendFailure> {
    let mut parts = data_parts(data)?;
    parts.extend(files.iter().map(file_part));
    Ok(parts)
}

// FileField renders a filename only when it is truthy, and guesses a missing type from it
fn file_part(file: &RequestFile) -> MultipartPart {
    let file_name = file.filename.clone().filter(|name| !name.is_empty());
    let content_type = file
        .content_type
        .clone()
        .or_else(|| file_name.as_deref().map(guessed_content_type));
    MultipartPart {
        name: file.field.clone(),
        file_name,
        content_type,
        data: file.content.clone(),
    }
}

fn guessed_content_type(filename: &str) -> String {
    mime_guess::from_path(filename)
        .first_raw()
        .unwrap_or("application/octet-stream")
        .to_owned()
}

// serialize_multipart_form_fields: nested names flatten into key[subkey] and key[], so a file-less request still goes out as multipart
pub(crate) fn flattened(data: &Params) -> Result<Vec<MultipartPart>, SendFailure> {
    let mut fields = Vec::new();
    for (key, value) in data {
        flatten(key, value, 0, &mut fields)?;
    }
    Ok(fields
        .into_iter()
        .map(|(name, text)| text_part(name, text))
        .collect())
}

// A boundary derived from the parts, so a body carrying it would have to hold a hash of itself
#[must_use]
pub(crate) fn boundary(parts: &[MultipartPart]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        absorb(&mut hasher, part.name.as_bytes());
        absorb(&mut hasher, part.data.as_slice());
        if let Some(name) = &part.file_name {
            absorb(&mut hasher, name.as_bytes());
        }
        if let Some(content_type) = &part.content_type {
            absorb(&mut hasher, content_type.as_bytes());
        }
    }
    hex::encode(hasher.finalize())
}

fn absorb(hasher: &mut Sha256, bytes: &[u8]) {
    let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    hasher.update(length.to_be_bytes());
    hasher.update(bytes);
}

// httpx's DataField takes a primitive or a list of them, and rejects a nested object with Invalid type for value
fn data_parts(data: &Params) -> Result<Vec<MultipartPart>, SendFailure> {
    let mut parts = Vec::new();
    for (name, value) in data {
        if let Json::List(items) = value {
            for item in items {
                let Some(text) = primitive(item) else {
                    return Err(primitive_error(name));
                };
                parts.push(text_part(name.clone(), text));
            }
            continue;
        }
        let Some(text) = primitive(value) else {
            return Err(primitive_error(name));
        };
        parts.push(text_part(name.clone(), text));
    }
    Ok(parts)
}

// _flatten_form_field: objects and lists become key[subkey] and key[], and null or an empty string drops the field
fn flatten(
    key: &str,
    value: &Json,
    depth: usize,
    fields: &mut Vec<(String, String)>,
) -> Result<(), SendFailure> {
    if depth > MAX_RECURSE_DEPTH {
        return Err(SendFailure::Other {
            message: "form field nesting exceeds max depth".to_owned(),
        });
    }
    match value {
        Json::Object(entries) => {
            for (name, entry) in entries {
                flatten(
                    &format!("{key}[{name}]"),
                    entry,
                    depth.saturating_add(1),
                    fields,
                )?;
            }
        }
        Json::List(items) => {
            for item in items {
                flatten(&format!("{key}[]"), item, depth.saturating_add(1), fields)?;
            }
        }
        Json::Null => {}
        Json::Bool(flag) => push_field(key, flag.to_string(), fields),
        Json::Number(number) => push_field(key, number.to_string(), fields),
        Json::Text(text) => push_field(key, text.clone(), fields),
    }
    Ok(())
}

// primitive_value_to_str: JSON's own text, with httpx's lowercase booleans and an empty string for null
fn primitive(value: &Json) -> Option<String> {
    match value {
        Json::Null => Some(String::new()),
        Json::Bool(flag) => Some(flag.to_string()),
        Json::Number(number) => Some(number.to_string()),
        Json::Text(text) => Some(text.clone()),
        Json::List(_) | Json::Object(_) => None,
    }
}

fn push_field(key: &str, text: String, fields: &mut Vec<(String, String)>) {
    if !text.is_empty() {
        fields.push((key.to_owned(), text));
    }
}

fn text_part(name: String, text: String) -> MultipartPart {
    MultipartPart {
        name,
        file_name: None,
        content_type: None,
        data: text.as_bytes().to_vec(),
    }
}

fn primitive_error(name: &str) -> SendFailure {
    SendFailure::Other {
        message: format!("the {name} form field is not a primitive value"),
    }
}
