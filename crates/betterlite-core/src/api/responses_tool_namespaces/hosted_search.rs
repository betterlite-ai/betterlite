use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::builders::WebSearchStatus;
use serde::{Deserialize, Serialize};

use super::records::ChoiceRecord;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    Url,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostedSearchType {
    WebSearchCall,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchSource {
    #[serde(rename = "type")]
    pub kind: SourceType,
    pub url: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SearchAction {
    Search {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        queries: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        query: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sources: Option<Vec<SearchSource>>,
        #[serde(flatten)]
        extra: BTreeMap<String, Json>,
    },
    OpenPage {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(flatten)]
        extra: BTreeMap<String, Json>,
    },
    #[serde(rename = "find_in_page")]
    Find {
        pattern: String,
        url: String,
        #[serde(flatten)]
        extra: BTreeMap<String, Json>,
    },
}

// the shape of the OpenAI SDK's ResponseFunctionWebSearch, which allows extra fields
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostedSearchCall {
    pub id: String,
    pub action: SearchAction,
    pub status: WebSearchStatus,
    #[serde(rename = "type")]
    pub kind: HostedSearchType,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Json>,
}

// a provider's web_search_calls, keyed by call id without the ws_ prefix; invalid items are skipped and a later duplicate wins
#[must_use]
pub fn calls_by_id(choices: &[ChoiceRecord]) -> BTreeMap<String, HostedSearchCall> {
    let mut calls = BTreeMap::new();
    for choice in choices {
        let Json::Object(fields) = &choice.provider_specific_fields else {
            continue;
        };
        let items: Vec<&Json> = match fields.get("web_search_calls") {
            Some(Json::Object(by_key)) => by_key.values().collect(),
            Some(Json::List(listed)) => listed.iter().collect(),
            None | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_)) => Vec::new(),
        };
        let valid = items.into_iter().filter_map(|item| {
            serde_json::from_value::<HostedSearchCall>(serde_json::to_value(item).ok()?).ok()
        });
        for call in valid {
            let key = call.id.strip_prefix("ws_").unwrap_or(&call.id).to_owned();
            calls.insert(key, call);
        }
    }
    calls
}
