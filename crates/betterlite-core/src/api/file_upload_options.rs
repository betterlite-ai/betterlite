pub mod error;
pub mod managed;

mod tests;

use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::decisions::Json;

use crate::api::file_upload_options::error::FileUploadOptionsError;
use crate::api::request_body::JsonObject;
use crate::api::request_form::values::{FormParts, PartValue};

const DEFAULT_TARGET_STORAGE: &str = "default";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCreationParams {
    pub target_storage: String,
    pub target_model_names: Vec<String>,
    pub model: Option<String>,
}

// the route's B1100 body view: extraction asks it for the parsed body only when the caller passed none
pub trait RequestBodyRead {
    fn request_body(&self) -> Option<JsonObject>;
}

// the request as its route decoded it: B1100's query and lowercase header maps, B1101's ordered form parts, and the lazy body view
pub struct FileCreationRequest<'request> {
    pub query_params: &'request BTreeMap<String, String>,
    pub headers: &'request BTreeMap<String, String>,
    pub form_parts: &'request FormParts,
    pub read_body: &'request dyn RequestBodyRead,
}

impl Default for FileCreationParams {
    fn default() -> Self {
        Self {
            target_storage: DEFAULT_TARGET_STORAGE.to_owned(),
            target_model_names: Vec::new(),
            model: None,
        }
    }
}

impl FileCreationParams {
    // __post_init__: a falsy storage takes the default without stripping, and names are trimmed and dropped when empty
    #[must_use]
    pub fn new(
        target_storage: Option<String>,
        target_model_names: Option<Vec<String>>,
        model: Option<String>,
    ) -> Self {
        let storage = target_storage.filter(|storage| !storage.is_empty());
        let names = target_model_names
            .unwrap_or_default()
            .iter()
            .map(String::as_str)
            .map(strip_space)
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect();
        Self {
            target_storage: storage.unwrap_or_else(|| DEFAULT_TARGET_STORAGE.to_owned()),
            target_model_names: names,
            model,
        }
    }
}

pub fn extract_file_creation_params(
    request: &FileCreationRequest<'_>,
    request_body: Option<&JsonObject>,
    target_model_names_form: Option<&Json>,
    target_storage_form: Option<&str>,
) -> Result<FileCreationParams, FileUploadOptionsError> {
    let read_body = if request_body.is_none() {
        request.read_body.request_body().unwrap_or_default()
    } else {
        JsonObject::default()
    };
    let body = request_body.unwrap_or(&read_body);
    let target_storage = extract_target_storage_simple(target_storage_form);
    let mut target_model_names = simple_model_names(target_model_names_form)?;
    if target_model_names.is_empty() {
        target_model_names = form_target_model_names(request.form_parts);
    }
    let model = extract_model_param(request, body)?;
    Ok(FileCreationParams::new(
        Some(target_storage),
        Some(target_model_names),
        model,
    ))
}

// str.strip() removes U+001C to U+001F too, which char::is_whitespace keeps
fn strip_space(text: &str) -> &str {
    text.trim_matches(|character: char| {
        character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
    })
}

fn extract_target_storage_simple(target_storage_form: Option<&str>) -> String {
    let Some(form) = target_storage_form else {
        return DEFAULT_TARGET_STORAGE.to_owned();
    };
    strip_space(form).to_owned()
}

fn simple_model_names(form: Option<&Json>) -> Result<Vec<String>, FileUploadOptionsError> {
    match form {
        Some(Json::Text(text)) => Ok(split_model_names(text)),
        Some(Json::List(values)) => list_model_names(values),
        None | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_)) => {
            Ok(Vec::new())
        }
    }
}

fn split_model_names(text: &str) -> Vec<String> {
    text.split(',')
        .map(strip_space)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

fn list_model_names(values: &[Json]) -> Result<Vec<String>, FileUploadOptionsError> {
    let rejected = values.iter().any(|value| match value {
        Json::Bool(true) => true,
        Json::List(items) => !items.is_empty(),
        Json::Object(entries) => !entries.is_empty(),
        Json::Null | Json::Bool(false) | Json::Text(_) | Json::Number(_) => false,
    });
    if rejected {
        return Err(FileUploadOptionsError::NamesNotText);
    }
    Ok(values
        .iter()
        .filter_map(|value| match value {
            Json::Text(text) if !text.is_empty() => Some(strip_space(text).to_owned()),
            Json::Number(number) if number.as_f64().is_none_or(|parsed| parsed != 0.0_f64) => {
                Some(number.to_string())
            }
            Json::Null
            | Json::Bool(_)
            | Json::Text(_)
            | Json::Number(_)
            | Json::List(_)
            | Json::Object(_) => None,
        })
        .collect())
}

fn form_target_model_names(parts: &FormParts) -> Vec<String> {
    let mut seen = BTreeSet::new();
    collected_form_names(parts)
        .into_iter()
        .filter(|name| seen.insert(name.clone()))
        .collect()
}

fn collected_form_names(parts: &FormParts) -> Vec<String> {
    let mut names = Vec::new();
    for part in &parts.items {
        if !is_target_model_names_key(&part.name) {
            continue;
        }
        if let PartValue::Text(text) = &part.value {
            names.extend(split_model_names(text));
        }
    }
    names
}

fn is_target_model_names_key(key: &str) -> bool {
    if key == "target_model_names" {
        return true;
    }
    key.starts_with("target_model_names[") && key.ends_with(']')
}

fn extract_model_param(
    request: &FileCreationRequest<'_>,
    request_body: &JsonObject,
) -> Result<Option<String>, FileUploadOptionsError> {
    if let Some(value) = request_body.get("model") {
        let text = value.as_str();
        if text.is_none() && !value.is_null() {
            return Err(FileUploadOptionsError::ModelNotText);
        }
        if let Some(body_model) = text.filter(|candidate| !candidate.is_empty()) {
            return Ok(Some(body_model.to_owned()));
        }
    }
    if let Some(model) = request
        .query_params
        .get("model")
        .filter(|model| !model.is_empty())
    {
        return Ok(Some(model.clone()));
    }
    Ok(request.headers.get("x-litellm-model").cloned())
}
