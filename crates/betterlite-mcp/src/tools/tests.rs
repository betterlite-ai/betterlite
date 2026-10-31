#![cfg(test)]

mod mcp_tool_helpers_behavior_tests;
mod pagination_behavior_tests;
mod tool_call_behavior_tests;

use betterlite_prompts::arguments::json::Json;
use serde::Serialize;
use std::collections::VecDeque;
use std::io;
use std::time::Duration;
use tokio::time::sleep;

use super::pagination::{McpSession, Page, Timeouts};
use super::schemas::McpTool;
use super::tool_calls::CallRequest;

struct Fake {
    pages: VecDeque<(Duration, Page)>,
    cursors: Vec<Option<String>>,
    calls: Vec<CallRequest>,
    call_delay: Duration,
}

impl McpSession for Fake {
    type Error = io::Error;
    type Output = CallRequest;

    async fn list_tools(&mut self, cursor: Option<&str>) -> Result<Page, Self::Error> {
        self.cursors.push(cursor.map(str::to_owned));
        let (delay, page) = self
            .pages
            .pop_front()
            .ok_or_else(|| io::Error::other("missing page"))?;
        sleep(delay).await;
        Ok(page)
    }

    async fn call_tool(&mut self, request: CallRequest) -> Result<Self::Output, Self::Error> {
        self.calls.push(request.clone());
        sleep(self.call_delay).await;
        Ok(request)
    }
}

fn fake(pages: Vec<(Duration, Page)>) -> Fake {
    Fake {
        pages: VecDeque::from(pages),
        cursors: Vec::new(),
        calls: Vec::new(),
        call_delay: Duration::ZERO,
    }
}

fn tool() -> McpTool {
    serde_json::from_str(r#"{"name":"echo","inputSchema":{}}"#).unwrap()
}

fn timeouts(client: u64, listing: u64) -> Timeouts {
    Timeouts {
        client: Duration::from_secs(client),
        listing: Duration::from_secs(listing),
        listing_deadline: None,
    }
}

fn assert_json<Wire>(actual: &Wire, expected: &str)
where
    Wire: Serialize,
{
    let encoded = serde_json::to_string(actual).unwrap();
    assert_eq!(
        serde_json::from_str::<Json>(&encoded).unwrap(),
        serde_json::from_str::<Json>(expected).unwrap()
    );
}
