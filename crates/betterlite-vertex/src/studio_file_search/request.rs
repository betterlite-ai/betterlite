use betterlite_bedrock::vector_stores::metadata::non_empty;
use betterlite_config::sections::studio_identity::Settings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::{Map, Value, json};

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
    let base = api_base.map_or_else(|| resolve_base(None, settings), str::to_owned);
    format!("{base}/{API_VERSION}")
}

pub fn search_request(input: &SearchInput<'_>) -> Result<SearchPlan, StudioFileSearchError> {
    if resolve_key(&[input.params.api_key.as_ref()], input.settings).is_none() {
        return Err(StudioFileSearchError::MissingKey);
    }
    let api_base = complete_url(input.api_base, input.settings);
    let query = input.query.joined();
    let mut file_search = Map::new();
    file_search.insert(
        "file_search_store_names".to_owned(),
        json!([input.vector_store_id]),
    );
    if let Some(filters) = input.filters.filter(|value| non_empty(value)) {
        let value = match filters {
            Json::Object(fields) => Value::String(metadata_filter(fields)?),
            other => serde_json::to_value(other)?,
        };
        file_search.insert("metadata_filter".to_owned(), value);
    }
    let mut body = json!({
        "contents": [{"parts": [{"text": query}]}],
        "tools": [{"file_search": file_search}],
    });
    if input.max_num_results.is_some_and(|count| count != 0)
        && let Value::Object(fields) = &mut body
    {
        fields.insert("generationConfig".to_owned(), json!({"candidateCount": 1}));
    }
    let model = resolved_model(input.params.model.as_deref());
    Ok(SearchPlan {
        url: format!("{api_base}/models/{model}:generateContent"),
        body: serde_json::from_value(body)?,
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

// LiteLLM quotes string filters without escaping and interpolates any other value with str().
fn metadata_filter(fields: &IndexMap<String, Json>) -> Result<String, serde_json::Error> {
    let mut parts = Vec::with_capacity(fields.len());
    for (key, value) in fields {
        let spelled = match value {
            Json::Text(text) => format!("{key} = \"{text}\""),
            Json::Null => format!("{key} = None"),
            Json::Bool(true) => format!("{key} = True"),
            Json::Bool(false) => format!("{key} = False"),
            Json::Number(_) => format!("{key} = {}", betterlite_logging::python_json::dumps(value)),
            Json::List(_) | Json::Object(_) => {
                format!("{key} = {}", serde_json::to_string(value)?)
            }
        };
        parts.push(spelled);
    }
    Ok(parts.join(" AND "))
}
