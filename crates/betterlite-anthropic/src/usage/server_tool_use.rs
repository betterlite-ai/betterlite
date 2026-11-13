use std::error::Error as StdError;

use betterlite_models::decisions::Json;

use super::ServerToolUse;
use super::count::optional_int;
use super::error::UsageError;
use super::wire::{ServerToolUseValue, UsageObject};
use crate::response_content::wire::{get, json_text};

pub(super) fn resolve_server_tool_use<CounterFailure>(
    object: &UsageObject,
    completion_response: Option<&Json>,
) -> Result<Option<ServerToolUse>, UsageError<CounterFailure>>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    let mut web_search = None;
    let mut tool_search = None;
    match &object.server_tool_use {
        None | Some(ServerToolUseValue::Other) => {}
        Some(ServerToolUseValue::Mapping(usage)) => {
            web_search = optional_int(
                usage.web_search_requests.as_ref(),
                "server_tool_use.web_search_requests",
            )?;
            tool_search = optional_int(
                usage.tool_search_requests.as_ref(),
                "server_tool_use.tool_search_requests",
            )?;
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

fn content_tool_search_count<CounterFailure>(
    completion_response: Option<&Json>,
) -> Result<Option<i64>, UsageError<CounterFailure>>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
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
            return Err(UsageError::FieldType {
                field: "content.name",
            });
        };
        if tool_name.contains("tool_search") {
            count = count.checked_add(1).ok_or(UsageError::Overflow {
                field: "server_tool_use.tool_search_requests",
            })?;
        }
    }
    Ok((count > 0).then_some(count))
}
