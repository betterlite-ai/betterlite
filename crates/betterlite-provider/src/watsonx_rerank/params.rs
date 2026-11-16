use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::WatsonxRerankError;
use crate::api::rerank_contracts::{MapCohereRerankParams, Params};

pub const SUPPORTED_COHERE_RERANK_PARAMS: [&str; 5] = [
    "query",
    "documents",
    "top_n",
    "return_documents",
    "max_tokens_per_doc",
];

pub fn map_cohere_rerank_params(
    request: &MapCohereRerankParams<'_>,
) -> Result<Params, WatsonxRerankError> {
    let mut mapped = Params::new();
    for (key, value) in request.non_default_params.iter() {
        match key.as_str() {
            "query" | "project_id" | "space_id" => {
                if !matches!(value, Json::Null) {
                    mapped.insert(key.clone(), value.clone());
                }
            }
            "documents" => match value {
                Json::Null => {}
                Json::List(_) => {
                    mapped.insert(String::from("inputs"), inputs(value)?);
                }
                Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
                    return Err(WatsonxRerankError::InvalidDocuments);
                }
            },
            "top_n" => {
                if !matches!(value, Json::Null) {
                    if let Some(return_options) = return_options(&mut mapped) {
                        return_options.insert(String::from("top_n"), value.clone());
                    }
                }
            }
            "return_documents" => {
                if let Json::Bool(_) = value {
                    if let Some(return_options) = return_options(&mut mapped) {
                        return_options.insert(String::from("inputs"), value.clone());
                    }
                }
            }
            "max_tokens_per_doc" => {
                if !matches!(value, Json::Null) {
                    if let Some(parameters) = parameters(&mut mapped) {
                        parameters.insert(String::from("truncate_input_tokens"), value.clone());
                    }
                }
            }
            _ => {}
        }
    }
    Ok(mapped)
}

fn inputs(value: &Json) -> Result<Json, WatsonxRerankError> {
    let Json::List(items) = value else {
        return Err(WatsonxRerankError::InvalidDocuments);
    };
    let mut converted = Vec::with_capacity(items.len());
    for item in items {
        match item {
            Json::Text(text) => {
                let mut fields = IndexMap::new();
                fields.insert(String::from("text"), Json::Text(text.clone()));
                converted.push(Json::Object(fields));
            }
            Json::Object(fields) => converted.push(Json::Object(fields.clone())),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
                return Err(WatsonxRerankError::InvalidDocuments);
            }
        }
    }
    Ok(Json::List(converted))
}

fn as_object(value: &mut Json) -> Option<&mut IndexMap<String, Json>> {
    match value {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

fn parameters(mapped: &mut Params) -> Option<&mut IndexMap<String, Json>> {
    as_object(
        mapped
            .entry(String::from("parameters"))
            .or_insert_with(|| Json::Object(IndexMap::new())),
    )
}

fn return_options(mapped: &mut Params) -> Option<&mut IndexMap<String, Json>> {
    as_object(
        parameters(mapped)?
            .entry(String::from("return_options"))
            .or_insert_with(|| Json::Object(IndexMap::new())),
    )
}
