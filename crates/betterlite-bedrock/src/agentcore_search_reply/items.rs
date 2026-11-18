use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::records::SearchResult;
use indexmap::IndexMap;

use crate::agentcore_search_reply::error::SearchReplyError;

pub(crate) fn text_blocks(response: &IndexMap<String, Json>) -> Vec<&IndexMap<String, Json>> {
    let Some(Json::Object(result)) = response.get("result") else {
        return Vec::new();
    };
    let Some(Json::List(content)) = result.get("content") else {
        return Vec::new();
    };
    let mut blocks = Vec::new();
    for block in content {
        if let Json::Object(fields) = block
            && matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "text")
        {
            blocks.push(fields);
        }
    }
    blocks
}

pub(crate) fn text_items(response: &IndexMap<String, Json>) -> Vec<IndexMap<String, Json>> {
    let mut items = Vec::new();
    for block in text_blocks(response) {
        items.extend(parse_result_items(block));
    }
    items
}

pub(crate) fn structured_items(result: Option<&Json>) -> Vec<IndexMap<String, Json>> {
    let Some(Json::Object(fields)) = result else {
        return Vec::new();
    };
    let Some(structured) = fields.get("structuredContent") else {
        return Vec::new();
    };
    result_items(structured)
}

pub(crate) fn to_search_result(item: &IndexMap<String, Json>) -> SearchResult {
    SearchResult {
        title: first_string(item, &["title"]).unwrap_or_default(),
        url: first_string(item, &["url"]).unwrap_or_default(),
        snippet: first_string(item, &["text", "snippet"]).unwrap_or_default(),
        date: first_string(item, &["publishedDate", "date"]),
        last_updated: None,
        extra: Object::new(),
    }
}

pub(crate) fn tool_error_detail(
    response: &IndexMap<String, Json>,
    result: &IndexMap<String, Json>,
) -> Result<String, SearchReplyError> {
    let mut texts = Vec::new();
    for block in text_blocks(response) {
        if let Some(Json::Text(text)) = block.get("text") {
            texts.push(text.as_str());
        }
    }
    if texts.is_empty() {
        let rendered =
            serde_json::to_string(result).map_err(|source| SearchReplyError::Encode { source })?;
        return Ok(rendered.chars().take(500).collect());
    }
    Ok(texts.join(" "))
}

fn parse_result_items(block: &IndexMap<String, Json>) -> Vec<IndexMap<String, Json>> {
    let Some(Json::Text(text)) = block.get("text") else {
        return Vec::new();
    };
    let Ok(parsed) = serde_json::from_str::<Json>(text) else {
        return Vec::new();
    };
    result_items(&parsed)
}

fn result_items(parsed: &Json) -> Vec<IndexMap<String, Json>> {
    let candidate = match parsed {
        Json::Object(fields) => {
            let Some(results) = fields.get("results") else {
                return Vec::new();
            };
            results
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => parsed,
    };
    let Json::List(items) = candidate else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match item {
            Json::Object(fields) => Some(fields.clone()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
        })
        .collect()
}

fn first_string(item: &IndexMap<String, Json>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| match item.get(*key) {
        Some(Json::Text(text)) if !text.is_empty() => Some(text.clone()),
        None | Some(_) => None,
    })
}
