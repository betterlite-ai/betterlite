use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::error::VectorStoreRegistryError;

type ParamResult<T> = Result<T, VectorStoreRegistryError>;

// VectorStoreToolParams: the file_search parameters LiteLLM forwards into a vector store call
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VectorStoreToolParams {
    pub filters: Option<JsonObject>,
    pub max_num_results: Option<i64>,
    pub ranking_options: Option<JsonObject>,
}

// _extract_tool_params: every declared VECTOR_STORE_OPENAI_PARAMS name the tool carries, except rewrite_query, which VectorStoreToolParams never accepted and whose presence raises TypeError
pub fn extract_tool_params(
    tool: &JsonObject,
) -> Result<VectorStoreToolParams, VectorStoreRegistryError> {
    if tool.contains_key("rewrite_query") {
        return Err(VectorStoreRegistryError::UnexpectedToolParam {
            name: "rewrite_query",
        });
    }
    Ok(VectorStoreToolParams {
        filters: object_param(tool, "filters")?,
        max_num_results: integer_param(tool, "max_num_results")?,
        ranking_options: object_param(tool, "ranking_options")?,
    })
}

fn object_param(tool: &JsonObject, field: &'static str) -> ParamResult<Option<JsonObject>> {
    match tool.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => Ok(Some(fields.clone())),
        Some(_) => Err(VectorStoreRegistryError::ToolParamType { field }),
    }
}

fn integer_param(tool: &JsonObject, field: &'static str) -> ParamResult<Option<i64>> {
    let Some(value) = tool.get(field) else {
        return Ok(None);
    };
    match value {
        Json::Null => Ok(None),
        Json::Number(number) => number
            .as_i64()
            .map(Some)
            .ok_or(VectorStoreRegistryError::ToolParamType { field }),
        Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            Err(VectorStoreRegistryError::ToolParamType { field })
        }
    }
}

// get_vector_store_ids_to_run: request-level ids first, then the ids every tool carries, each kept at its first position; a non-list top-level value is ignored and the tool type is never consulted
pub fn ids_to_run(
    params: &JsonObject,
    tools: Option<&[JsonObject]>,
) -> Result<Vec<String>, VectorStoreRegistryError> {
    let mut ids = Vec::new();
    if let Some(Json::List(items)) = params.get("vector_store_ids") {
        ids_from_list(items, "vector_store_ids", &mut ids)?;
    }
    ids_from_tool_calls(tools, &mut ids)?;
    let mut seen = BTreeSet::new();
    let mut unique = Vec::new();
    for id in ids {
        if seen.insert(id.clone()) {
            unique.push(id);
        }
    }
    Ok(unique)
}

// _get_vector_store_ids_from_tool_calls
pub fn ids_from_tool_calls(
    tools: Option<&[JsonObject]>,
    ids: &mut Vec<String>,
) -> Result<(), VectorStoreRegistryError> {
    let Some(calls) = tools else {
        return Ok(());
    };
    for tool in calls {
        let Some(value) = tool.get("vector_store_ids") else {
            continue;
        };
        let Json::List(items) = value else {
            return Err(VectorStoreRegistryError::IdList {
                field: "tool.vector_store_ids",
            });
        };
        ids_from_list(items, "tool.vector_store_ids", ids)?;
    }
    Ok(())
}

fn ids_from_list(
    items: &[Json],
    field: &'static str,
    ids: &mut Vec<String>,
) -> Result<(), VectorStoreRegistryError> {
    for item in items {
        match item {
            Json::Text(id) => ids.push(id.clone()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                return Err(VectorStoreRegistryError::IdList { field });
            }
        }
    }
    Ok(())
}
