use serde::Serialize;
use serde_json::Value;
use std::sync::LazyLock;

use super::{
    AGENT_SEARCH_TOOL_NAME, MCP_PROXY_CALL_TOOL_NAME, MCP_PROXY_SCHEMA_TOOL_NAME,
    MCP_PROXY_SEARCH_TOOL_NAME, MCP_TOOL_CALL_TOOL_NAME, MCP_TOOL_SEARCH_TOOL_NAME,
    SKILL_SEARCH_TOOL_NAME,
};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct VirtualToolDefinition {
    pub name: &'static str,
    pub description: &'static str,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TextToolResult {
    pub content: Vec<TextContent>,
    #[serde(rename = "isError")]
    pub is_error: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TextContent {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub text: String,
}

pub fn text_tool_result(text: &str, is_error: bool) -> TextToolResult {
    TextToolResult {
        content: vec![TextContent {
            kind: "text",
            text: text.to_owned(),
        }],
        is_error,
    }
}

static VIRTUAL_TOOL_DEFINITIONS: LazyLock<Vec<VirtualToolDefinition>> = LazyLock::new(|| {
    vec![
        virtual_definition(
            MCP_TOOL_SEARCH_TOOL_NAME,
            "Search for MCP tools by describing what you need. Returns top matching tools with names, descriptions, and input schemas.",
            &[
                (
                    "query",
                    "string",
                    "What the tool should do, matched against names and descriptions.",
                ),
                ("top_k", "integer", "Maximum number of results to return."),
            ],
            &["query"],
        ),
        virtual_definition(
            MCP_TOOL_CALL_TOOL_NAME,
            "Call an MCP tool by name with the given arguments.",
            &[
                (
                    "tool_name",
                    "string",
                    "The exact name of the MCP tool to call.",
                ),
                ("arguments", "object", "Arguments to pass to the tool."),
            ],
            &["tool_name"],
        ),
        virtual_definition(
            AGENT_SEARCH_TOOL_NAME,
            "Find A2A agents by describing the task in natural language. Returns the best matching agents you can access, ranked by semantic similarity, each with its agent_id, name, description, skills, and score.",
            &[
                (
                    "query",
                    "string",
                    "The task the agent should be able to do, in natural language.",
                ),
                ("top_k", "integer", "Maximum number of agents to return."),
            ],
            &["query"],
        ),
        virtual_definition(
            SKILL_SEARCH_TOOL_NAME,
            "Find registered skills by describing what you need in natural language. Returns the best matching skills you can access, ranked by semantic similarity, each with its skill_id, display_title, description, and score.",
            &[
                (
                    "query",
                    "string",
                    "What you need the skill to do, in natural language.",
                ),
                ("top_k", "integer", "Maximum number of skills to return."),
            ],
            &["query"],
        ),
        virtual_definition(
            MCP_PROXY_SEARCH_TOOL_NAME,
            "Search accessible MCP tools by describing what you need. Returns opaque tool IDs.",
            &[("query", "string", "What the tool should do.")],
            &["query"],
        ),
        virtual_definition(
            MCP_PROXY_SCHEMA_TOOL_NAME,
            "Return the complete schema for an accessible MCP tool ID.",
            &[("tool_id", "string", "Opaque ID from search_tools.")],
            &["tool_id"],
        ),
        virtual_definition(
            MCP_PROXY_CALL_TOOL_NAME,
            "Call an accessible MCP tool by opaque ID with schema-valid arguments.",
            &[
                ("tool_id", "string", "Opaque ID from search_tools."),
                (
                    "arguments",
                    "object",
                    "Arguments validated against the selected tool schema.",
                ),
            ],
            &["tool_id"],
        ),
    ]
});

pub fn get_virtual_tool_definitions() -> Vec<VirtualToolDefinition> {
    let mut definitions = Vec::new();
    VIRTUAL_TOOL_DEFINITIONS
        .iter()
        .filter(|definition| {
            matches!(
                definition.name,
                MCP_TOOL_SEARCH_TOOL_NAME
                    | MCP_TOOL_CALL_TOOL_NAME
                    | AGENT_SEARCH_TOOL_NAME
                    | SKILL_SEARCH_TOOL_NAME
            )
        })
        .for_each(|definition| definitions.push(definition.clone()));
    definitions
}

pub fn get_mcp_proxy_tool_definitions() -> Vec<VirtualToolDefinition> {
    VIRTUAL_TOOL_DEFINITIONS
        .iter()
        .fold(Vec::new(), |mut definitions, definition| {
            if matches!(
                definition.name,
                MCP_PROXY_SEARCH_TOOL_NAME | MCP_PROXY_SCHEMA_TOOL_NAME | MCP_PROXY_CALL_TOOL_NAME
            ) {
                definitions.push(definition.clone());
            }
            definitions
        })
}

fn virtual_definition(
    name: &'static str,
    description: &'static str,
    fields: &[(&str, &str, &str)],
    required: &[&str],
) -> VirtualToolDefinition {
    let properties: serde_json::Map<String, Value> = fields
        .iter()
        .map(|(name, kind, help)| {
            let mut field = serde_json::Map::new();
            field.insert("type".to_owned(), Value::String((*kind).to_owned()));
            field.insert("description".to_owned(), Value::String((*help).to_owned()));
            if *kind == "integer" && matches!(*name, "top_k") {
                field.insert("default".to_owned(), Value::from(5));
            }
            ((*name).to_owned(), Value::Object(field))
        })
        .collect();
    VirtualToolDefinition {
        name,
        description,
        input_schema: serde_json::json!({"type":"object", "properties":properties, "required":required}),
    }
}
