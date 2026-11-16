use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;

use crate::databricks_chat::error::DatabricksChatError;

// LiteLLM's ChatCompletionThinkingBlock, the one block shape both the reply and the delta emit
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThinkingBlock {
    #[serde(rename = "type")]
    pub kind: String,
    pub thinking: String,
    pub signature: Option<String>,
}

pub fn extract_content_str(content: Option<&Json>) -> Result<Option<String>, DatabricksChatError> {
    match content {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::List(items)) => Ok(Some(text_of_blocks(items)?)),
        Some(Json::Bool(_) | Json::Number(_) | Json::Object(_)) => Err(unsupported_content()),
    }
}

fn text_of_blocks(items: &[Json]) -> Result<String, DatabricksChatError> {
    let mut collected = String::new();
    for item in items {
        let Some(fields) = item.as_object() else {
            return Err(DatabricksChatError::Shape {
                message: "a content block must be an object".to_owned(),
            });
        };
        if fields.get("type").and_then(Json::as_str) != Some("text") {
            continue;
        }
        match fields.get("text") {
            None | Some(Json::Null) => {}
            Some(Json::Text(text)) => collected.push_str(text),
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                return Err(DatabricksChatError::Shape {
                    message: "the text of a text content block is not a string".to_owned(),
                });
            }
        }
    }
    Ok(collected)
}

// extract_reasoning_content: reasoning items with summaries become content text and thinking blocks
pub fn extract_reasoning(
    content: Option<&Json>,
) -> Result<(Option<String>, Option<Vec<ThinkingBlock>>), DatabricksChatError> {
    let Some(content) = content else {
        return Ok((None, None));
    };
    let Json::List(items) = content else {
        return Ok((None, None));
    };
    let mut reasoning = String::new();
    let mut blocks = Vec::new();
    for item in items {
        let Some(fields) = item.as_object() else {
            return Err(DatabricksChatError::Shape {
                message: "a content block must be an object".to_owned(),
            });
        };
        if fields.get("type").and_then(Json::as_str) != Some("reasoning") {
            continue;
        }
        let Some(Json::List(summaries)) = fields.get("summary") else {
            continue;
        };
        for summary in summaries {
            let Some(summary_fields) = summary.as_object() else {
                return Err(DatabricksChatError::Shape {
                    message: "a reasoning summary must be an object".to_owned(),
                });
            };
            let text = required_text(summary_fields, "text")?;
            reasoning.push_str(text);
            blocks.push(ThinkingBlock {
                kind: "thinking".to_owned(),
                thinking: text.to_owned(),
                signature: optional_text(summary_fields, "signature")?,
            });
        }
    }
    let reasoning = if blocks.is_empty() {
        None
    } else {
        Some(reasoning)
    };
    let blocks = if blocks.is_empty() {
        None
    } else {
        Some(blocks)
    };
    Ok((reasoning, blocks))
}

// Python reaches these keys with [], so an absent or null text is an error rather than ""
fn required_text<'value>(
    fields: &'value Object,
    name: &str,
) -> Result<&'value str, DatabricksChatError> {
    let Some(Json::Text(text)) = fields.get(name) else {
        return Err(DatabricksChatError::Shape {
            message: format!("the {name} of a reasoning summary must be a string"),
        });
    };
    Ok(text)
}

// Python reaches the signature with .get(..., ""), so a missing one is "" and a null one stays null
fn optional_text(fields: &Object, name: &str) -> Result<Option<String>, DatabricksChatError> {
    match fields.get(name) {
        None => Ok(Some(String::new())),
        Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(DatabricksChatError::Shape {
                message: format!("the {name} of a reasoning summary must be a string"),
            })
        }
    }
}

// extract_citations: each cited text block becomes a list of citations carrying its text
pub fn extract_citations(content: Option<&Json>) -> Result<Option<Vec<Json>>, DatabricksChatError> {
    let Some(Json::List(items)) = content else {
        return Ok(None);
    };
    let mut citations: Vec<Json> = Vec::new();
    for item in items {
        let Some(fields) = item.as_object() else {
            return Err(DatabricksChatError::Shape {
                message: "a content block must be an object".to_owned(),
            });
        };
        let Some(value) = fields.get("citations") else {
            continue;
        };
        if !value.is_truthy() {
            continue;
        }
        let Json::List(entries) = value else {
            return Err(DatabricksChatError::Shape {
                message: "the citations of a content block must be a list".to_owned(),
            });
        };
        let supported_text = match fields.get("text") {
            None => Json::Null,
            Some(text) => text.clone(),
        };
        let mut supported = Vec::with_capacity(entries.len());
        for entry in entries {
            let Some(entry_fields) = entry.as_object() else {
                return Err(DatabricksChatError::Shape {
                    message: "a citation must be an object".to_owned(),
                });
            };
            let mut citation = entry_fields.clone();
            citation.insert("supported_text".to_owned(), supported_text.clone());
            supported.push(Json::Object(citation));
        }
        citations.push(Json::List(supported));
    }
    if citations.is_empty() {
        return Ok(None);
    }
    Ok(Some(citations))
}

fn unsupported_content() -> DatabricksChatError {
    DatabricksChatError::Shape {
        message: "Unsupported content type".to_owned(),
    }
}
