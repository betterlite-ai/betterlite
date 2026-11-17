use super::super::support::{Request, Session, list};
use super::{ADAPTIVE_MODEL, prepare};
use crate::bedrock_converse_tools_beta::error::ToolsAndBetaError;

pub(super) fn partitioning(session: &mut Session<'_>) {
    let mixed = prepare(
        session,
        ADAPTIVE_MODEL,
        r#"[{"systemTool": null}, {"type": "function", "function": {"name": "lookup", "parameters": {"type": "object", "properties": {}}}}]"#,
        None,
        "{}",
    );
    assert_eq!(
        mixed.tools,
        list(
            r#"[{"toolSpec": {"inputSchema": {"json": {"type": "object", "properties": {}, "required": []}}, "name": "lookup", "description": "lookup"}}, {"systemTool": null}]"#
        )
    );

    let grounding = prepare(
        session,
        ADAPTIVE_MODEL,
        r#"[{"systemTool": {"name": "grounding"}}]"#,
        None,
        "{}",
    );
    assert_eq!(
        grounding.tools,
        list(r#"[{"systemTool": {"name": "grounding"}}]"#)
    );
}

const TOOL_SEARCH_TOOLS: &str = r#"[{"type": "tool_search_tool_regex_20251119", "function": {"name": "regex_tool", "parameters": {"type": "object", "properties": {}}}}, {"type": "tool_search_tool_bm25_20251119", "function": {"name": "bm25_tool", "parameters": {"type": "object", "properties": {}}}}, {"type": "tool_search_tool_regex_20250101", "function": {"name": "search", "parameters": {"type": "object", "properties": {}}}}]"#;
const SEARCH_CONVERTED: &str = r#"[{"toolSpec": {"inputSchema": {"json": {"type": "object", "properties": {}, "required": []}}, "name": "search", "description": "search"}}]"#;

pub(super) fn tool_search_dropped(session: &mut Session<'_>) {
    let searched = prepare(session, ADAPTIVE_MODEL, TOOL_SEARCH_TOOLS, None, "{}");
    assert_eq!(searched.tools, list(SEARCH_CONVERTED));
}

pub(super) fn conversion(session: &mut Session<'_>) {
    let sanitized = prepare(
        session,
        ADAPTIVE_MODEL,
        r#"[{"type": "function", "function": {"name": "get weather!", "description": "Read it"}}]"#,
        None,
        "{}",
    );
    assert_eq!(
        sanitized.tools,
        list(
            r#"[{"toolSpec": {"inputSchema": {"json": {"type": "object", "properties": {}, "required": []}}, "name": "get_weather_", "description": "Read it"}}]"#
        )
    );
    assert_eq!(sanitized.names.len(), 1);
    assert_eq!(
        sanitized.names.get("get_weather_").map(String::as_str),
        Some("get weather!")
    );

    let unnamed = prepare(
        session,
        ADAPTIVE_MODEL,
        r#"[{"type": "function", "function": {"name": "  "}}]"#,
        None,
        "{}",
    );
    assert_eq!(
        unnamed.tools,
        list(
            r#"[{"toolSpec": {"inputSchema": {"json": {"type": "object", "properties": {}, "required": []}}, "name": "litellm_unnamed_tool_0", "description": "litellm_unnamed_tool_0"}}]"#
        )
    );
    assert!(unnamed.names.is_empty());
}

pub(super) fn schemas(session: &mut Session<'_>) {
    let cases = [
        (
            r#"[{"name": "anthropic-tool", "input_schema": {"type": "object", "properties": {"x": {"type": "number"}}}}]"#,
            r#"[{"toolSpec": {"inputSchema": {"json": {"type": "object", "properties": {"x": {"type": "number"}}, "required": []}}, "name": "anthropic-tool", "description": "anthropic-tool"}}]"#,
        ),
        (
            r#"[{"toolSpec": {"name": "ready", "description": "d", "inputSchema": {"json": {}}}}, {"type": "function", "function": {"name": "later"}}]"#,
            r#"[{"toolSpec": {"name": "ready", "description": "d", "inputSchema": {"json": {}}}}, {"toolSpec": {"inputSchema": {"json": {"type": "object", "properties": {}, "required": []}}, "name": "later", "description": "later"}}]"#,
        ),
    ];
    for (tools, blocks) in cases {
        let converted = prepare(session, ADAPTIVE_MODEL, tools, None, "{}");
        assert_eq!(converted.tools, list(blocks));
    }
}

pub(super) fn failures(session: &mut Session<'_>) {
    match session.process(&Request::new(
        ADAPTIVE_MODEL,
        r#"[{"type": 5}]"#,
        None,
        "{}",
    )) {
        Err(ToolsAndBetaError::Tools(error)) => assert_eq!(error.status_code(), Some(400)),
        Err(_) | Ok(_) => panic!("a non-string tool type must fail as a tool error"),
    }
    match session.process(&Request::new(
        ADAPTIVE_MODEL,
        r#"[{"type": "function", "function": {"name": 5}}]"#,
        None,
        "{}",
    )) {
        Err(ToolsAndBetaError::Tool(error)) => {
            assert_eq!(ToolsAndBetaError::Tool(error).status_code(), None);
        }
        Err(_) | Ok(_) => panic!("a non-string tool name must fail as a tool spec error"),
    }
    match session.process(&Request::new(ADAPTIVE_MODEL, r#"["text"]"#, None, "{}")) {
        Err(ToolsAndBetaError::Tools(error)) => assert_eq!(error.status_code(), Some(400)),
        Err(_) | Ok(_) => panic!("a tool that is not an object must be refused"),
    }
}
