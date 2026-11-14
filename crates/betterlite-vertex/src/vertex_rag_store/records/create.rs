use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use serde::Serialize;

use super::super::error::VertexRagStoreError;

const DEFAULT_CORPUS_NAME: &str = "litellm-vector-store";
const CORPUS_DESCRIPTION: &str = "Vector store created via LiteLLM";
const CORPUS_SUFFIX: &str = "/ragCorpora";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CreateBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub description: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateCall {
    pub url: String,
    pub body: CreateBody,
}

// LiteLLM's transform_create_vector_store_request: an absent name takes the default and an explicit null drops the key
pub fn transform_create_request(
    optional_params: &BTreeMap<String, Json>,
    api_base: &str,
) -> Result<CreateCall, VertexRagStoreError> {
    Ok(CreateCall {
        url: format!("{api_base}{CORPUS_SUFFIX}"),
        body: CreateBody {
            display_name: display_name(optional_params)?,
            description: CORPUS_DESCRIPTION,
            labels: labels(optional_params)?,
        },
    })
}

fn display_name(
    optional_params: &BTreeMap<String, Json>,
) -> Result<Option<String>, VertexRagStoreError> {
    match optional_params.get("name") {
        None => Ok(Some(DEFAULT_CORPUS_NAME.to_owned())),
        Some(Json::Null) => Ok(None),
        Some(Json::Text(name)) => Ok(Some(name.clone())),
        Some(_) => Err(VertexRagStoreError::ParamType {
            field: "name",
            expected: "a string",
        }),
    }
}

fn labels(
    optional_params: &BTreeMap<String, Json>,
) -> Result<Option<BTreeMap<String, String>>, VertexRagStoreError> {
    match optional_params.get("metadata") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => fields
            .iter()
            .map(|(name, value)| match value {
                Json::Text(text) => Ok((name.clone(), text.clone())),
                _ => Err(VertexRagStoreError::ParamType {
                    field: "metadata",
                    expected: "an object of strings",
                }),
            })
            .collect::<Result<_, _>>()
            .map(Some),
        Some(_) => Err(VertexRagStoreError::ParamType {
            field: "metadata",
            expected: "an object",
        }),
    }
}
