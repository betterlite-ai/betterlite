#![cfg(test)]

mod chat_tool_conversion;
mod config_init;
mod native_blocks_flag;
mod provider_filter;
mod responses_tool_conversion;
mod stream_conversion;
mod tool_choice_rewrite;

use betterlite_prompts::arguments::json::{Json, Object};

use super::WebSearchInterceptionLogger;

pub(crate) fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

pub(crate) fn object(source: &str) -> Object {
    match json(source) {
        Json::Object(fields) => fields,
        other => panic!("{source} must be an object, got {other:?}"),
    }
}

pub(crate) fn logger(providers: &[&str]) -> WebSearchInterceptionLogger {
    WebSearchInterceptionLogger {
        enabled_providers: Some(providers.iter().map(|name| (*name).to_owned()).collect()),
        search_tool_name: None,
        max_agentic_loops: None,
    }
}

pub(crate) fn list<'request>(request: &'request Object, key: &str) -> &'request [Json] {
    match request.get(key) {
        Some(Json::List(items)) => items,
        other => panic!("{key} must be a list, got {other:?}"),
    }
}

pub(crate) fn tool_name(tool: &Json) -> Option<&str> {
    let fields = tool.as_object()?;
    if let Some(nested) = fields.get("function").and_then(Json::as_object) {
        return nested.get("name").and_then(Json::as_str);
    }
    fields.get("name").and_then(Json::as_str)
}
