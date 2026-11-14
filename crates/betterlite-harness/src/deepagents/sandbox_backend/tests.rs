#![cfg(test)]

use tokio::task::yield_now;

use crate::deepagents::transformation::message::{ToolCall, ToolResult};
use crate::types::JsonObject;

use super::{ModelTool, TOOL_FILTER_NAME, ToolFilterMiddleware};

fn tool(name: &str) -> ModelTool {
    ModelTool {
        name: name.to_owned(),
        description: None,
        parameters: JsonObject::new(),
    }
}

fn call(name: &str) -> ToolCall {
    ToolCall {
        id: format!("call-{name}"),
        name: name.to_owned(),
        args: JsonObject::new(),
    }
}

fn filter(blocked: &[&str]) -> ToolFilterMiddleware {
    ToolFilterMiddleware {
        blocked: blocked.iter().map(|name| (*name).to_owned()).collect(),
    }
}

#[test]
fn middleware_keeps_the_litellm_name() {
    assert_eq!(TOOL_FILTER_NAME, "LiteLLMHarnessToolFilter");
}

#[test]
fn filter_tools_hides_blocked_names_and_keeps_the_order() {
    let middleware = filter(&["write_file", "execute"]);
    let tools = vec![
        tool("read_file"),
        tool("write_file"),
        tool("execute"),
        tool("grep"),
    ];
    let kept = middleware.filter_tools(tools);
    let names: Vec<&str> = kept.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(names, ["read_file", "grep"]);
}

#[test]
fn filter_tools_keeps_every_tool_when_nothing_is_blocked() {
    let tools = vec![tool("read_file"), tool("write_file")];
    let kept = filter(&[]).filter_tools(tools);
    assert_eq!(kept.len(), 2);
}

#[test]
fn refusal_returns_the_error_result_for_a_blocked_call() {
    let blocked = filter(&["execute"]);
    let result = blocked.refusal(&call("execute")).unwrap();
    assert_eq!(
        result,
        ToolResult {
            id: "call-execute".to_owned(),
            output: "Error: execute is disabled for this session.".to_owned(),
            is_error: true,
        }
    );
}

#[test]
fn refusal_allows_an_unblocked_call() {
    assert!(
        filter(&["write_file"])
            .refusal(&call("read_file"))
            .is_none()
    );
}

#[tokio::test]
async fn the_same_filter_serves_an_async_handler() {
    let middleware = filter(&["execute"]);
    let kept = middleware.filter_tools(vec![tool("read_file"), tool("execute")]);
    assert_eq!(kept.len(), 1);
    yield_now().await;
    assert!(middleware.refusal(&call("execute")).is_some());
}
