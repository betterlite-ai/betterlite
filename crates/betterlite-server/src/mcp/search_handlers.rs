pub mod error;
pub mod ports;
pub mod types;

mod dispatch;
mod tests;

use std::collections::BTreeMap;

use betterlite_core::schema::SchemaLimits;
use betterlite_core::schema::validate::validate_schema;
use betterlite_core::skills::search::MAX_SKILL_SEARCH_TOP_K;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_mcp::client::tools::result::{CallToolResult, ContentBlock, TextContent};
use betterlite_mcp::wire_compat::result_conversion::complete_call_tool_result;
use betterlite_mcp::wire_compat::tool_outcome::WireCompat;
use betterlite_prompts::arguments::json::Object;
use http::StatusCode;

use self::dispatch::{agent_search_text, proxy_search, ranked_tool_search};
use self::error::SearchHandlerError;
use self::ports::ToolExecution;
use self::types::{
    AgentSearchRequest, HandlerContext, McpToolSearchRequest, ProxyToolIdentity, ProxyToolRequest,
    SearchOutcome, SkillSearchRequest, ToolCallRequest, json_string,
};
use crate::management_rbac::FeatureName;
use crate::mcp::catalog_guard::types::MCPTool;
use crate::mcp::tool_search::identity::{mcp_proxy_tool_id, proxy_schema_result};
use crate::mcp::tool_search::{
    MCP_PROXY_CALL_TOOL_NAME, MCP_PROXY_SCHEMA_TOOL_NAME, MCP_PROXY_SEARCH_TOOL_NAME,
    MCP_TOOL_SEARCH_SETTINGS_KEY,
};

// the message litellm_settings.mcp_tool_search.embedding_model gets when no model_list exists
pub(super) const EMBEDDING_WITHOUT_ROUTER: &str =
    "litellm_settings.mcp_tool_search.embedding_model needs a model_list so it can be called";
const UNKNOWN_TOOL_ID: &str = "Unknown or unauthorized tool_id";

pub async fn handle_agent_search(
    request: AgentSearchRequest<'_>,
    context: &HandlerContext<'_>,
) -> Result<CallToolResult, SearchHandlerError> {
    context
        .ports
        .check_feature_access_for_user(context.caller, FeatureName::Agents)
        .await?;
    agent_search_text(request.query, request.top_k.max(1), context).await
}

pub async fn handle_skill_search(
    request: SkillSearchRequest<'_>,
    context: &HandlerContext<'_>,
) -> Result<CallToolResult, SearchHandlerError> {
    let skills = context.ports.accessible_skills(context.caller).await?;
    match context
        .ports
        .search_skills(
            request.query,
            &skills,
            request
                .top_k
                .clamp(1, i64::try_from(MAX_SKILL_SEARCH_TOP_K).unwrap_or(i64::MAX)),
            context.settings.skill_embedding_model(),
            context.caller,
        )
        .await?
    {
        SearchOutcome::Hits(hits) => Ok(_text_tool_result(serde_json::to_string(&hits)?, false)),
        SearchOutcome::NotConfigured { reason } | SearchOutcome::EmbeddingFailed { reason } => {
            Ok(_text_tool_result(reason, true))
        }
    }
}

pub async fn handle_mcp_tool_search(
    request: McpToolSearchRequest<'_>,
    context: &HandlerContext<'_>,
) -> Result<CallToolResult, SearchHandlerError> {
    let settings = match context.settings.mcp_tool_search_settings() {
        Ok(settings) => settings,
        Err(error) => {
            return Ok(_text_tool_result(
                format!("litellm_settings.{MCP_TOOL_SEARCH_SETTINGS_KEY} is invalid: {error}"),
                true,
            ));
        }
    };
    if settings.embedding_model.is_some() && context.settings.llm_router().is_none() {
        return Ok(_text_tool_result(EMBEDDING_WITHOUT_ROUTER.to_owned(), true));
    }
    let tools = context
        .operations
        ._list_mcp_tools(context.caller, context.scope, false)
        .await?;
    let ranking =
        ranked_tool_search(request.query, request.top_k, &tools, &settings, context).await;
    match ranking {
        Ok(results) => Ok(_text_tool_result(serde_json::to_string(&results)?, false)),
        Err(error) => Ok(_text_tool_result(error.to_string(), true)),
    }
}

