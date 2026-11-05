use serde_json::Number;
use thiserror::Error;

use crate::arguments::json::{Json, Object};

const EXCLUDED_PARAMS: [&str; 3] = ["model", "input", "content"];

#[derive(Debug, Error)]
pub enum TemplateError {
    #[error("metadata field 'model' must be a string")]
    ModelNotString,
    #[error("metadata field '{0}' must be a number")]
    NotANumber(&'static str),
    #[error("metadata field 'input' must be a mapping")]
    InputNotMapping,
    #[error("metadata field 'input.schema' must be a mapping")]
    SchemaNotMapping,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitBucketPromptTemplate {
    pub template_id: String,
    pub content: String,
    pub metadata: Object,
    pub model: Option<String>,
    pub temperature: Option<Number>,
    pub max_tokens: Option<Number>,
    pub input_schema: Object,
    pub optional_params: Object,
}

fn number_field(metadata: &Object, field: &'static str) -> Result<Option<Number>, TemplateError> {
    match metadata.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => Ok(Some(number.clone())),
        Some(_) => Err(TemplateError::NotANumber(field)),
    }
}

fn mapping_field(
    source: &Object,
    field: &str,
    error: TemplateError,
) -> Result<Option<Object>, TemplateError> {
    match source.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(mapping)) => Ok(Some(mapping.clone())),
        Some(_) => Err(error),
    }
}

impl BitBucketPromptTemplate {
    pub fn new(
        template_id: String,
        content: String,
        metadata: Object,
        model: Option<String>,
    ) -> Result<Self, TemplateError> {
        let metadata_model = match metadata.get("model") {
            None | Some(Json::Null) => None,
            Some(Json::Text(text)) => Some(text.clone()),
            Some(_) => return Err(TemplateError::ModelNotString),
        };
        let input_schema = match mapping_field(&metadata, "input", TemplateError::InputNotMapping)?
        {
            Some(section) => mapping_field(&section, "schema", TemplateError::SchemaNotMapping)?
                .unwrap_or_default(),
            None => Object::new(),
        };
        let optional_params = metadata
            .iter()
            .filter(|(key, _)| !EXCLUDED_PARAMS.contains(&key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();

        Ok(Self {
            template_id,
            content,
            model: model.or(metadata_model),
            temperature: number_field(&metadata, "temperature")?,
            max_tokens: number_field(&metadata, "max_tokens")?,
            input_schema,
            optional_params,
            metadata,
        })
    }
}
