use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::WatsonxRerankError;
use crate::api::rerank_contracts::{MapCohereRerankParams, Params};

pub fn map_cohere_rerank_params(
    request: &MapCohereRerankParams<'_>,
) -> Result<Params, WatsonxRerankError> {
    let mut mapped = Params::new();
    for (key, value) in &*request.non_default_params {
        match key.as_str() {
            "query" | "project_id" | "space_id" => {
                if !matches!(value, Json::Null) {
                    mapped.insert(key.clone(), value.clone());
                }
            }
            "documents" => match value {
                Json::Null => {}
                Json::List(items) => {
                    mapped.insert(String::from("inputs"), inputs(items)?);
                }
                Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
                    return Err(WatsonxRerankError::InvalidDocuments);
                }
            },
            "top_n" if !matches!(value, Json::Null) => {
                if let Some(return_options) = return_options(&mut mapped) {
                    return_options.insert(String::from("top_n"), value.clone());
                }
            }
            "return_documents" if matches!(value, Json::Bool(_)) => {
                if let Some(return_options) = return_options(&mut mapped) {
                    return_options.insert(String::from("inputs"), value.clone());
                }
            }
            "max_tokens_per_doc" if !matches!(value, Json::Null) => {
                if let Some(parameters) = parameters(&mut mapped) {
                    parameters.insert(String::from("truncate_input_tokens"), value.clone());
                }
            }
            _ => {}
        }
    }
    Ok(mapped)
}

fn inputs(items: &[Json]) -> Result<Json, WatsonxRerankError> {
    let converted = items
        .iter()
        .map(|item| match item {
            Json::Text(text) => Ok(Json::Object(IndexMap::from([(
                String::from("text"),
                Json::Text(text.clone()),
            )]))),
            Json::Object(fields) => Ok(Json::Object(fields.clone())),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
                Err(WatsonxRerankError::InvalidDocuments)
            }
        })
        .collect::<Result<_, _>>()?;
    Ok(Json::List(converted))
}

const fn as_object(value: &mut Json) -> Option<&mut IndexMap<String, Json>> {
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
