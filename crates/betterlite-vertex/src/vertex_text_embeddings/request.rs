use std::slice;

use betterlite_models::decisions::Extra;
use betterlite_models::decisions::Json;
use betterlite_openai::embedding::EmbeddingInput;

use crate::request_labels::pop_vertex_request_labels;

use super::error::VertexTextEmbeddingError;
use super::family::Family;
use super::family::family_of;
use super::task_type::TaskType;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 1] = ["dimensions"];
pub const MAPPED_SPECIAL_AUTH_PARAMS: [(&str, &str); 2] = [
    ("project", "vertex_project"),
    ("region_name", "vertex_location"),
];

#[must_use]
pub fn map_openai_params(
    non_default_params: &Extra,
    mut optional_params: Extra,
    mut kwargs: Extra,
) -> (Extra, Extra) {
    if let Some(dimensions) = non_default_params.get("dimensions") {
        optional_params.insert(String::from("outputDimensionality"), dimensions.clone());
    }
    if let Some(input_type) = kwargs.shift_remove("input_type") {
        optional_params.insert(String::from("task_type"), input_type);
    }
    (optional_params, kwargs)
}

#[must_use]
pub fn map_special_auth_params(non_default_params: &Extra, mut optional_params: Extra) -> Extra {
    for (source, target) in MAPPED_SPECIAL_AUTH_PARAMS {
        if let Some(value) = non_default_params.get(source) {
            optional_params.insert(String::from(target), value.clone());
        }
    }
    optional_params
}

pub fn transform_request(
    input: &EmbeddingInput,
    optional_params: &Json,
    model: &str,
    litellm_params: Option<&Json>,
) -> Result<Extra, VertexTextEmbeddingError> {
    let mut params = Some(optional_params.clone());
    let request_labels = pop_vertex_request_labels(&mut params, litellm_params);
    let Some(Json::Object(mut parameters)) = params else {
        return Err(VertexTextEmbeddingError::OptionalParams);
    };
    let texts = texts(input)?;
    let family = family_of(model);
    let (task_type, title) = instance_fields(&parameters, family)?;
    let instances = match family {
        Family::Numeric => {
            parameters.shift_remove("shared_session");
            records(texts, "inputs", None, None)
        }
        Family::Bge => records(texts, "prompt", task_type, title),
        Family::Standard => records(texts, "content", task_type, title),
    };
    let mut request = Extra::new();
    request.insert(String::from("instances"), instances);
    request.insert(String::from("parameters"), Json::Object(parameters));
    if let Some(labels) = request_labels {
        let fields = labels
            .into_iter()
            .map(|(name, value)| (name, Json::Text(value)))
            .collect();
        request.insert(String::from("labels"), Json::Object(fields));
    }
    Ok(request)
}

const fn texts(input: &EmbeddingInput) -> Result<&[String], VertexTextEmbeddingError> {
    match input {
        EmbeddingInput::Text(text) => Ok(slice::from_ref(text)),
        EmbeddingInput::Texts(values) => Ok(values.as_slice()),
        EmbeddingInput::Tokens(_) | EmbeddingInput::TokenLists(_) => {
            Err(VertexTextEmbeddingError::InputNotText)
        }
    }
}

fn records(texts: &[String], key: &str, task_type: Option<&str>, title: Option<&str>) -> Json {
    let mut list = Vec::with_capacity(texts.len());
    for text in texts {
        let mut record = Extra::new();
        record.insert(key.to_owned(), Json::Text(text.clone()));
        if let Some(kind) = task_type {
            record.insert(String::from("task_type"), Json::Text(kind.to_owned()));
        }
        if let Some(document_title) = title {
            record.insert(String::from("title"), Json::Text(document_title.to_owned()));
        }
        list.push(Json::Object(record));
    }
    Json::List(list)
}

fn instance_fields(
    parameters: &Extra,
    family: Family,
) -> Result<(Option<&'static str>, Option<&str>), VertexTextEmbeddingError> {
    if matches!(family, Family::Numeric) {
        return Ok((None, None));
    }
    let task_type = match text_parameter(parameters, "task_type")? {
        Some(text) => Some(parse_task_type(text)?),
        None => None,
    };
    Ok((task_type, text_parameter(parameters, "title")?))
}

fn text_parameter<'params>(
    parameters: &'params Extra,
    name: &'static str,
) -> Result<Option<&'params str>, VertexTextEmbeddingError> {
    let Some(value) = present(parameters, name) else {
        return Ok(None);
    };
    let Json::Text(text) = value else {
        return Err(VertexTextEmbeddingError::NotText { field: name });
    };
    Ok(Some(text))
}

fn parse_task_type(value: &str) -> Result<&'static str, VertexTextEmbeddingError> {
    let Some(task_type) = TaskType::parse(value) else {
        return Err(VertexTextEmbeddingError::TaskType {
            value: value.to_owned(),
        });
    };
    Ok(task_type.as_str())
}

fn present<'params>(parameters: &'params Extra, name: &str) -> Option<&'params Json> {
    let value = parameters.get(name)?;
    if matches!(value, Json::Null) {
        return None;
    }
    Some(value)
}
