use serde_json::{Map, Value};
use thiserror::Error;

use crate::compatible_wire::error::CompatibleWireError;
use crate::compatible_wire::{CompatibleReply, ReplyOptions, decode_compatible_chat_reply};

use super::PROVIDER;
use super::error::GroqChatError;

const SEARCH_ACTIONS: [&str; 2] = ["browser.search", "browser_search"];
const OPEN_ACTION: &str = "browser.open";

#[derive(Debug, Error)]
enum ActionShapeError {
    #[error("executed_tools is not a list: {value}")]
    NotList { value: Value },
    #[error("an executed_tools entry is not an object: {entry}")]
    Entry { entry: Value },
    #[error("an executed_tools {name} is neither a string nor null: {value}")]
    Field { name: &'static str, value: Value },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrowserActions {
    pub web_search_requests: u64,
    pub browser_open_requests: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroqReply {
    pub reply: CompatibleReply,
    // absent leaves a provider server_tool_use exactly as it arrived
    pub browser_actions: Option<BrowserActions>,
}

pub fn transform_response(
    body: &[u8],
    options: &ReplyOptions<'_>,
) -> Result<GroqReply, GroqChatError> {
    let mut reply = decode_compatible_chat_reply(body, options)?;
    reply.response.service_tier =
        Some(mapped_service_tier(reply.response.service_tier.as_deref()).to_owned());
    let raw: Value = serde_json::from_slice(body)
        .map_err(|source| GroqChatError::Reply(CompatibleWireError::Json(source)))?;
    Ok(GroqReply {
        browser_actions: browser_actions(&raw),
        reply,
    })
}

fn mapped_service_tier(tier: Option<&str>) -> &'static str {
    match tier {
        Some("auto") => "auto",
        Some("default") => "default",
        Some("flex") => "flex",
        Some(_) | None => "auto",
    }
}

// only a reply that carries usage is billed, and a bad shape anywhere voids the whole count
fn browser_actions(raw: &Value) -> Option<BrowserActions> {
    if raw.get("usage").is_none_or(Value::is_null) {
        return None;
    }
    let Some(choices) = raw.get("choices").and_then(Value::as_array) else {
        return None;
    };
    let mut actions = Vec::new();
    for choice in choices {
        let executed = choice
            .get("message")
            .and_then(|message| message.get("executed_tools"));
        match executed_actions(executed) {
            Ok(found) => actions.extend(found),
            Err(error) => {
                tracing::info!(
                    provider = PROVIDER,
                    %error,
                    "Groq executed_tools entries did not match the expected shape; not billed"
                );
                return None;
            }
        }
    }
    let mut searches = 0_u64;
    let mut opens = 0_u64;
    for action in actions.iter().filter_map(Option::as_deref) {
        if SEARCH_ACTIONS.contains(&action) {
            searches = searches.saturating_add(1);
        } else if action == OPEN_ACTION {
            opens = opens.saturating_add(1);
        }
    }
    if searches == 0 && opens == 0 {
        return None;
    }
    Some(BrowserActions {
        web_search_requests: searches,
        browser_open_requests: opens,
    })
}

fn executed_actions(executed: Option<&Value>) -> Result<Vec<Option<String>>, ActionShapeError> {
    let Some(executed) = executed.filter(|value| !value.is_null()) else {
        return Ok(Vec::new());
    };
    let Some(entries) = executed.as_array() else {
        return Err(ActionShapeError::NotList {
            value: executed.clone(),
        });
    };
    let mut actions = Vec::with_capacity(entries.len());
    for entry in entries {
        let Some(fields) = entry.as_object() else {
            return Err(ActionShapeError::Entry {
                entry: entry.clone(),
            });
        };
        let name = identity_field(fields, "name")?;
        let kind = identity_field(fields, "type")?;
        actions.push(name.filter(|name| !name.is_empty()).or(kind));
    }
    Ok(actions)
}

fn identity_field(
    fields: &Map<String, Value>,
    name: &'static str,
) -> Result<Option<String>, ActionShapeError> {
    match fields.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(other) => Err(ActionShapeError::Field {
            name,
            value: other.clone(),
        }),
    }
}
