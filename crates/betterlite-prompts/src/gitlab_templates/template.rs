use thiserror::Error;

use crate::arguments::json::{Json, Object};

const NON_OPTIONAL_KEYS: [&str; 3] = ["model", "input", "content"];

#[derive(Debug, Error)]
pub enum TemplateError {
    #[error("metadata field 'model' must be a string")]
    ModelNotString,
    #[error("metadata field 'input' must be a mapping")]
    InputNotMapping,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitLabPromptTemplate {
    pub template_id: String,
    pub content: String,
    pub metadata: Object,
    pub model: Option<String>,
    pub temperature: Option<Json>,
    pub max_tokens: Option<Json>,
    pub input_schema: Json,
    pub optional_params: Object,
}

impl GitLabPromptTemplate {
    // An empty model argument counts as absent, like Python's `model or metadata.get("model")`.
    pub fn new(
        template_id: String,
        content: String,
        metadata: Object,
        model: Option<String>,
    ) -> Result<Self, TemplateError> {
        let chosen_model = match model.filter(|name| !name.is_empty()) {
            Some(name) => Some(name),
            None => metadata_model(&metadata)?,
        };
        let input_schema = input_schema(&metadata)?;
        let optional_params = metadata
            .iter()
            .filter(|(key, _)| !NON_OPTIONAL_KEYS.contains(&key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        Ok(Self {
            template_id,
            content,
            temperature: present(&metadata, "temperature"),
            max_tokens: present(&metadata, "max_tokens"),
            metadata,
            model: chosen_model,
            input_schema,
            optional_params,
        })
    }
}

fn present(metadata: &Object, key: &str) -> Option<Json> {
    metadata
        .get(key)
        .filter(|value| **value != Json::Null)
        .cloned()
}

fn metadata_model(metadata: &Object) -> Result<Option<String>, TemplateError> {
    match metadata.get("model") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(name)) => Ok(Some(name.clone())),
        Some(_) => Err(TemplateError::ModelNotString),
    }
}

fn input_schema(metadata: &Object) -> Result<Json, TemplateError> {
    match metadata.get("input") {
        None | Some(Json::Null) => Ok(Json::Object(Object::new())),
        Some(Json::Object(input)) => {
            Ok(present(input, "schema").unwrap_or_else(|| Json::Object(Object::new())))
        }
        Some(_) => Err(TemplateError::InputNotMapping),
    }
}

mod tests;
