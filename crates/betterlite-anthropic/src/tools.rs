use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::definitions::error::DefinitionsError;

use crate::feature_flags::FeatureFlagsError;

mod choice;
mod fields;
mod helper;
mod mcp;
mod namespace;
mod options;
mod tests;
pub mod wire;

use choice::{named_mode, object_choice};
use fields::field;
use helper::{advisor_tool, computer_tool, function_tool, hosted_name, push, sent_fields};
use mcp::{authorization_token, tool_configuration};
use namespace::expand_namespace;
use options::collect_options;
use wire::{
    ADVISOR_TYPE, CUSTOM_TYPE, ChoiceKind, HostedTool, MappedServer, McpServerTool, ToolChoice,
    ToolDefinition, URL_TYPE,
};

// every tool type whose name starts with one of these maps as a hosted Anthropic tool
const HOSTED_PREFIXES: [&str; 7] = [
    "web_search",
    "bash",
    "text_editor",
    "code_execution",
    "web_fetch",
    "memory",
    "tool_search_tool",
];

#[derive(Debug, thiserror::Error)]
pub enum ToolsError {
    #[error("tool must be a JSON object")]
    NotAnObject,
    #[error("Missing required parameter: {0}")]
    Missing(&'static str),
    #[error("{field} must be {expected}")]
    WrongType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("Unsupported tool type: {0}")]
    UnsupportedType(String),
    #[error("Advisor tool must have a valid model")]
    AdvisorModel,
    #[error("defer_loading must be a boolean")]
    DeferLoading,
    #[error("allowed_callers must be a list of strings")]
    AllowedCallers,
    #[error(transparent)]
    Eager(#[from] FeatureFlagsError),
    #[error(transparent)]
    Definitions(#[from] DefinitionsError),
}

impl ToolsError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Eager(error) => error.status_code(),
            Self::NotAnObject
            | Self::Missing(_)
            | Self::WrongType { .. }
            | Self::UnsupportedType(_)
            | Self::AdvisorModel
            | Self::DeferLoading
            | Self::AllowedCallers
            | Self::Definitions(_) => None,
        }
    }
}

#[derive(Clone, Default)]
pub struct ToolMapping {
    pub tools: Vec<ToolDefinition>,
    pub mcp_servers: Vec<McpServerTool>,
}

pub fn map_tool_choice(
    tool_choice: Option<&Json>,
    parallel_tool_use: Option<bool>,
) -> Result<Option<ToolChoice>, ToolsError> {
    let mut choice = match tool_choice {
        Some(Json::Text(text)) => named_mode(text.as_str()),
        Some(Json::Object(fields)) => object_choice(fields)?,
        None | Some(Json::Null) => None,
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_)) => {
            return Err(ToolsError::WrongType {
                field: "tool_choice",
                expected: "a string or an object",
            });
        }
    };
    let Some(parallel) = parallel_tool_use else {
        return Ok(choice);
    };
    if matches!(tool_choice, Some(Json::Text(text)) if text == "none") {
        return Ok(choice);
    }
    let disable = Some(!parallel);
    match &mut choice {
        Some(existing) => existing.disable_parallel_tool_use = disable,
        None => {
            choice = Some(ToolChoice {
                kind: ChoiceKind::Auto,
                name: None,
                disable_parallel_tool_use: disable,
            });
        }
    }
    Ok(choice)
}

pub fn map_tool_helper(
    tool: &Json,
) -> Result<(Option<ToolDefinition>, Option<McpServerTool>), ToolsError> {
    let fields = tool.as_object().ok_or(ToolsError::NotAnObject)?;
    let kind =
        field(fields, "type", "a string", Json::as_str)?.ok_or(ToolsError::Missing("type"))?;
    if matches!(kind, "function" | CUSTOM_TYPE) {
        return Ok((
            Some(ToolDefinition::Function(function_tool(tool, fields)?)),
            None,
        ));
    }
    if kind.starts_with("computer_") {
        return Ok((
            Some(ToolDefinition::Computer(computer_tool(kind, fields)?)),
            None,
        ));
    }
    if HOSTED_PREFIXES
        .iter()
        .any(|prefix| kind.starts_with(prefix))
    {
        let mut sent = sent_fields(fields);
        collect_options(fields, kind)?.apply(&mut sent);
        let hosted = HostedTool {
            kind: kind.to_owned(),
            name: hosted_name(fields)?,
            fields: sent,
        };
        return Ok((Some(ToolDefinition::Hosted(hosted)), None));
    }
    if kind == URL_TYPE {
        return Ok((None, Some(McpServerTool::Descriptor(fields.clone()))));
    }
    if kind == "mcp" {
        return Ok((None, Some(map_openai_mcp_server_tool(tool)?)));
    }
    if kind == ADVISOR_TYPE {
        return Ok((Some(ToolDefinition::Advisor(advisor_tool(fields)?)), None));
    }
    Err(ToolsError::UnsupportedType(kind.to_owned()))
}

pub fn map_openai_mcp_server_tool(tool: &Json) -> Result<McpServerTool, ToolsError> {
    let fields = tool.as_object().ok_or(ToolsError::NotAnObject)?;
    let url = field(fields, "server_url", "a string", Json::as_str)?
        .ok_or(ToolsError::Missing("server_url"))?;
    let name = field(fields, "server_label", "a string", Json::as_str)?
        .ok_or(ToolsError::Missing("server_label"))?;
    Ok(McpServerTool::Mapped(MappedServer {
        kind: URL_TYPE,
        url: url.to_owned(),
        name: name.to_owned(),
        tool_configuration: tool_configuration(fields)?,
        authorization_token: authorization_token(fields)?,
    }))
}

pub fn map_tools(tools: &[Json]) -> Result<ToolMapping, ToolsError> {
    let mut mapping = ToolMapping::default();
    for tool in tools {
        let fields = tool.as_object().ok_or(ToolsError::NotAnObject)?;
        if fields.get("type").and_then(Json::as_str) == Some("namespace") {
            expand_namespace(fields, &mut mapping)?;
        } else if fields.contains_key("input_schema") {
            mapping.tools.push(ToolDefinition::Native(fields.clone()));
        } else {
            push(&mut mapping, map_tool_helper(tool)?);
        }
    }
    Ok(mapping)
}
