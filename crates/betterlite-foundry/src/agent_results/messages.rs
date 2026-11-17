use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;

pub const ASSISTANT_ROLE: &str = "assistant";
pub const TEXT_TYPE: &str = "text";
pub const URL_CITATION_TYPE: &str = "url_citation";

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MessagesData {
    #[serde(default)]
    pub data: Vec<AgentMessage>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AgentMessage {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub content: Vec<ContentItem>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ContentItem {
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub text: Option<TextBlock>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TextBlock {
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub annotations: Option<Vec<Object>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtractedContent {
    pub content: String,
    pub annotations: Option<Vec<Object>>,
}

// LiteLLM returns the first text block of the first assistant message, even when that text is empty
#[must_use]
pub fn extract_content(messages: &MessagesData) -> ExtractedContent {
    for message in &messages.data {
        if message.role.as_deref() != Some(ASSISTANT_ROLE) {
            continue;
        }
        for item in &message.content {
            if item.kind.as_deref() != Some(TEXT_TYPE) {
                continue;
            }
            let block = item.text.as_ref();
            return ExtractedContent {
                content: block
                    .and_then(|text| text.value.clone())
                    .unwrap_or_default(),
                annotations: transform_annotations(
                    block.and_then(|text| text.annotations.as_deref()),
                ),
            };
        }
    }
    ExtractedContent::default()
}

// a falsey annotation list becomes None; url_citation outputs move the indexes inside the citation
#[must_use]
pub fn transform_annotations(raw: Option<&[Object]>) -> Option<Vec<Object>> {
    let annotations = raw.filter(|values| !values.is_empty())?;
    let mut result = Vec::with_capacity(annotations.len());
    for annotation in annotations {
        if annotation.get("type").and_then(Json::as_str) == Some(URL_CITATION_TYPE) {
            result.push(transform_url_citation(annotation));
        } else {
            result.push(annotation.clone());
        }
    }
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

fn transform_url_citation(annotation: &Object) -> Object {
    let mut citation = annotation
        .get("url_citation")
        .and_then(Json::as_object)
        .cloned()
        .unwrap_or_default();
    relocate_index(annotation, &mut citation, "start_index");
    relocate_index(annotation, &mut citation, "end_index");
    let mut transformed = Object::new();
    transformed.insert(
        String::from("type"),
        Json::Text(String::from(URL_CITATION_TYPE)),
    );
    transformed.insert(String::from("url_citation"), Json::Object(citation));
    transformed
}

// an index the citation already holds wins, including an explicit null
fn relocate_index(annotation: &Object, citation: &mut Object, key: &str) {
    let Some(value) = annotation.get(key) else {
        return;
    };
    if citation.contains_key(key) {
        return;
    }
    citation.insert(String::from(key), value.clone());
}
