use betterlite_bedrock::vector_stores::metadata::non_empty;
use betterlite_config::sections::studio_identity::Settings;
use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;

use super::RequestParams;
use super::error::StudioFileSearchError;
use crate::studio_files::{resolve_base, resolve_key};
use crate::studio_identity::API_VERSION;

pub const DEFAULT_MODEL: &str = "gemini-2.5-flash";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchQuery {
    Text(String),
    Sequence(Vec<String>),
}

impl SearchQuery {
    #[must_use]
    pub fn joined(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Sequence(items) => items.join(" "),
        }
    }
}

pub struct SearchInput<'input> {
    pub vector_store_id: &'input str,
    pub query: &'input SearchQuery,
    pub filters: Option<&'input Json>,
    pub max_num_results: Option<i64>,
    pub extra_body: Option<&'input Json>,
    pub params: &'input RequestParams,
    pub api_base: Option<&'input str>,
    pub settings: &'input Settings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchPlan {
    pub url: String,
    pub body: Json,
    pub query: String,
    pub vector_store_id: String,
}

#[derive(Debug, Serialize)]
struct SearchBody<'input> {
    contents: [Content<'input>; 1],
    tools: [Tool<'input>; 1],
    #[serde(rename = "generationConfig", skip_serializing_if = "Option::is_none")]
    generation_config: Option<GenerationConfig>,
}

#[derive(Debug, Serialize)]
struct Content<'input> {
    parts: [Part<'input>; 1],
}

#[derive(Debug, Serialize)]
struct Part<'input> {
    text: &'input str,
}

#[derive(Debug, Serialize)]
struct Tool<'input> {
    file_search: FileSearch<'input>,
}

#[derive(Debug, Serialize)]
struct FileSearch<'input> {
    file_search_store_names: [&'input str; 1],
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata_filter: Option<Json>,
}

#[derive(Debug, Serialize)]
struct GenerationConfig {
    #[serde(rename = "candidateCount")]
    candidate_count: u8,
}

pub fn search_request(input: &SearchInput<'_>) -> Result<SearchPlan, StudioFileSearchError> {
    if resolve_key(&[input.params.api_key.as_ref()], input.settings).is_none() {
        return Err(StudioFileSearchError::MissingKey);
    }
    // get_complete_url appends the version to a configured base exactly as given, so a trailing slash stays.
    let base = input
        .api_base
        .map_or_else(|| resolve_base(None, input.settings), str::to_owned);
    let api_base = format!("{base}/{API_VERSION}");
    let query = input.query.joined();
    let filter = match input.filters.filter(|value| non_empty(value)) {
        Some(Json::Object(fields)) => Some(Json::Text(metadata_filter(fields)?)),
        other => other.cloned(),
    };
    let search_body = SearchBody {
        contents: [Content {
            parts: [Part { text: &query }],
        }],
        tools: [Tool {
            file_search: FileSearch {
                file_search_store_names: [input.vector_store_id],
                metadata_filter: filter,
            },
        }],
        generation_config: input
            .max_num_results
            .is_some_and(|count| count != 0)
            .then_some(GenerationConfig { candidate_count: 1 }),
    };
    let body = serde_json::to_value(search_body).and_then(serde_json::from_value::<Json>)?;
    let model = resolved_model(input.params.model.as_deref());
    Ok(SearchPlan {
        url: format!("{api_base}/models/{model}:generateContent"),
        body,
        query,
        vector_store_id: input.vector_store_id.to_owned(),
    })
}

fn resolved_model(configured: Option<&str>) -> String {
    let model = configured
        .filter(|model| !model.is_empty())
        .unwrap_or(DEFAULT_MODEL);
    if model.starts_with("gemini/") {
        return model.replace("gemini/", "");
    }
    model.to_owned()
}

// LiteLLM quotes string filters without escaping and interpolates any other value with str().
fn metadata_filter(fields: &IndexMap<String, Json>) -> Result<String, serde_json::Error> {
    let parts = fields
        .iter()
        .map(|(key, value)| match value {
            Json::Text(text) => Ok(format!("{key} = \"{text}\"")),
            Json::Null => Ok(format!("{key} = None")),
            Json::Bool(true) => Ok(format!("{key} = True")),
            Json::Bool(false) => Ok(format!("{key} = False")),
            Json::Number(_) => Ok(format!("{key} = {}", python_json::dumps(value))),
            Json::List(_) | Json::Object(_) => {
                Ok(format!("{key} = {}", serde_json::to_string(value)?))
            }
        })
        .collect::<Result<Vec<_>, serde_json::Error>>()?;
    Ok(parts.join(" AND "))
}
