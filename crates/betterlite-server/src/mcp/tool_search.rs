mod tests;

use std::cmp::Ordering;
use std::collections::BTreeMap;

use crate::mcp::catalog_guard::types::MCPTool;
use betterlite_db::mcp_stored_contracts::JsonValue;

pub mod identity;
pub mod settings;
pub mod virtual_tools;

use betterlite_semantic::{Embedder, SemanticError, SemanticTextIndex};
use serde::Serialize;
use settings::MCPToolSearchSettings;
use thiserror::Error;
pub const MCP_TOOL_SEARCH_SETTINGS_KEY: &str = "mcp_tool_search";
pub const MCP_TOOL_SEARCH_TOOL_NAME: &str = "mcp_tool_search";
pub const MCP_TOOL_CALL_TOOL_NAME: &str = "mcp_tool_call";
pub const MCP_PROXY_SEARCH_TOOL_NAME: &str = "search_tools";
pub const MCP_PROXY_SCHEMA_TOOL_NAME: &str = "get_tool_schema";
pub const MCP_PROXY_CALL_TOOL_NAME: &str = "call_tool";
pub const AGENT_SEARCH_TOOL_NAME: &str = "agent_search";
pub const SKILL_SEARCH_TOOL_NAME: &str = "skill_search";

#[derive(Clone, Debug, PartialEq)]
pub struct MCPToolSearchHit {
    pub tool: MCPTool,
    pub score: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSearchResult {
    pub name: String,
    pub description: String,
    pub input_schema: BTreeMap<String, JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
}

#[must_use]
pub fn tool_result(tool: &MCPTool, score: Option<f64>) -> ToolSearchResult {
    ToolSearchResult {
        name: tool.name.clone(),
        description: tool.description.clone().unwrap_or_default(),
        input_schema: tool.input_schema.clone(),
        score,
    }
}

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("tool count {tools} differs from score count {scores}")]
    ScoreCount { tools: usize, scores: usize },
    #[error(transparent)]
    Semantic(#[from] SemanticError),
}

pub struct SemanticToolRanker<'state> {
    pub embed: &'state dyn Embedder,
    pub embedding_model: &'state str,
    pub index: &'state mut SemanticTextIndex,
}

#[must_use]
pub fn keyword_score(query: &str, tool: &MCPTool) -> f64 {
    let haystack = tool_text(tool).to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .filter(|token| haystack.contains(token))
        .map(|_| 1.0_f64)
        .sum()
}

#[must_use]
pub fn tool_text(tool: &MCPTool) -> String {
    match tool.description.as_deref() {
        Some(description) if tool.name.is_empty() => description.to_owned(),
        Some(description) if !description.is_empty() => format!("{}\n{}", tool.name, description),
        _ => tool.name.clone(),
    }
}

#[must_use]
pub fn split_core_tools(tools: &[MCPTool], core_names: &[String]) -> (Vec<MCPTool>, Vec<MCPTool>) {
    let mut core = Vec::new();
    for name in core_names {
        if let Some(tool) = tools.iter().rfind(|tool| tool.name == *name)
            && !core
                .iter()
                .any(|candidate: &MCPTool| candidate.name == tool.name)
        {
            core.push(tool.clone());
        }
    }
    let rest = tools
        .iter()
        .filter(|tool| !core_names.contains(&tool.name))
        .cloned()
        .collect();
    (core, rest)
}

pub fn top_hits(
    tools: &[MCPTool],
    scores: &[f64],
    minimum: f64,
    limit: i64,
) -> Result<Vec<(f64, MCPTool)>, SearchError> {
    if tools.len() != scores.len() {
        return Err(SearchError::ScoreCount {
            tools: tools.len(),
            scores: scores.len(),
        });
    }
    let mut hits: Vec<(usize, f64, MCPTool)> = tools
        .iter()
        .zip(scores)
        .enumerate()
        .filter(|(_, (_, score))| **score >= minimum)
        .map(|(order, (tool, score))| (order, *score, tool.clone()))
        .collect();
    hits.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.0.cmp(&right.0))
    });
    let count = if limit < 0 {
        hits.len()
            .saturating_sub(usize::try_from(limit.unsigned_abs()).unwrap_or(usize::MAX))
    } else {
        usize::try_from(limit).unwrap_or(usize::MAX)
    };
    Ok(hits
        .into_iter()
        .take(count)
        .map(|(_, score, tool)| (score, tool))
        .collect())
}

pub async fn rank_mcp_tools(
    query: &str,
    tools: &[MCPTool],
    top_k: i64,
    settings: &MCPToolSearchSettings,
    ranker: Option<&mut SemanticToolRanker<'_>>,
) -> Result<Vec<MCPToolSearchHit>, SearchError> {
    let (core, rest) = split_core_tools(tools, &settings.core_tools);
    let mut hits: Vec<MCPToolSearchHit> = core
        .into_iter()
        .map(|tool| MCPToolSearchHit { tool, score: None })
        .collect();
    if query.is_empty() {
        return Ok(hits);
    }
    let limit = top_k.min(settings.top_k);
    let semantic = ranker.is_some();
    let scores = match ranker {
        Some(state) => state.scores(query, &rest).await?,
        None => rest.iter().map(|tool| keyword_score(query, tool)).collect(),
    };
    let minimum = if semantic {
        settings.similarity_threshold
    } else {
        1.0_f64
    };
    hits.extend(
        top_hits(&rest, &scores, minimum, limit)?
            .into_iter()
            .map(|(score, tool)| MCPToolSearchHit {
                tool,
                score: semantic.then_some(score),
            }),
    );
    Ok(hits)
}

pub async fn search_mcp_tools(
    query: &str,
    tools: &[MCPTool],
    top_k: i64,
    settings: &MCPToolSearchSettings,
    ranker: Option<&mut SemanticToolRanker<'_>>,
) -> Result<Vec<ToolSearchResult>, SearchError> {
    let hits = rank_mcp_tools(query, tools, top_k, settings, ranker).await?;
    Ok(hits
        .into_iter()
        .map(|hit| ToolSearchResult {
            name: hit.tool.name,
            description: hit.tool.description.unwrap_or_default(),
            input_schema: hit.tool.input_schema,
            score: hit.score,
        })
        .collect())
}

impl SemanticToolRanker<'_> {
    async fn scores(&mut self, query: &str, tools: &[MCPTool]) -> Result<Vec<f64>, SemanticError> {
        let texts: Vec<String> = tools.iter().map(tool_text).collect();
        self.index
            .scores(query, &texts, self.embed, self.embedding_model)
            .await
    }
}
