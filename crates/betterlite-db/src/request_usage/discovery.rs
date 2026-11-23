use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::autorouter_facts::counts_as_present;
use crate::tool_discovery::{ToolDiscoveryItem, ToolDiscoveryQueue};

use super::payload::DiscoveryKwargs;

mod mcp;

use self::mcp::mcp_call_metadata;

const USER_DEFINED: &str = "user_defined";

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("the tool discovery source {field} is not an object")]
    NotAnObject { field: &'static str },
    #[error("the tool discovery source {field} cannot be iterated as a list")]
    NotIterable { field: &'static str },
    #[error("the tool discovery field {field} is not text")]
    NotText { field: &'static str },
}

// _enqueue_tool_registry_upsert: MCP metadata, then the OpenAI tools, then the Anthropic passthrough tools, then the response calls.
pub async fn enqueue_tool_registry_upsert(
    queue: &ToolDiscoveryQueue,
    kwargs: Option<&[u8]>,
    hashed_token: Option<&str>,
    team_id: Option<&str>,
    response_names: &[String],
) {
    let Some(kwargs) = kwargs else {
        return;
    };
    let kwargs: DiscoveryKwargs = match serde_json::from_slice(kwargs) {
        Ok(kwargs) => kwargs,
        Err(error) => {
            tracing::debug!("_enqueue_tool_registry_upsert error (non-blocking): {error}");
            return;
        }
    };
    if let Err(error) = discover_tools(queue, &kwargs, hashed_token, team_id, response_names).await
    {
        tracing::debug!("_enqueue_tool_registry_upsert error (non-blocking): {error}");
    }
}

struct DiscoveryFields<'fields> {
    key_hash: Option<&'fields str>,
    team_id: Option<&'fields str>,
    key_alias: Option<&'fields str>,
    user_agent: Option<&'fields str>,
}

impl DiscoveryFields<'_> {
    fn item(&self, tool_name: String, origin: &str) -> ToolDiscoveryItem {
        ToolDiscoveryItem {
            tool_name: Some(tool_name),
            origin: Some(origin.to_owned()),
            created_by: None,
            key_hash: self.key_hash.map(ToOwned::to_owned),
            team_id: self.team_id.map(ToOwned::to_owned),
            key_alias: self.key_alias.map(ToOwned::to_owned),
            user_agent: self.user_agent.map(ToOwned::to_owned),
        }
    }
}

async fn discover_tools(
    queue: &ToolDiscoveryQueue,
    kwargs: &DiscoveryKwargs,
    hashed_token: Option<&str>,
    team_id: Option<&str>,
    response_names: &[String],
) -> Result<(), DiscoveryError> {
    let (key_alias, user_agent) = attribution(kwargs)?;
    let fields = DiscoveryFields {
        key_hash: hashed_token,
        team_id,
        key_alias,
        user_agent,
    };
    enqueue_mcp_tool(queue, kwargs, &fields).await?;
    enqueue_request_tools(queue, kwargs, &fields).await?;
    enqueue_passthrough_tools(queue, kwargs, &fields).await?;
    for name in response_names {
        queue
            .add_update(fields.item(name.clone(), USER_DEFINED))
            .await;
    }
    Ok(())
}

// _litellm_params.get("metadata") or {}: an absent or falsey pair reads as no metadata, and a truthy non-mapping fails the read.
fn litellm_metadata(kwargs: &DiscoveryKwargs) -> Result<Option<&JsonObject>, DiscoveryError> {
    let Some(params) = kwargs
        .litellm_params
        .as_ref()
        .filter(|value| counts_as_present(value))
    else {
        return Ok(None);
    };
    let Json::Object(params) = params else {
        return Err(DiscoveryError::NotAnObject {
            field: "litellm_params",
        });
    };
    let Some(metadata) = params
        .get("metadata")
        .filter(|value| counts_as_present(value))
    else {
        return Ok(None);
    };
    let Json::Object(metadata) = metadata else {
        return Err(DiscoveryError::NotAnObject {
            field: "litellm_params.metadata",
        });
    };
    Ok(Some(metadata))
}

fn attribution(kwargs: &DiscoveryKwargs) -> Result<(Option<&str>, Option<&str>), DiscoveryError> {
    let Some(metadata) = litellm_metadata(kwargs)? else {
        return Ok((None, None));
    };
    Ok((
        present_text(
            metadata.get("user_api_key_alias"),
            "litellm_params.metadata.user_api_key_alias",
        )?,
        present_text(
            metadata.get("user_agent"),
            "litellm_params.metadata.user_agent",
        )?,
    ))
}

