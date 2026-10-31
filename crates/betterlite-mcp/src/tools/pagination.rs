use std::collections::BTreeSet;
use std::error::Error;
use std::future::Future;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::time::{Instant, error::Elapsed, timeout};
use tracing::warn;

use super::schemas::{McpTool, OpenAiTool, mcp_tool_to_openai};
use super::tool_calls::CallRequest;

pub const MAX_PAGES: usize = 1_000;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct Page {
    pub tools: Vec<McpTool>,
    #[serde(rename = "nextCursor")]
    pub next_cursor: Option<String>,
}

pub trait McpSession {
    type Error: Error + Send + Sync + 'static;
    type Output;
    fn list_tools(
        &mut self,
        cursor: Option<&str>,
    ) -> impl Future<Output = Result<Page, Self::Error>> + Send;
    fn call_tool(
        &mut self,
        request: CallRequest,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send;
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError<Source> {
    #[error("MCP session failed: {0}")]
    Session(#[source] Source),
    #[error("MCP tool call timed out")]
    Timeout(#[source] Elapsed),
}

#[derive(Clone, Copy, Debug)]
pub struct Timeouts {
    pub client: Duration,
    pub listing: Duration,
    pub listing_deadline: Option<Duration>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ToolFormat {
    #[default]
    Mcp,
    Openai,
}

pub async fn list_tools_with_pagination<Session>(
    session: &mut Session,
    timeouts: Timeouts,
) -> Result<Vec<McpTool>, ToolError<Session::Error>>
where
    Session: McpSession,
{
    let deadline = timeouts
        .listing_deadline
        .unwrap_or_else(|| timeouts.client.max(timeouts.listing));
    let mut tools = Vec::new();
    let mut cursor = None;
    let mut seen = BTreeSet::new();
    let started = Instant::now();
    for _page in 0..MAX_PAGES {
        let remaining = deadline.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            warn!("MCP tools/list listing deadline expired");
            return Ok(tools);
        }
        let Ok(response) = timeout(remaining, session.list_tools(cursor.as_deref())).await else {
            warn!("MCP tools/list listing deadline expired");
            return Ok(tools);
        };
        let page = response.map_err(ToolError::Session)?;
        tools.extend(page.tools);
        let Some(next_cursor) = page.next_cursor.filter(|value| !value.is_empty()) else {
            return Ok(tools);
        };
        if !seen.insert(next_cursor.clone()) {
            warn!("MCP server repeated a tools/list cursor");
            return Ok(tools);
        }
        cursor = Some(next_cursor);
    }
    warn!("MCP tools/list pagination reached the 1,000 page limit");
    Ok(tools)
}

pub async fn load_mcp_tools<Session>(
    session: &mut Session,
    format: ToolFormat,
    timeouts: Timeouts,
) -> Result<Vec<LoadedTool>, ToolError<Session::Error>>
where
    Session: McpSession,
{
    let tools = list_tools_with_pagination(session, timeouts).await?;
    Ok(match format {
        ToolFormat::Mcp => tools.into_iter().map(LoadedTool::Mcp).collect(),
        ToolFormat::Openai => tools
            .iter()
            .map(|tool| LoadedTool::Openai(mcp_tool_to_openai(tool)))
            .collect(),
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum LoadedTool {
    Mcp(McpTool),
    Openai(OpenAiTool),
}
