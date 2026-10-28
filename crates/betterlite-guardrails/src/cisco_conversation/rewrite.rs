use serde::Serialize;
use serde_json::value::RawValue;

use crate::cisco_conversation::json::Json;
use crate::cisco_conversation::{Fields, Role, caller_role, is_text_part};

#[derive(Debug, Clone, Serialize)]
pub struct InputMessage {
    pub role: Role,
    pub content: InputContent,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum InputContent {
    Text([TextPart; 1]),
    Parts(Vec<Box<RawValue>>),
}

#[derive(Debug, Clone, Serialize)]
pub struct TextPart {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum RewrittenInput {
    Text(String),
    Items(Vec<RewrittenItem>),
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum RewrittenItem {
    Kept(Box<RawValue>),
    Edited {
        #[serde(flatten)]
        fields: Fields,
        #[serde(flatten)]
        edit: Edit,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Edit {
    Text(String),
    Content(EditedContent),
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum EditedContent {
    Text(String),
    Parts(Vec<RewrittenItem>),
}

#[must_use]
pub fn messages_to_input(messages: &[Box<RawValue>]) -> Option<Vec<InputMessage>> {
    let mut input = Vec::new();
    for message in messages {
        let Json::Object(fields) = Json::parse(message.get()) else {
            continue;
        };
        let role = caller_role(&fields).unwrap_or(Role::User);
        let content = match Json::field(&fields, "content") {
            Json::Text(text) => {
                let assistant = matches!(&role, Role::Caller(raw) if matches!(Json::parse(raw.get()), Json::Text(name) if name == "assistant"));
                let kind = if assistant {
                    "output_text"
                } else {
                    "input_text"
                };
                InputContent::Text([TextPart { kind, text }])
            }
            Json::List(parts) => InputContent::Parts(parts),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => continue,
        };
        input.push(InputMessage { role, content });
    }
    (!input.is_empty()).then_some(input)
}

#[must_use]
pub fn rewrite_input_text(original: &Json, sanitized: &str) -> Option<RewrittenInput> {
    match original {
        Json::Text(_) => Some(RewrittenInput::Text(sanitized.to_owned())),
        Json::List(items)
            if items.iter().any(|item| {
                matches!(Json::parse(item.get()), Json::Object(fields) if fields.contains_key("role"))
            }) =>
        {
            rewrite_last_user(items, sanitized).map(RewrittenInput::Items)
        }
        Json::List(items) => rewrite_matching(
            items,
            SearchOrder::First,
            is_text_part,
            |mut part| {
                part.shift_remove("text");
                Some((part, Edit::Text(sanitized.to_owned())))
            },
        )
        .map(RewrittenInput::Items),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => None,
    }
}

fn rewrite_last_user(items: &[Box<RawValue>], sanitized: &str) -> Option<Vec<RewrittenItem>> {
    let last_position = items.len().checked_sub(1)?;
    items.get(last_position)?;
    rewrite_matching(
        items,
        SearchOrder::Last,
        |message| matches!(Json::field(message, "role"), Json::Text(role) if role == "user"),
        |mut message| {
            let content = rewrite_content(&message, sanitized)?;
            message.shift_remove("content");
            Some((message, Edit::Content(content)))
        },
    )
}

fn rewrite_content(message: &Fields, sanitized: &str) -> Option<EditedContent> {
    match Json::field(message, "content") {
        Json::Text(_) => Some(EditedContent::Text(sanitized.to_owned())),
        Json::List(parts) => {
            rewrite_matching(&parts, SearchOrder::First, is_text_part, |mut part| {
                part.shift_remove("text");
                Some((part, Edit::Text(sanitized.to_owned())))
            })
            .map(EditedContent::Parts)
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => None,
    }
}

fn rewrite_matching<Matches, EditFields>(
    items: &[Box<RawValue>],
    order: SearchOrder,
    matches: Matches,
    edit: EditFields,
) -> Option<Vec<RewrittenItem>>
where
    Matches: Fn(&Fields) -> bool,
    EditFields: Fn(Fields) -> Option<(Fields, Edit)>,
{
    let mut positions: Vec<usize> = (0..items.len()).collect();
    if matches!(order, SearchOrder::Last) {
        positions.reverse();
    }
    for position in positions {
        let Some(item) = items.get(position) else {
            continue;
        };
        let Json::Object(original_fields) = Json::parse(item.get()) else {
            continue;
        };
        if !matches(&original_fields) {
            continue;
        }
        let Some((edited_fields, replacement)) = edit(original_fields) else {
            continue;
        };
        return replace(items, position, edited_fields, replacement);
    }
    None
}

#[derive(Clone, Copy)]
enum SearchOrder {
    First,
    Last,
}

fn replace(
    items: &[Box<RawValue>],
    position: usize,
    fields: Fields,
    edit: Edit,
) -> Option<Vec<RewrittenItem>> {
    let mut rewritten: Vec<RewrittenItem> =
        items.iter().cloned().map(RewrittenItem::Kept).collect();
    let slot = rewritten.get_mut(position)?;
    *slot = RewrittenItem::Edited { fields, edit };
    Some(rewritten)
}
