use serde::Serialize;
use serde_json::value::RawValue;

use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;
use crate::cisco_mcp::blocking::StructuredMirror;

#[derive(Debug, Clone, Serialize)]
pub struct TextUpdate {
    pub response: RewrittenResponse,
    pub replaced: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum RewrittenResponse {
    Null,
    Bool(bool),
    Number(Box<RawValue>),
    Text(String),
    Unchanged(Fields),
    Wrapper {
        #[serde(flatten)]
        rest: Fields,
        mcp_tool_call_response: Box<Self>,
    },
    Model {
        #[serde(flatten)]
        rest: Fields,
        content: Vec<ContentItem>,
        #[serde(flatten)]
        mirror: MirrorField,
    },
    Dictionary {
        #[serde(flatten)]
        rest: Fields,
        #[serde(flatten)]
        mirror: MirrorField,
    },
    Result {
        #[serde(flatten)]
        rest: Fields,
        result: Box<Self>,
    },
    Pairs(Vec<(String, PairValue)>),
    Content(Vec<ContentItem>),
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum MirrorField {
    Snake {
        structured_content: StructuredMirror,
    },
    Camel {
        #[serde(rename = "structuredContent")]
        structured_content: StructuredMirror,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ContentItem {
    Unchanged(Box<RawValue>),
    Text {
        #[serde(flatten)]
        rest: Fields,
        text: String,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum PairValue {
    Unchanged(Box<RawValue>),
    Content(Vec<ContentItem>),
    Mirror(StructuredMirror),
}

#[must_use]
pub fn set_tool_response_text(response: &Json, text: &str) -> TextUpdate {
    let preserved = match response {
        Json::Object(fields) => return rewrite_object(fields.clone(), text),
        Json::List(items) => {
            let pairs = items
                .iter()
                .map(|raw| serde_json::from_str(raw.get()))
                .collect::<Result<Vec<(String, Box<RawValue>)>, _>>();
            let mut replaced = false;
            let rewritten = match pairs {
                Ok(fields) if !fields.is_empty() => {
                    RewrittenResponse::Pairs(rewrite_pairs(fields, text, &mut replaced))
                }
                Ok(_) | Err(_) => {
                    let content = rewrite_items(items, text);
                    replaced = content
                        .iter()
                        .any(|item| matches!(item, ContentItem::Text { .. }));
                    RewrittenResponse::Content(content)
                }
            };
            return TextUpdate {
                response: rewritten,
                replaced,
            };
        }
        Json::Null => RewrittenResponse::Null,
        Json::Bool(value) => RewrittenResponse::Bool(*value),
        Json::Number(value) => {
            // Json numbers originate in validated raw JSON; keep their exact numeric representation.
            RawValue::from_string(value.clone())
                .map_or(RewrittenResponse::Null, RewrittenResponse::Number)
        }
        Json::Text(value) => RewrittenResponse::Text(value.clone()),
    };
    TextUpdate {
        response: preserved,
        replaced: false,
    }
}

fn rewrite_object(mut fields: Fields, text: &str) -> TextUpdate {
    if let Some(raw) = fields.get("mcp_tool_call_response") {
        let inner = Json::parse(raw.get());
        if !matches!(inner, Json::Null) {
            let update = set_tool_response_text(&inner, text);
            return enclose(fields, Enclosure::Wrapper, update);
        }
    }
    if let Json::List(items) = Json::parse(fields.get("content").map_or("null", |raw| raw.get())) {
        let content = rewrite_items(&items, text);
        fields.shift_remove("content");
        let mirror = take_mirror(&mut fields, text);
        return TextUpdate {
            response: RewrittenResponse::Model {
                rest: fields,
                content,
                mirror,
            },
            replaced: true,
        };
    }
    if let Json::Object(result) = Json::parse(fields.get("result").map_or("null", |raw| raw.get()))
    {
        return enclose(fields, Enclosure::Result, rewrite_mirror(result, text));
    }
    rewrite_mirror(fields, text)
}

#[derive(Clone, Copy)]
enum Enclosure {
    Wrapper,
    Result,
}

fn enclose(mut fields: Fields, enclosure: Enclosure, mut update: TextUpdate) -> TextUpdate {
    let key = match enclosure {
        Enclosure::Wrapper => "mcp_tool_call_response",
        Enclosure::Result => "result",
    };
    fields.shift_remove(key);
    let inner = Box::new(update.response);
    update.response = match enclosure {
        Enclosure::Wrapper => RewrittenResponse::Wrapper {
            rest: fields,
            mcp_tool_call_response: inner,
        },
        Enclosure::Result => RewrittenResponse::Result {
            rest: fields,
            result: inner,
        },
    };
    update
}

fn rewrite_mirror(mut fields: Fields, text: &str) -> TextUpdate {
    if fields.contains_key("structured_content") || fields.contains_key("structuredContent") {
        let mirror = take_mirror(&mut fields, text);
        TextUpdate {
            response: RewrittenResponse::Dictionary {
                rest: fields,
                mirror,
            },
            replaced: true,
        }
    } else {
        TextUpdate {
            response: RewrittenResponse::Unchanged(fields),
            replaced: false,
        }
    }
}

fn take_mirror(fields: &mut Fields, text: &str) -> MirrorField {
    let structured_content = StructuredMirror {
        result: text.to_owned(),
    };
    if fields.shift_remove("structured_content").is_some() {
        MirrorField::Snake { structured_content }
    } else {
        fields.shift_remove("structuredContent");
        MirrorField::Camel { structured_content }
    }
}

fn rewrite_pairs(
    pairs: Vec<(String, Box<RawValue>)>,
    text: &str,
    replaced: &mut bool,
) -> Vec<(String, PairValue)> {
    // LiteLLM converts pairs to a dict to find content, but rewrites every mirror pair.
    let content_index = pairs.iter().rposition(|(key, _)| key == "content");
    pairs
        .into_iter()
        .enumerate()
        .map(|(index, (key, value))| {
            let updated = if matches!(key.as_str(), "structuredContent" | "structured_content") {
                *replaced = true;
                PairValue::Mirror(StructuredMirror {
                    result: text.to_owned(),
                })
            } else if Some(index) == content_index {
                if let Json::List(items) = Json::parse(value.get()) {
                    let content = rewrite_items(&items, text);
                    *replaced |= content
                        .iter()
                        .any(|item| matches!(item, ContentItem::Text { .. }));
                    PairValue::Content(content)
                } else {
                    PairValue::Unchanged(value)
                }
            } else {
                PairValue::Unchanged(value)
            };
            (key, updated)
        })
        .collect()
}

fn rewrite_items(items: &[Box<RawValue>], text: &str) -> Vec<ContentItem> {
    items.iter().map(|raw| {
        if let Json::Object(mut fields) = Json::parse(raw.get())
            && matches!(Json::parse(fields.get("type").map_or("null", |kind| kind.get())), Json::Text(kind) if kind == "text")
        {
            fields.shift_remove("text");
            ContentItem::Text { rest: fields, text: text.to_owned() }
        } else {
            ContentItem::Unchanged(raw.clone())
        }
    }).collect()
}

mod tests;