// `value or None`: a falsey value names nothing, and a truthy one must be text the queue item accepts.
fn present_text<'text>(
    value: Option<&'text Json>,
    field: &'static str,
) -> Result<Option<&'text str>, DiscoveryError> {
    let Some(present) = value.filter(|candidate| counts_as_present(candidate)) else {
        return Ok(None);
    };
    match present {
        Json::Text(text) => Ok(Some(text)),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(DiscoveryError::NotText { field })
        }
    }
}

// `for tool_def in value or []:`: a mapping or text iterates to no mapping entries, and only a truthy number or bool cannot iterate.
fn tool_list<'list>(
    value: Option<&'list Json>,
    field: &'static str,
) -> Result<&'list [Json], DiscoveryError> {
    let Some(present) = value.filter(|candidate| counts_as_present(candidate)) else {
        return Ok(&[]);
    };
    match present {
        Json::List(items) => Ok(items),
        Json::Object(_) | Json::Text(_) => Ok(&[]),
        Json::Null | Json::Bool(_) | Json::Number(_) => Err(DiscoveryError::NotIterable { field }),
    }
}

async fn enqueue_mcp_tool(
    queue: &ToolDiscoveryQueue,
    kwargs: &DiscoveryKwargs,
    fields: &DiscoveryFields<'_>,
) -> Result<(), DiscoveryError> {
    let Some(mcp) = mcp_call_metadata(kwargs)? else {
        return Ok(());
    };
    let tool_name = match present_text(
        mcp.get("namespaced_tool_name"),
        "mcp_tool_call_metadata.namespaced_tool_name",
    )? {
        Some(name) => Some(name),
        None => present_text(mcp.get("name"), "mcp_tool_call_metadata.name")?,
    };
    let Some(tool_name) = tool_name else {
        return Ok(());
    };
    let origin = present_text(
        mcp.get("mcp_server_name"),
        "mcp_tool_call_metadata.mcp_server_name",
    )?
    .unwrap_or(USER_DEFINED);
    queue
        .add_update(fields.item(tool_name.to_owned(), origin))
        .await;
    Ok(())
}

async fn enqueue_request_tools(
    queue: &ToolDiscoveryQueue,
    kwargs: &DiscoveryKwargs,
    fields: &DiscoveryFields<'_>,
) -> Result<(), DiscoveryError> {
    for tool_def in tool_list(kwargs.tools.as_ref(), "tools")? {
        let Some(name) = function_tool_name(tool_def)? else {
            continue;
        };
        queue.add_update(fields.item(name, USER_DEFINED)).await;
    }
    Ok(())
}

// tools[].function.name: a non-mapping entry or a non-mapping function names no tool, and a truthy non-text name fails the read.
fn function_tool_name(tool_def: &Json) -> Result<Option<String>, DiscoveryError> {
    let Json::Object(tool) = tool_def else {
        return Ok(None);
    };
    let Some(function) = tool
        .get("function")
        .filter(|value| counts_as_present(value))
    else {
        return Ok(None);
    };
    let Json::Object(function) = function else {
        return Ok(None);
    };
    Ok(present_text(function.get("name"), "tools.function.name")?.map(ToOwned::to_owned))
}

fn passthrough_body(kwargs: &DiscoveryKwargs) -> Result<Option<&JsonObject>, DiscoveryError> {
    let Some(passthrough) = kwargs
        .passthrough_logging_payload
        .as_ref()
        .filter(|value| counts_as_present(value))
    else {
        return Ok(None);
    };
    let Json::Object(payload) = passthrough else {
        return Ok(None);
    };
    let Some(body) = payload
        .get("request_body")
        .filter(|value| counts_as_present(value))
    else {
        return Ok(None);
    };
    let Json::Object(body) = body else {
        return Err(DiscoveryError::NotAnObject {
            field: "passthrough_logging_payload.request_body",
        });
    };
    Ok(Some(body))
}

async fn enqueue_passthrough_tools(
    queue: &ToolDiscoveryQueue,
    kwargs: &DiscoveryKwargs,
    fields: &DiscoveryFields<'_>,
) -> Result<(), DiscoveryError> {
    let Some(body) = passthrough_body(kwargs)? else {
        return Ok(());
    };
    for tool_def in tool_list(
        body.get("tools"),
        "passthrough_logging_payload.request_body.tools",
    )? {
        let Json::Object(tool) = tool_def else {
            continue;
        };
        let Some(name) = present_text(tool.get("name"), "request_body.tools.name")? else {
            continue;
        };
        queue
            .add_update(fields.item(name.to_owned(), USER_DEFINED))
            .await;
    }
    Ok(())
}
