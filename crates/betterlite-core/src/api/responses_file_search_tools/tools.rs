use serde::Serialize;

use betterlite_models::decisions::Json;

use crate::api::responses_tool_namespaces::values::{Fields, has_value};

use super::error::FileSearchToolsError;
use super::wire;

pub const FUNCTION_TOOL_NAME: &str = "litellm_file_search";
const FUNCTION_DESCRIPTION: &str = "Search the knowledge base for information relevant to the query. Use this whenever you need to look up specific facts, documents, or content from the vector store. You can provide multiple queries to search for different aspects of the information.";
const QUERIES_DESCRIPTION: &str = "One or more search queries to look up in the vector store. Multiple queries help find comprehensive information from different angles.";
const VECTOR_STORE_DESCRIPTION: &str = "ID of the vector store to search.";
const FILE_SEARCH_TYPE: &str = "file_search";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionTool {
    pub r#type: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: FunctionParameters,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionParameters {
    pub r#type: &'static str,
    pub properties: FunctionProperties,
    pub required: Vec<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionProperties {
    pub queries: QueriesParameter,
    pub vector_store_id: VectorStoreParameter,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QueriesParameter {
    pub r#type: &'static str,
    pub items: ItemType,
    pub description: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ItemType {
    pub r#type: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VectorStoreParameter {
    pub r#type: &'static str,
    pub description: &'static str,
    pub r#enum: Vec<String>,
}

#[must_use]
pub fn build_function_tool(vector_store_ids: &[String]) -> FunctionTool {
    FunctionTool {
        r#type: "function",
        name: FUNCTION_TOOL_NAME,
        description: FUNCTION_DESCRIPTION,
        parameters: FunctionParameters {
            r#type: "object",
            properties: FunctionProperties {
                queries: QueriesParameter {
                    r#type: "array",
                    items: ItemType { r#type: "string" },
                    description: QUERIES_DESCRIPTION,
                },
                vector_store_id: VectorStoreParameter {
                    r#type: "string",
                    description: VECTOR_STORE_DESCRIPTION,
                    r#enum: vector_store_ids.to_vec(),
                },
            },
            required: vec!["queries"],
        },
    }
}

// only a mapping whose type is exactly "file_search" is removed; every other tool keeps its place, and one function tool follows when ids were found
pub fn replace_file_search_tools(
    tools: Option<&[Json]>,
) -> Result<(Vec<Json>, Vec<String>), FileSearchToolsError> {
    let mut kept: Vec<Json> = Vec::new();
    let mut collected: Vec<String> = Vec::new();
    for tool in tools.unwrap_or_default() {
        let Json::Object(fields) = tool else {
            kept.push(tool.clone());
            continue;
        };
        if !matches!(fields.get("type"), Some(Json::Text(kind)) if kind == FILE_SEARCH_TYPE) {
            kept.push(tool.clone());
            continue;
        }
        collected.extend(vector_store_ids(fields)?);
    }
    let ids = first_seen(collected);
    if !ids.is_empty() {
        kept.push(wire(&build_function_tool(&ids))?);
    }
    Ok((kept, ids))
}

// `tool.get("vector_store_ids") or []`: absent, null and falsey values collect nothing
fn vector_store_ids(fields: &Fields) -> Result<Vec<String>, FileSearchToolsError> {
    match fields.get("vector_store_ids") {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::List(items)) => items
            .iter()
            .map(|item| match item {
                Json::Text(id) => Ok(id.clone()),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    Err(FileSearchToolsError::WrongType {
                        field: "vector_store_ids",
                        expected: "a list of strings",
                    })
                }
            })
            .collect(),
        Some(value) if !has_value(value) => Ok(Vec::new()),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            Err(FileSearchToolsError::WrongType {
                field: "vector_store_ids",
                expected: "a list of strings",
            })
        }
    }
}

// dict.fromkeys: the ids keep their first-seen order and are not sorted
fn first_seen(ids: Vec<String>) -> Vec<String> {
    let mut unique: Vec<String> = Vec::new();
    for id in ids {
        if !unique.contains(&id) {
            unique.push(id);
        }
    }
    unique
}
