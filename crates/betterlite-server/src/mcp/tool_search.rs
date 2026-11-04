mod tests;

use std::collections::BTreeMap;

use crate::mcp::catalog_guard::types::MCPTool;
use betterlite_db::mcp_stored_contracts::JsonValue;

mod identity;
mod virtual_tools;

use betterlite_semantic::{Embedder, EmbeddingFailed, SemanticTextIndex};
pub use identity::{
    MCPProxySchemaResult, MCPProxySearchResult, MCPProxyToolIdentityError, mcp_proxy_tool_id,
    proxy_schema_result, proxy_search_result, with_mcp_proxy_identity,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
pub use virtual_tools::{
    TextContent, TextToolResult, VirtualToolDefinition, get_mcp_proxy_tool_definitions,
    get_virtual_tool_definitions, text_tool_result,
};

pub const MCP_TOOL_SEARCH_SETTINGS_KEY: &str = "mcp_tool_search";
pub const MCP_TOOL_SEARCH_TOOL_NAME: &str = "mcp_tool_search";
pub const MCP_TOOL_CALL_TOOL_NAME: &str = "mcp_tool_call";
pub const MCP_PROXY_SEARCH_TOOL_NAME: &str = "search_tools";
pub const MCP_PROXY_SCHEMA_TOOL_NAME: &str = "get_tool_schema";
pub const MCP_PROXY_CALL_TOOL_NAME: &str = "call_tool";
pub const AGENT_SEARCH_TOOL_NAME: &str = "agent_search";
pub const SKILL_SEARCH_TOOL_NAME: &str = "skill_search";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MCPToolSearchSettings {
    pub embedding_model: Option<String>,
    pub top_k: usize,
    pub similarity_threshold: f64,
    pub core_tools: Vec<String>,
}

impl Default for MCPToolSearchSettings {
    fn default() -> Self {
        Self {
            embedding_model: None,
            top_k: 5,
            similarity_threshold: 0.0,
            core_tools: Vec::new(),
        }
    }
}

impl MCPToolSearchSettings {
    pub fn from_value(value: &Value) -> Result<Self, MCPToolSearchSettingsError> {
        let settings: Self = serde_json::from_value(value.clone())?;
        if !(1..=100).contains(&settings.top_k) {
            return Err(MCPToolSearchSettingsError::TopKOutOfRange);
        }
        if !settings.similarity_threshold.is_finite()
            || !(0.0..=1.0).contains(&settings.similarity_threshold)
        {
            return Err(MCPToolSearchSettingsError::SimilarityThresholdOutOfRange);
        }
        Ok(settings)
    }
}

#[derive(Debug, Error)]
pub enum MCPToolSearchSettingsError {
    #[error("invalid mcp_tool_search settings: {0}")]
    Invalid(#[from] serde_json::Error),
    #[error("mcp_tool_search.top_k must be between 1 and 100")]
    TopKOutOfRange,
    #[error("mcp_tool_search.similarity_threshold must be between 0 and 1")]
    SimilarityThresholdOutOfRange,
}

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

pub fn tool_result(tool: &MCPTool, score: Option<f64>) -> ToolSearchResult {
    ToolSearchResult {
        name: tool.name.clone(),
        description: tool.description.clone().unwrap_or_default(),
        input_schema: tool.input_schema.clone(),
        score,
    }
}

pub struct SemanticToolRanker<'a> {
    pub embed: &'a dyn Embedder,
    pub embedding_model: &'a str,
    pub index: &'a mut SemanticTextIndex,
}

pub fn coerce_top_k(value: &Value, default: usize) -> usize {
    match value {
        Value::Number(number) => number
            .as_i64()
            .and_then(|number| usize::try_from(number).ok())
            .unwrap_or(default),
        Value::String(text) => text
            .parse::<i64>()
            .ok()
            .and_then(|number| usize::try_from(number).ok())
            .unwrap_or(default),
        Value::Bool(flag) => usize::from(*flag),
        Value::Null | Value::Array(_) | Value::Object(_) => default,
    }
}

pub fn keyword_score(query: &str, tool: &MCPTool) -> f64 {
    let haystack = tool_text(tool).to_lowercase();
    let score = query
        .to_lowercase()
        .split_whitespace()
        .filter(|token| haystack.contains(token))
        .count();
    f64::from(u32::try_from(score).unwrap_or(u32::MAX))
}

pub fn tool_text(tool: &MCPTool) -> String {
    match tool.description.as_deref() {
        Some(description) if !description.is_empty() => format!("{}\n{}", tool.name, description),
        _ => tool.name.clone(),
    }
}

pub fn split_core_tools(tools: &[MCPTool], core_names: &[String]) -> (Vec<MCPTool>, Vec<MCPTool>) {
    let mut core = Vec::new();
    for name in core_names {
        if let Some(tool) = tools.iter().rfind(|tool| tool.name == *name) {
            if !core
                .iter()
                .any(|candidate: &MCPTool| candidate.name == tool.name)
            {
                core.push(tool.clone());
            }
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
    limit: usize,
) -> Vec<(f64, MCPTool)> {
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
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    hits.into_iter()
        .take(limit)
        .map(|(_, score, tool)| (score, tool))
        .collect()
}

pub async fn rank_mcp_tools(
    query: &str,
    tools: &[MCPTool],
    top_k: usize,
    settings: &MCPToolSearchSettings,
    ranker: Option<&mut SemanticToolRanker<'_>>,
) -> Result<Vec<MCPToolSearchHit>, EmbeddingFailed> {
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
        Some(ranker) => {
            let texts: Vec<String> = rest.iter().map(tool_text).collect();
            ranker
                .index
                .scores(query, &texts, ranker.embed, ranker.embedding_model)
                .await
                .map_err(|error| match error {
                    betterlite_semantic::SemanticError::Failed(failure) => failure,
                    betterlite_semantic::SemanticError::Http {
                        status_code,
                        detail,
                    } => EmbeddingFailed {
                        reason: format!("HTTP {status_code}: {detail}"),
                    },
                })?
        }
        None => rest.iter().map(|tool| keyword_score(query, tool)).collect(),
    };
    let minimum = if semantic {
        settings.similarity_threshold
    } else {
        1.0
    };
    hits.extend(
        top_hits(&rest, &scores, minimum, limit)
            .into_iter()
            .map(|(score, tool)| MCPToolSearchHit {
                tool,
                score: if semantic { Some(score) } else { None },
            }),
    );
    Ok(hits)
}

pub fn search_tools(query: &str, tools: &[MCPTool], top_k: usize) -> Vec<ToolSearchResult> {
    if query.is_empty() {
        return Vec::new();
    }
    let hits = top_hits(
        tools,
        &tools
            .iter()
            .map(|tool| keyword_score(query, tool))
            .collect::<Vec<_>>(),
        1.0,
        top_k,
    )
    .into_iter()
    .map(|(score, tool)| MCPToolSearchHit {
        tool,
        score: Some(score),
    })
    .collect();
    project_hits(hits)
}

pub async fn search_mcp_tools(
    query: &str,
    tools: &[MCPTool],
    top_k: usize,
    settings: &MCPToolSearchSettings,
    ranker: Option<&mut SemanticToolRanker<'_>>,
) -> Result<Vec<ToolSearchResult>, EmbeddingFailed> {
    Ok(project_hits(
        rank_mcp_tools(query, tools, top_k, settings, ranker).await?,
    ))
}

fn project_hits(hits: Vec<MCPToolSearchHit>) -> Vec<ToolSearchResult> {
    hits.iter()
        .map(|hit| tool_result(&hit.tool, hit.score))
        .collect()
}
