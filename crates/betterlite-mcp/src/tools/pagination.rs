use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use super::schemas::{McpTool, OpenAiTool, mcp_tool_to_openai};
use super::tool_calls::{CallToolResult, McpCallToolRequest};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub tools: Vec<McpTool>,
    pub next_cursor: Option<String>,
}

pub trait McpSession {
    async fn list_tools(&mut self, cursor: Option<String>) -> Result<Page, String>;
    async fn call_tool(&mut self, request: McpCallToolRequest) -> Result<CallToolResult, String>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolFormat {
    Mcp,
    Openai,
}

pub async fn list_tools_with_pagination<S: McpSession>(
    session: &mut S,
    client_timeout: Duration,
    listing_timeout: Duration,
) -> Vec<McpTool> {
    let deadline = client_timeout.max(listing_timeout);
    collect_pages(session, deadline).await
}

async fn collect_pages<S: McpSession>(session: &mut S, deadline: Duration) -> Vec<McpTool> {
    let mut tools = Vec::new();
    let mut cursor = None;
    let mut seen = BTreeSet::new();
    let started = Instant::now();
    for _page in 0..1_000 {
        let remaining = match deadline.checked_sub(started.elapsed()) {
            Some(remaining) => remaining,
            None => {
                tracing::warn!("MCP tools/list listing deadline expired");
                return tools;
            }
        };
        let page = match tokio::time::timeout(remaining, session.list_tools(cursor.clone())).await {
            Ok(page) => page,
            Err(_) => {
                tracing::warn!("MCP tools/list listing deadline expired");
                return tools;
            }
        };
        let page = match page {
            Ok(page) => page,
            Err(error) => {
                tracing::warn!(%error, "MCP tools/list page failed");
                return tools;
            }
        };
        tools.extend(page.tools);
        let Some(next_cursor) = page.next_cursor.filter(|cursor| !cursor.is_empty()) else {
            return tools;
        };
        if !seen.insert(next_cursor.clone()) {
            tracing::warn!("MCP server repeated a tools/list cursor");
            return tools;
        }
        cursor = Some(next_cursor);
    }
    tracing::warn!("MCP tools/list pagination reached the 1,000 page limit");
    tools
}

pub async fn load_mcp_tools<S: McpSession>(
    session: &mut S,
    format: ToolFormat,
    client_timeout: Duration,
    listing_timeout: Duration,
) -> Result<Vec<McpToolOrOpenAi>, String> {
    let tools = list_tools_with_pagination(session, client_timeout, listing_timeout).await;
    Ok(match format {
        ToolFormat::Mcp => tools.into_iter().map(McpToolOrOpenAi::Mcp).collect(),
        ToolFormat::Openai => tools
            .iter()
            .map(|tool| McpToolOrOpenAi::Openai(mcp_tool_to_openai(tool)))
            .collect(),
    })
}

#[derive(Clone, Debug, PartialEq)]
pub enum McpToolOrOpenAi {
    Mcp(McpTool),
    Openai(OpenAiTool),
}

#[cfg(test)]
mod pagination_behavior_tests {
    use super::*;
    use std::time::Duration;

    struct Fake {
        pages: Vec<Page>,
    }
    impl McpSession for Fake {
        async fn list_tools(&mut self, cursor: Option<String>) -> Result<Page, String> {
            let index = cursor
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            self.pages
                .get(index)
                .cloned()
                .ok_or_else(|| "missing page".to_owned())
        }
        async fn call_tool(
            &mut self,
            _request: McpCallToolRequest,
        ) -> Result<CallToolResult, String> {
            Err("unused".to_owned())
        }
    }

    #[tokio::test]
    async fn pagination_behavior_tests_follows_pages_and_keeps_partial_catalog() {
        let mut session = Fake {
            pages: vec![
                Page {
                    tools: vec![McpTool {
                        name: "one".to_owned(),
                        description: None,
                        input_schema: serde_json::Value::Null,
                    }],
                    next_cursor: Some("1".to_owned()),
                },
                Page {
                    tools: vec![],
                    next_cursor: Some("1".to_owned()),
                },
            ],
        };
        let tools = list_tools_with_pagination(
            &mut session,
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .await;
        assert_eq!(tools.len(), 1);
    }
}
