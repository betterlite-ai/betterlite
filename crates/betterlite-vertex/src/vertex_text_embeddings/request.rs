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
    for (name, value) in non_default_params {
        if name == "dimensions" {
            optional_params.insert(String::from("outputDimensionality"), value.clone());
        }
    }
    if let Some(input_type) = kwargs.shift_remove("input_type") {
        optional_params.insert(String::from("task_type"), input_type);
    }
    (optional_params, kwargs)
}

#[must_use]
pub fn map_special_auth_params(non_default_params: &Extra, mut optional_params: Extra) -> Extra {
    for (name, value) in non_default_params {
        for (source, target) in MAPPED_SPECIAL_AUTH_PARAMS {
            if name == source {
                optional_params.insert(String::from(target), value.clone());
            }
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
    let instances = match family_of(model) {
        Family::Numeric => {
            parameters.shift_remove("shared_session");
            records(texts, "inputs", None, None)
        }
        Family::Bge => records(
            texts,
            "prompt",
            task_type_value(&parameters)?,
            title_value(&parameters)?,
        ),
        Family::Standard => records(
            texts,
            "content",
            task_type_value(&parameters)?,
            title_value(&parameters)?,
        ),
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

fn task_type_value(parameters: &Extra) -> Result<Option<&'static str>, VertexTextEmbeddingError> {
    let Some(value) = present(parameters, "task_type") else {
        return Ok(None);
    };
    let Json::Text(text) = value else {
        return Err(VertexTextEmbeddingError::NotText { field: "task_type" });
    };
    let Some(task_type) = TaskType::parse(text) else {
        return Err(VertexTextEmbeddingError::TaskType {
            value: text.clone(),
        });
    };
    Ok(Some(task_type.as_str()))
}

fn title_value(parameters: &Extra) -> Result<Option<&str>, VertexTextEmbeddingError> {
    let Some(value) = present(parameters, "title") else {
        return Ok(None);
    };
    let Json::Text(text) = value else {
        return Err(VertexTextEmbeddingError::NotText { field: "title" });
    };
    Ok(Some(text))
}

fn present<'params>(parameters: &'params Extra, name: &str) -> Option<&'params Json> {
    let value = parameters.get(name)?;
    if matches!(value, Json::Null) {
        return None;
    }
    Some(value)
}
