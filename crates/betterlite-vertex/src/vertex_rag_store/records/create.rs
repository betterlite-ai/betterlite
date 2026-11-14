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
    let display_name = match optional_params.get("name") {
        None => Some(DEFAULT_CORPUS_NAME.to_owned()),
        Some(Json::Null) => None,
        Some(Json::Text(name)) => Some(name.clone()),
        Some(_) => return Err(VertexRagStoreError::ParamType("name", "a string")),
    };
    let labels = match optional_params.get("metadata") {
        None | Some(Json::Null) => None,
        Some(Json::Object(fields)) => Some(
            fields
                .iter()
                .map(|(name, value)| match value {
                    Json::Text(text) => Ok((name.clone(), text.clone())),
                    _ => Err(VertexRagStoreError::ParamType(
                        "metadata",
                        "an object of strings",
                    )),
                })
                .collect::<Result<BTreeMap<_, _>, _>>()?,
        ),
        Some(_) => return Err(VertexRagStoreError::ParamType("metadata", "an object")),
    };
    let mut url = api_base.to_owned();
    url.push_str(CORPUS_SUFFIX);
    Ok(CreateCall {
        url,
        body: CreateBody {
            display_name,
            description: CORPUS_DESCRIPTION,
            labels,
        },
    })
}
