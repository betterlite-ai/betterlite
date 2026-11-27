use std::collections::BTreeMap;
use std::ops::DerefMut as _;

use betterlite_core::schema::SchemaLimits;
use betterlite_core::schema::validate::validate_schema;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_mcp::client::tools::result::CallToolResult;

use super::error::SearchHandlerError;
use super::types::{HandlerContext, ProxyToolIdentity, ToolCallRequest, json_string};
use super::{_text_tool_result, EMBEDDING_WITHOUT_ROUTER, handle_mcp_tool_call};
use crate::mcp::catalog_guard::types::MCPTool;
use crate::mcp::tool_search::identity::{MCPProxyToolIdentityError, proxy_search_result};
use crate::mcp::tool_search::settings::MCPToolSearchSettings;
use crate::mcp::tool_search::{
    SearchError, SemanticToolRanker, ToolSearchResult, rank_mcp_tools, search_mcp_tools,
};

pub(super) async fn ranked_tool_search(
    query: &str,
    top_k: i64,
    tools: &[MCPTool],
    settings: &MCPToolSearchSettings,
    context: &HandlerContext<'_>,
) -> Result<Vec<ToolSearchResult>, SearchError> {
    let router = settings
        .embedding_model
        .as_deref()
        .zip(context.settings.llm_router());
    let mut guard = context.settings.mcp_search_index().lock().await;
    match router {
        Some((embedding_model, router)) => {
            let embedder = router.embedder(embedding_model);
            let mut ranker = SemanticToolRanker {
                embed: embedder.as_ref(),
                embedding_model,
                index: guard.deref_mut(),
            };
            search_mcp_tools(query, tools, top_k, settings, Some(&mut ranker)).await
        }
        None => search_mcp_tools(query, tools, top_k, settings, None).await,
    }
}

pub(super) async fn proxy_search(
    arguments: &BTreeMap<String, JsonValue>,
    tools: &[MCPTool],
    context: &HandlerContext<'_>,
) -> Result<CallToolResult, SearchHandlerError> {
    let settings = match context.settings.mcp_tool_search_settings() {
        Ok(settings) => settings,
        Err(error) => return Ok(_text_tool_result(error.to_string(), true)),
    };
    if settings.embedding_model.is_some() && context.settings.llm_router().is_none() {
        return Ok(_text_tool_result(EMBEDDING_WITHOUT_ROUTER.to_owned(), true));
    }
    let query = arguments
        .get("query")
        .and_then(json_string)
        .unwrap_or_default();
    let router = settings
        .embedding_model
        .as_deref()
        .zip(context.settings.llm_router());
    let mut guard = context.settings.mcp_search_index().lock().await;
    let ranked = match router {
        Some((embedding_model, router)) => {
            let embedder = router.embedder(embedding_model);
            let mut ranker = SemanticToolRanker {
                embed: embedder.as_ref(),
                embedding_model,
                index: guard.deref_mut(),
            };
            rank_mcp_tools(query, tools, 5, &settings, Some(&mut ranker)).await
        }
        None => rank_mcp_tools(query, tools, 5, &settings, None).await,
    };
    Ok(match ranked {
        Ok(hits) => {
            let results: Result<Vec<_>, MCPProxyToolIdentityError> =
                hits.iter().map(proxy_search_result).collect();
            _text_tool_result(serde_json::to_string(&results?)?, false)
        }
        Err(error) => _text_tool_result(error.to_string(), true),
    })
}

pub(super) async fn proxy_call(
    tool: &MCPTool,
    arguments: &BTreeMap<String, JsonValue>,
    context: &HandlerContext<'_>,
) -> Result<CallToolResult, SearchHandlerError> {
    let tool_arguments = match arguments.get("arguments") {
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
