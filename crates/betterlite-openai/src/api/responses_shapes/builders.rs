use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};

use super::{CodeInterpreterLogType, OutputCodeInterpreterCallLog};

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Field<T> {
    Known(T),
    Other(IgnoredAny),
}

impl<T> Default for Field<T> {
    fn default() -> Self {
        Self::Other(IgnoredAny)
    }
}

#[derive(Debug, Deserialize)]
pub struct SearchInput {
    #[serde(default)]
    pub query: Field<String>,
}

#[derive(Debug, Deserialize)]
pub struct SearchResult {
    #[serde(default)]
    pub content: SearchContent,
}

#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
pub enum SearchContent {
    Items(Vec<Field<SearchItem>>),
    Error {
        r#type: String,
    },
    Other(IgnoredAny),
    #[default]
    Missing,
}

#[derive(Debug, Deserialize)]
pub struct SearchItem {
    pub r#type: String,
    pub url: Field<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebSearchStatus {
    InProgress,
    Searching,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename = "url")]
pub struct WebSearchSource {
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename = "search")]
pub struct WebSearchAction {
    pub query: String,
    pub queries: Vec<String>,
    pub sources: Vec<WebSearchSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename = "web_search_call")]
pub struct WebSearchCall {
    pub id: String,
    pub status: WebSearchStatus,
    pub action: WebSearchAction,
}

pub fn build_web_search_call(
    tool_id: &str,
    tool_input: &Field<SearchInput>,
    result: &Field<SearchResult>,
    status: Option<WebSearchStatus>,
) -> WebSearchCall {
    let query = match tool_input {
        Field::Known(SearchInput {
            query: Field::Known(text),
        }) => text.clone(),
        Field::Known(SearchInput {
            query: Field::Other(_),
        })
        | Field::Other(_) => String::new(),
    };
    let (sources, inferred) = search_sources(result);
    WebSearchCall {
        id: format!("ws_{tool_id}"),
        status: status.unwrap_or(inferred),
        action: WebSearchAction {
            queries: if query.is_empty() {
                Vec::new()
            } else {
                vec![query.clone()]
            },
            query,
            sources,
        },
    }
}

fn search_sources(result: &Field<SearchResult>) -> (Vec<WebSearchSource>, WebSearchStatus) {
    match result {
        Field::Known(SearchResult {
            content: SearchContent::Items(items),
        }) => (
            items
                .iter()
                .filter_map(|item| match item {
                    Field::Known(SearchItem {
                        r#type,
                        url: Field::Known(url),
                    }) if r#type == "web_search_result" => {
                        Some(WebSearchSource { url: url.clone() })
                    }
                    Field::Known(_) | Field::Other(_) => None,
                })
                .collect(),
            WebSearchStatus::Completed,
        ),
        Field::Known(SearchResult {
            content: SearchContent::Error { r#type },
        }) if r#type == "web_search_tool_result_error" => (Vec::new(), WebSearchStatus::Failed),
        Field::Known(_) | Field::Other(_) => (Vec::new(), WebSearchStatus::Completed),
    }
}

#[derive(Debug, Deserialize)]
pub struct InterpreterContent {
    #[serde(default)]
    pub stdout: Option<String>,
    #[serde(default)]
    pub stderr: Option<String>,
}

pub fn build_interpreter_logs(
    content: &Field<InterpreterContent>,
) -> Option<Vec<OutputCodeInterpreterCallLog>> {
    let Field::Known(fields) = content else {
        return None;
    };
    let mut logs = fields.stdout.clone().unwrap_or_default();
    if let Some(stderr) = fields.stderr.as_ref().filter(|text| !text.is_empty()) {
        logs.push_str("STDERR: ");
        logs.push_str(stderr);
    }
    if logs.is_empty() {
        None
    } else {
        Some(vec![OutputCodeInterpreterCallLog {
            r#type: CodeInterpreterLogType::Logs,
            logs,
        }])
    }
}
