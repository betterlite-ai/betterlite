use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde::Serialize;
use serde::de::{DeserializeOwned, IgnoredAny};

#[derive(Debug, Default, Deserialize)]
pub struct ReplayedSearchQuery {
    #[serde(default)]
    pub query: String,
    #[serde(flatten)]
    pub ignored: IgnoredAny,
}

#[derive(Debug, Deserialize)]
pub struct ReplayedWebSearchResult {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub snippet: String,
    #[serde(default)]
    pub encrypted_content: String,
    #[serde(flatten)]
    pub ignored: IgnoredAny,
}

#[derive(Debug, Deserialize)]
pub struct WebSearchToolResultError {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub error_code: String,
    #[serde(flatten)]
    pub ignored: IgnoredAny,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum WebSearchContent {
    Results(Vec<ReplayedWebSearchResult>),
    Error(WebSearchToolResultError),
}

#[derive(Debug, Deserialize)]
pub struct ReplayedWebSearchToolResult {
    #[serde(rename = "type")]
    pub kind: String,
    pub tool_use_id: String,
    pub content: WebSearchContent,
    #[serde(flatten)]
    pub ignored: IgnoredAny,
}

#[derive(Debug, Deserialize)]
pub struct ReplayedServerToolUse {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    #[serde(default)]
    pub input: ReplayedSearchQuery,
    #[serde(flatten)]
    pub ignored: IgnoredAny,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TextKind {
    Text,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TextBlock {
    #[serde(rename = "type")]
    pub kind: TextKind,
    pub text: String,
}

impl From<TextBlock> for Json {
    fn from(block: TextBlock) -> Self {
        let TextKind::Text = block.kind;
        Self::Object(IndexMap::from([
            ("type".to_owned(), Self::Text("text".to_owned())),
            ("text".to_owned(), Self::Text(block.text)),
        ]))
    }
}

impl WebSearchContent {
    fn has_expected_types(&self) -> bool {
        match self {
            Self::Results(results) => results
                .iter()
                .all(|result| result.kind == "web_search_result"),
            Self::Error(error) => error.kind == "web_search_tool_result_error",
        }
    }

    fn has_encrypted_result(&self) -> bool {
        match self {
            Self::Results(results) => results
                .iter()
                .any(|result| !result.encrypted_content.is_empty()),
            Self::Error(_) => false,
        }
    }
}

#[must_use]
pub fn flatten_web_search_results(messages: &[Json]) -> Vec<Json> {
    let mut flattened = Vec::with_capacity(messages.len());
    for message in messages {
        flattened.push(flatten_message(message));
    }
    flattened
}

fn flatten_message(message: &Json) -> Json {
    let Json::Object(fields) = message else {
        return message.clone();
    };
    let Some(Json::List(content)) = fields.get("content") else {
        return message.clone();
    };
    let flattenable = flattenable_tool_use_ids(content);
    if flattenable.is_empty() {
        return message.clone();
    }
    let queries = replayed_queries(content);
    let mut rewritten = Vec::with_capacity(content.len());
    for block in content {
        if let Some(flattened_block) = rewrite_replayed_block(block, &flattenable, &queries) {
            rewritten.push(flattened_block);
        }
    }
    let mut flattened = fields.clone();
    flattened.insert("content".to_owned(), Json::List(rewritten));
    Json::Object(flattened)
}

fn flattenable_tool_use_ids(content: &[Json]) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for block in content {
        if let Some(parsed) = flattenable_search_result(block) {
            ids.insert(parsed.tool_use_id);
        }
    }
    ids
}

fn replayed_queries(content: &[Json]) -> BTreeMap<String, String> {
    let mut queries = BTreeMap::new();
    for block in content {
        if let Some(parsed) = replayed_server_tool_use(block) {
            queries.insert(parsed.id, parsed.input.query);
        }
    }
    queries
}

fn rewrite_replayed_block(
    block: &Json,
    flattenable: &BTreeSet<String>,
    queries: &BTreeMap<String, String>,
) -> Option<Json> {
    if let Some(parsed) = flattenable_search_result(block) {
        let query = queries.get(&parsed.tool_use_id).map_or("", String::as_str);
        let text = render_web_search_results(query, &parsed.content);
        return Some(Json::from(TextBlock {
            kind: TextKind::Text,
            text,
        }));
    }
    let replayed = replayed_server_tool_use(block);
    if replayed.is_some_and(|parsed| flattenable.contains(&parsed.id)) {
        return None;
    }
    Some(block.clone())
}

fn flattenable_search_result(block: &Json) -> Option<ReplayedWebSearchToolResult> {
    let parsed: ReplayedWebSearchToolResult = parse_block(block)?;
    if parsed.kind != "web_search_tool_result" || !parsed.content.has_expected_types() {
        return None;
    }
    if parsed.content.has_encrypted_result() {
        return None;
    }
    Some(parsed)
}

fn replayed_server_tool_use(block: &Json) -> Option<ReplayedServerToolUse> {
    let parsed: ReplayedServerToolUse = parse_block(block)?;
    if parsed.kind != "server_tool_use" {
        return None;
    }
    Some(parsed)
}

// serde decodes a struct from a JSON array too, so a mapping guard checks the object shape first
fn parse_block<Block>(block: &Json) -> Option<Block>
where
    Block: DeserializeOwned,
{
    let Json::Object(_) = block else {
        return None;
    };
    let source = serde_json::to_string(block).ok()?;
    let parsed = serde_json::from_str(&source).ok()?;
    Some(parsed)
}

fn render_web_search_results(query: &str, content: &WebSearchContent) -> String {
    let header = if query.is_empty() {
        "Web search results:".to_owned()
    } else {
        format!("Web search results for '{query}':")
    };
    match content {
        WebSearchContent::Error(error) => render_error(&header, error),
        WebSearchContent::Results(results) => render_results(&header, results),
    }
}

fn render_error(header: &str, error: &WebSearchToolResultError) -> String {
    let code = if error.error_code.is_empty() {
        "unavailable"
    } else {
        error.error_code.as_str()
    };
    format!("{header}\n\nSearch failed: {code}")
}

fn render_results(header: &str, results: &[ReplayedWebSearchResult]) -> String {
    if results.is_empty() {
        return format!("{header}\n\nNo results were returned.");
    }
    let body = results
        .iter()
        .map(render_result)
        .collect::<Vec<_>>()
        .join("\n\n");
    if body.is_empty() {
        return header.to_owned();
    }
    format!("{header}\n\n{body}")
}

fn render_result(result: &ReplayedWebSearchResult) -> String {
    let mut lines = Vec::new();
    if !result.title.is_empty() {
        lines.push(format!("Title: {}", result.title));
    }
    if !result.url.is_empty() {
        lines.push(format!("URL: {}", result.url));
    }
    if !result.snippet.is_empty() {
        lines.push(format!("Snippet: {}", result.snippet));
    }
    lines.join("\n")
}

mod tests;
