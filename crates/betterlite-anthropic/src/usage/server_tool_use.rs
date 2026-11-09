use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::ServerToolUse;
use super::shape::{UsageShapeError, optional_count};
use crate::response_content::wire::{get, json_text};

pub(super) fn resolve_server_tool_use(
    fields: &IndexMap<String, Json>,
    completion_response: Option<&Json>,
) -> Result<Option<ServerToolUse>, UsageShapeError> {
    let mut web_search = None;
    let mut tool_search = None;
    match fields.get("server_tool_use") {
        None | Some(Json::Null) => {}
        Some(Json::Object(usage)) => {
            web_search = optional_count(
                usage.get("web_search_requests"),
                "server_tool_use.web_search_requests",
            )?;
            tool_search = optional_count(
                usage.get("tool_search_requests"),
                "server_tool_use.tool_search_requests",
            )?;
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(UsageShapeError::FieldType {
                field: "server_tool_use",
            });
        }
    }
    if tool_search.is_none() {
        tool_search = content_tool_search_count(completion_response)?;
    }
    if web_search.is_none() && tool_search.is_none() {
        return Ok(None);
    }
    Ok(Some(ServerToolUse {
        web_search,
        tool_search,
        browser_open: None,
    }))
}

fn content_tool_search_count(
    completion_response: Option<&Json>,
) -> Result<Option<i64>, UsageShapeError> {
    let Some(Json::Object(response)) = completion_response else {
        return Ok(None);
    };
    let Some(Json::List(content)) = response.get("content") else {
        return Ok(None);
    };
    let mut count = 0_i64;
    for block in content {
        let Some(kind) = get(block, "type").and_then(json_text) else {
            continue;
        };
        if kind != "server_tool_use" {
            continue;
        }
        let Some(name) = get(block, "name") else {
            continue;
        };
        let Some(tool_name) = json_text(name) else {
            return Err(UsageShapeError::FieldType {
                field: "content.name",
            });
        };
        if tool_name.contains("tool_search") {
            count = count.checked_add(1).ok_or(UsageShapeError::Overflow {
                field: "server_tool_use.tool_search_requests",
            })?;
        }
    }
    Ok((count > 0).then_some(count))
}