pub async fn handle_mcp_proxy_tool(
    request: ProxyToolRequest<'_>,
    context: &HandlerContext<'_>,
) -> Result<CallToolResult, SearchHandlerError> {
    let tools = context
        .operations
        ._list_mcp_tools(context.caller, context.scope, true)
        .await?;
    if request.name == MCP_PROXY_SEARCH_TOOL_NAME {
        return proxy_search(request.arguments, &tools, context).await;
    }
    let tools_by_id = proxy_tools_by_id(&tools)?;
    let tool = request
        .arguments
        .get("tool_id")
        .and_then(json_string)
        .and_then(|tool_id| tools_by_id.get(tool_id).copied());
    let Some(tool) = tool else {
        return Ok(_text_tool_result(UNKNOWN_TOOL_ID.to_owned(), true));
    };
    if request.name == MCP_PROXY_SCHEMA_TOOL_NAME {
        return Ok(_text_tool_result(
            serde_json::to_string(&proxy_schema_result(tool)?)?,
            false,
        ));
    }
    if request.name != MCP_PROXY_CALL_TOOL_NAME {
        return Err(SearchHandlerError::Http {
            status: StatusCode::BAD_REQUEST,
            detail: format!("Unknown MCP proxy tool: {}", request.name),
        });
    }
    let tool_arguments = match request.arguments.get("arguments") {
        Some(JsonValue::Object(values)) => values.clone(),
        Some(_) => {
            return Ok(_text_tool_result(
                "arguments must be an object".to_owned(),
                true,
            ));
        }
        None => BTreeMap::new(),
    };
    let schema = serde_json::to_value(&tool.input_schema)?;
    let instance = serde_json::to_string(&tool_arguments)?;
    if let Err(error) = validate_schema(&schema, &instance, &SchemaLimits::default()) {
        return Ok(_text_tool_result(
            format!("Invalid arguments: {error}"),
            true,
        ));
    }
    let identity = ProxyToolIdentity::from_tool(tool)?;
    handle_mcp_tool_call(
        ToolCallRequest {
            tool_name: &identity.tool_name,
            arguments: &tool_arguments,
            requested_server_id: Some(&identity.server_id),
            guardrail_context: None,
        },
        context,
    )
    .await
}

pub async fn handle_mcp_tool_call(
    request: ToolCallRequest<'_>,
    context: &HandlerContext<'_>,
) -> Result<CallToolResult, SearchHandlerError> {
    let ToolCallRequest {
        tool_name,
        arguments,
        requested_server_id,
        guardrail_context,
    } = request;
    let allowed_servers = context
        .operations
        ._get_allowed_mcp_servers(context.caller, context.scope)
        .await?;
    let requested = context
        .scope
        .mcp_servers
        .as_deref()
        .filter(|servers| !servers.is_empty());
    if let Some(servers) = requested
        && allowed_servers.is_empty()
    {
        let denial = context
            .operations
            .raise_denied_scoped_mcp_access(context.caller, context.scope, servers)
            .await;
        return Err(SearchHandlerError::Http {
            status: StatusCode::FORBIDDEN,
            detail: denial.detail,
        });
    }
    if allowed_servers.is_empty() {
        return Err(SearchHandlerError::Http {
            status: StatusCode::FORBIDDEN,
            detail: "User not allowed to call this tool.".to_owned(),
        });
    }
    let call = ToolExecution {
        scope: context.scope,
        caller: context.caller,
        tool_name,
        arguments,
        allowed_servers: &allowed_servers,
        requested_server_id,
        guardrail_context,
    };
    Ok(complete_call_tool_result(
        context.operations.execute_mcp_tool(call).await?,
        WireCompat::Legacy,
    ))
}

fn proxy_tools_by_id(tools: &[MCPTool]) -> Result<BTreeMap<String, &MCPTool>, SearchHandlerError> {
    let mut tools_by_id = BTreeMap::new();
    for tool in tools {
        tools_by_id.insert(mcp_proxy_tool_id(tool)?, tool);
    }
    Ok(tools_by_id)
}

pub(super) fn _text_tool_result(text: String, is_error: bool) -> CallToolResult {
    CallToolResult {
        content: vec![ContentBlock::Text(TextContent {
            text,
            extra: Object::new(),
        })],
        is_error,
        structured_content: None,
        meta: None,
    }
}
