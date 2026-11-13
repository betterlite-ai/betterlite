use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::message_content::image::anthropic_image_source_url;
use serde::Serialize;

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct ImageUrl {
    pub url: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolResultPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
    ToolReference { tool_name: String },
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ToolResultContent {
    Text(String),
    Parts(Vec<ToolResultPart>),
}

#[must_use]
pub fn image_source_url(image_source: &Json) -> Option<String> {
    match image_source {
        Json::Object(source) => anthropic_image_source_url(source),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

// A url source with a non-string url projects to an empty url, and an empty url drops the part.
#[must_use]
pub fn tool_result_image_part(image_source: Option<&Json>) -> Option<ToolResultPart> {
    let url = image_source
        .and_then(image_source_url)
        .filter(|url| !url.is_empty())?;
    Some(ToolResultPart::ImageUrl {
        image_url: ImageUrl { url },
    })
}

// An empty value of any type becomes an empty string, as LiteLLM's `or ""` does.
fn text_of(value: &Json) -> String {
    if !value.is_truthy() {
        return String::new();
    }
    match value {
        Json::Text(text) => text.clone(),
        Json::Number(number) => number.to_string(),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => {
            serde_json::to_string(value).unwrap_or_default()
        }
    }
}

#[must_use]
pub fn tool_result_part(item: &Json) -> Option<ToolResultPart> {
    let block = match item {
        Json::Text(text) => {
            return Some(ToolResultPart::Text { text: text.clone() });
        }
        Json::Object(block) => block,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => return None,
    };
    match block.get("type").and_then(Json::as_str) {
        Some("text") => Some(ToolResultPart::Text {
            text: block.get("text").map(text_of).unwrap_or_default(),
        }),
        Some("image" | "document") => tool_result_image_part(block.get("source")),
        Some("tool_reference") => Some(ToolResultPart::ToolReference {
            tool_name: block.get("tool_name").map(text_of).unwrap_or_default(),
        }),
        Some(_) | None => None,
    }
}

#[must_use]
pub fn tool_result_content(raw_content: &Json) -> ToolResultContent {
    let items = match raw_content {
        Json::Text(text) => return ToolResultContent::Text(text.clone()),
        Json::List(items) => items,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => {
            return ToolResultContent::Text(String::new());
        }
    };
    let parts: Vec<ToolResultPart> = items.iter().filter_map(tool_result_part).collect();
    match parts.as_slice() {
        [] => ToolResultContent::Text(String::new()),
        [ToolResultPart::Text { text }] => ToolResultContent::Text(text.clone()),
        _ => ToolResultContent::Parts(parts),
    }
}
