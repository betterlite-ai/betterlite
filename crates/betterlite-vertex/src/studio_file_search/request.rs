use betterlite_bedrock::vector_stores::metadata::non_empty;
use betterlite_config::sections::studio_identity::Settings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

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

pub struct SearchInput<'a> {
    pub vector_store_id: &'a str,
    pub query: &'a SearchQuery,
    pub filters: Option<&'a Json>,
    pub max_num_results: Option<i64>,
    pub extra_body: Option<&'a Json>,
    pub params: &'a RequestParams,
    pub api_base: Option<&'a str>,
    pub settings: &'a Settings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchPlan {
    pub url: String,
    pub body: Json,
    pub query: String,
    pub vector_store_id: String,
}

// get_complete_url appends the version to a configured base exactly as given, so a trailing slash stays.
#[must_use]
pub fn complete_url(api_base: Option<&str>, settings: &Settings) -> String {
    let base = match api_base {
        Some(base) => base.to_owned(),
        None => resolve_base(None, settings),
    };
    format!("{base}/{API_VERSION}")
}

pub fn search_request(input: &SearchInput<'_>) -> Result<SearchPlan, StudioFileSearchError> {
    if resolve_key(&[input.params.api_key.as_ref()], input.settings).is_none() {
        return Err(StudioFileSearchError::MissingKey);
    }
    let api_base = complete_url(input.api_base, input.settings);
    let model = resolved_model(input.params.model.as_deref());
    let query = input.query.joined();
    let mut body = IndexMap::from([
        ("contents".to_owned(), contents(&query)),
        (
            "tools".to_owned(),
            tools(input.vector_store_id, input.filters)?,
        ),
    ]);
    if input.max_num_results.is_some_and(|count| count != 0) {
        body.insert("generationConfig".to_owned(), generation_config());
    }
    Ok(SearchPlan {
        url: format!("{api_base}/models/{model}:generateContent"),
        body: Json::Object(body),
        query,
        vector_store_id: input.vector_store_id.to_owned(),
    })
}

fn resolved_model(configured: Option<&str>) -> String {
    let model = configured
        .filter(|model| !model.is_empty())
        .unwrap_or(DEFAULT_MODEL);
    if model.starts_with("gemini/") {
        model.replace("gemini/", "")
    } else {
        model.to_owned()
    }
}

fn contents(query: &str) -> Json {
    let part = Json::Object(IndexMap::from([(
        "text".to_owned(),
        Json::Text(query.to_owned()),
    )]));
    Json::List(vec![Json::Object(IndexMap::from([(
        "parts".to_owned(),
        Json::List(vec![part]),
    )]))])
}

fn tools(vector_store_id: &str, filters: Option<&Json>) -> Result<Json, serde_json::Error> {
    let config = file_search_config(vector_store_id, filters)?;
    Ok(Json::List(vec![Json::Object(IndexMap::from([(
        "file_search".to_owned(),
        Json::Object(config),
    )]))]))
}

fn file_search_config(
    vector_store_id: &str,
    filters: Option<&Json>,
) -> Result<IndexMap<String, Json>, serde_json::Error> {
    let mut config = IndexMap::from([(
        "file_search_store_names".to_owned(),
        Json::List(vec![Json::Text(vector_store_id.to_owned())]),
    )]);
    if let Some(filters) = filters.filter(|value| non_empty(value)) {
        let value = match filters {
            Json::Object(fields) => Json::Text(metadata_filter(fields)?),
            other => other.clone(),
        };
        config.insert("metadata_filter".to_owned(), value);
    }
    Ok(config)
}

fn metadata_filter(fields: &IndexMap<String, Json>) -> Result<String, serde_json::Error> {
    let mut parts = Vec::with_capacity(fields.len());
    for (key, value) in fields {
        let part = match value {
            Json::Text(text) => format!("{key} = \"{text}\""),
            other => format!("{key} = {}", scalar_spelling(other)?),
        };
        parts.push(part);
    }
    Ok(parts.join(" AND "))
}

// LiteLLM interpolates non-string values with str(), so a scalar keeps its Python spelling.
fn scalar_spelling(value: &Json) -> Result<String, serde_json::Error> {
    match value {
        Json::Null => Ok("None".to_owned()),
        Json::Bool(true) => Ok("True".to_owned()),
        Json::Bool(false) => Ok("False".to_owned()),
        Json::Number(_) => Ok(betterlite_logging::python_json::dumps(value)),
        Json::Text(text) => Ok(text.clone()),
        Json::List(_) | Json::Object(_) => serde_json::to_string(value),
    }
}

fn generation_config() -> Json {
    Json::Object(IndexMap::from([(
        "candidateCount".to_owned(),
        Json::Number(Number::from(1_u8)),
    )]))
}
