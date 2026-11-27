use std::collections::BTreeMap;

use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_mcp::client::tools::result::CallToolResult;
use serde::Serialize;

use super::error::SearchHandlerError;
use super::types::{AgentSearchRequest, HandlerContext, SearchOutcome, json_string};
use super::{EMBEDDING_WITHOUT_ROUTER, text_tool_result};
use crate::mcp::catalog_guard::types::MCPTool;
use crate::mcp::tool_search::identity::{MCPProxyToolIdentityError, proxy_search_result};
use crate::mcp::tool_search::settings::MCPToolSearchSettings;
use crate::mcp::tool_search::{
    MCPToolSearchHit, SearchError, SemanticToolRanker, ToolSearchResult, rank_mcp_tools,
};

pub(super) async fn ranked_hits(
    query: &str,
    top_k: i64,
    tools: &[MCPTool],
    settings: &MCPToolSearchSettings,
    context: &HandlerContext<'_>,
) -> Result<Vec<MCPToolSearchHit>, SearchError> {
    let Some((embedding_model, router)) = settings
        .embedding_model
        .as_deref()
        .zip(context.settings.llm_router())
    else {
        return rank_mcp_tools(query, tools, top_k, settings, None).await;
    };
    let embedder = router.embedder(embedding_model);
    let mut guard = context.settings.mcp_search_index().lock().await;
    rank_mcp_tools(
        query,
        tools,
        top_k,
        settings,
        Some(&mut SemanticToolRanker {
            embed: embedder.as_ref(),
            embedding_model,
            index: &mut guard,
        }),
    )
    .await
}

pub(super) fn tool_search_result(hit: MCPToolSearchHit) -> ToolSearchResult {
    ToolSearchResult {
        name: hit.tool.name,
        description: hit.tool.description.unwrap_or_default(),
        input_schema: hit.tool.input_schema,
        score: hit.score,
    }
}

// LiteLLM's agent search handler resolves the caller's accessible agents and searches them
pub(super) async fn agent_search_text(
    query: &str,
    top_k: i64,
    context: &HandlerContext<'_>,
) -> Result<CallToolResult, SearchHandlerError> {
    let agents = context.ports.accessible_agents(context.caller).await?;
    let outcome = context
        .ports
        .search_agents(
            AgentSearchRequest { query, top_k },
            &agents,
            context.settings.agent_embedding_model(),
            context.caller,
        )
        .await?;
    search_text(outcome)
}

pub(super) fn tool_search_text(
    ranking: Result<Vec<MCPToolSearchHit>, SearchError>,
) -> Result<CallToolResult, SearchHandlerError> {
    match ranking {
        Ok(hits) => {
            let results: Vec<ToolSearchResult> = hits.into_iter().map(tool_search_result).collect();
            Ok(text_tool_result(serde_json::to_string(&results)?, false))
        }
        Err(error) => Ok(text_tool_result(error.to_string(), true)),
    }
}

fn search_text<T>(outcome: SearchOutcome<T>) -> Result<CallToolResult, SearchHandlerError>
where
    T: Serialize,
{
    match outcome {
        SearchOutcome::Hits(hits) => Ok(text_tool_result(serde_json::to_string(&hits)?, false)),
        SearchOutcome::NotConfigured { reason } | SearchOutcome::EmbeddingFailed { reason } => {
            Ok(text_tool_result(reason, true))
        }
    }
}

pub(super) async fn proxy_search(
    arguments: &BTreeMap<String, JsonValue>,
    tools: &[MCPTool],
    context: &HandlerContext<'_>,
) -> Result<CallToolResult, SearchHandlerError> {
    let settings = match context.settings.mcp_tool_search_settings() {
        Ok(settings) => settings,
        Err(error) => return Ok(text_tool_result(error.to_string(), true)),
    };
    if settings.embedding_model.is_some() && context.settings.llm_router().is_none() {
        return Ok(text_tool_result(EMBEDDING_WITHOUT_ROUTER.to_owned(), true));
    }
    let query = arguments
        .get("query")
        .and_then(json_string)
        .unwrap_or_default();
    match ranked_hits(query, 5, tools, &settings, context).await {
        Ok(hits) => {
            let results: Result<Vec<_>, MCPProxyToolIdentityError> =
                hits.iter().map(proxy_search_result).collect();
            Ok(text_tool_result(serde_json::to_string(&results?)?, false))
        }
        Err(error) => Ok(text_tool_result(error.to_string(), true)),
    }
}
