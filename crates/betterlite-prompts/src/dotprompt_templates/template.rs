use crate::arguments::json::{Json, Object};
use thiserror::Error;

const RESTRICTED_KEYS: [&str; 3] = ["model", "input", "output"];

#[derive(Debug, Error)]
pub enum TemplateError {
    #[error("metadata field 'model' must be a string")]
    ModelNotString,
    #[error("metadata field 'output.format' must be a string")]
    OutputFormatNotString,
    #[error("metadata field '{0}' must be a mapping")]
    SectionNotMapping(&'static str),
    #[error("metadata field '{0}.schema' must be a mapping")]
    SchemaNotMapping(&'static str),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptTemplate {
    pub content: String,
    pub metadata: Object,
    pub template_id: Option<String>,
    pub model: Option<String>,
    pub input_schema: Object,
    pub output_format: Option<String>,
    pub output_schema: Object,
    pub optional_params: Object,
}

impl PromptTemplate {
    pub fn new(
        content: String,
        metadata: Object,
        template_id: Option<String>,
    ) -> Result<Self, TemplateError> {
        let model = string_field(&metadata, "model", TemplateError::ModelNotString)?;
        let input_schema = schema_field(&metadata, "input")?;
        let (output_format, output_schema) = output_field(&metadata)?;
        let optional_params = metadata
            .iter()
            .filter(|(key, _)| !RESTRICTED_KEYS.contains(&key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();

        Ok(Self {
            content,
            metadata,
            template_id,
            model,
            input_schema,
            output_format,
            output_schema,
            optional_params,
        })
    }
}

fn string_field(
    metadata: &Object,
    field: &str,
    error: TemplateError,
) -> Result<Option<String>, TemplateError> {
    match metadata.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(error),
    }
}

fn section(metadata: &Object, field: &'static str) -> Result<Object, TemplateError> {
    match metadata.get(field) {
        None => Ok(Object::new()),
        Some(Json::Object(section)) => Ok(section.clone()),
        Some(_) => Err(TemplateError::SectionNotMapping(field)),
    }
}

fn nested_schema(section: &Object, field: &'static str) -> Result<Object, TemplateError> {
    match section.get("schema") {
        None | Some(Json::Null) => Ok(Object::new()),
        Some(Json::Object(schema)) => Ok(schema.clone()),
        Some(_) => Err(TemplateError::SchemaNotMapping(field)),
    }
}

fn schema_field(metadata: &Object, field: &'static str) -> Result<Object, TemplateError> {
    let section = section(metadata, field)?;
    nested_schema(&section, field)
}

fn output_field(metadata: &Object) -> Result<(Option<String>, Object), TemplateError> {
    let section = section(metadata, "output")?;
    let format = string_field(&section, "format", TemplateError::OutputFormatNotString)?;
    let schema = nested_schema(&section, "output")?;
    Ok((format, schema))
}

mod tests;
