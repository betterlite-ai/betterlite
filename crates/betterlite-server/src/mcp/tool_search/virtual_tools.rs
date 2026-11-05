use serde::Serialize;

use super::{
    AGENT_SEARCH_TOOL_NAME, MCP_PROXY_CALL_TOOL_NAME, MCP_PROXY_SCHEMA_TOOL_NAME,
    MCP_PROXY_SEARCH_TOOL_NAME, MCP_TOOL_CALL_TOOL_NAME, MCP_TOOL_SEARCH_TOOL_NAME,
    SKILL_SEARCH_TOOL_NAME,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct VirtualToolDefinition {
    pub name: &'static str,
    pub description: &'static str,
    #[serde(rename = "inputSchema")]
    pub input_schema: ToolInputSchema,
}

#[serde_with::serde_as]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ToolInputSchema {
    #[serde(rename = "type")]
    pub kind: SchemaType,
    #[serde_as(as = "&serde_with::Map<_, _>")]
    pub properties: &'static [(&'static str, ToolParamSchema)],
    pub required: &'static [&'static str],
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ToolParamSchema {
    #[serde(rename = "type")]
    pub kind: SchemaType,
    pub description: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SchemaType {
    String,
    Object,
    Integer,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TextToolResult {
    pub content: Vec<TextContent>,
    #[serde(rename = "isError")]
    pub is_error: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum TextContent {
    Text { text: String },
}

pub const VIRTUAL_TOOL_DEFINITIONS: [VirtualToolDefinition; 4] = [
    VirtualToolDefinition {
        name: MCP_TOOL_SEARCH_TOOL_NAME,
        description: "Search for MCP tools by describing what you need. Returns top matching tools with names, descriptions, and input schemas.",
        input_schema: ToolInputSchema {
            kind: SchemaType::Object,
            properties: &[
                (
                    "query",
                    ToolParamSchema {
                        kind: SchemaType::String,
                        description: "What the tool should do, matched against names and descriptions.",
                        default: None,
                    },
                ),
                (
                    "top_k",
                    ToolParamSchema {
                        kind: SchemaType::Integer,
                        description: "Maximum number of results to return.",
                        default: Some(5),
                    },
                ),
            ],
            required: &["query"],
        },
    },
    VirtualToolDefinition {
        name: MCP_TOOL_CALL_TOOL_NAME,
        description: "Call an MCP tool by name with the given arguments.",
        input_schema: ToolInputSchema {
            kind: SchemaType::Object,
            properties: &[
                (
                    "tool_name",
                    ToolParamSchema {
                        kind: SchemaType::String,
                        description: "The exact name of the MCP tool to call.",
                        default: None,
                    },
                ),
                (
                    "arguments",
                    ToolParamSchema {
                        kind: SchemaType::Object,
                        description: "Arguments to pass to the tool.",
                        default: None,
                    },
                ),
            ],
            required: &["tool_name"],
        },
    },
    VirtualToolDefinition {
        name: AGENT_SEARCH_TOOL_NAME,
        description: "Find A2A agents by describing the task in natural language. Returns the best matching agents you can access, ranked by semantic similarity, each with its agent_id, name, description, skills, and score.",
        input_schema: ToolInputSchema {
            kind: SchemaType::Object,
            properties: &[
                (
                    "query",
                    ToolParamSchema {
                        kind: SchemaType::String,
                        description: "The task the agent should be able to do, in natural language.",
                        default: None,
                    },
                ),
                (
                    "top_k",
                    ToolParamSchema {
                        kind: SchemaType::Integer,
                        description: "Maximum number of agents to return.",
                        default: Some(5),
                    },
                ),
            ],
            required: &["query"],
        },
    },
    VirtualToolDefinition {
        name: SKILL_SEARCH_TOOL_NAME,
        description: "Find registered skills by describing what you need in natural language. Returns the best matching skills you can access, ranked by semantic similarity, each with its skill_id, display_title, description, and score.",
        input_schema: ToolInputSchema {
            kind: SchemaType::Object,
            properties: &[
                (
                    "query",
                    ToolParamSchema {
                        kind: SchemaType::String,
                        description: "What you need the skill to do, in natural language.",
                        default: None,
                    },
                ),
                (
                    "top_k",
                    ToolParamSchema {
                        kind: SchemaType::Integer,
                        description: "Maximum number of skills to return.",
                        default: Some(5),
                    },
                ),
            ],
            required: &["query"],
        },
    },
];

pub const MCP_PROXY_TOOL_DEFINITIONS: [VirtualToolDefinition; 3] = [
    VirtualToolDefinition {
        name: MCP_PROXY_SEARCH_TOOL_NAME,
        description: "Search accessible MCP tools by describing what you need. Returns opaque tool IDs.",
        input_schema: ToolInputSchema {
            kind: SchemaType::Object,
            properties: &[(
                "query",
                ToolParamSchema {
                    kind: SchemaType::String,
                    description: "What the tool should do.",
                    default: None,
                },
            )],
            required: &["query"],
        },
    },
    VirtualToolDefinition {
        name: MCP_PROXY_SCHEMA_TOOL_NAME,
        description: "Return the complete schema for an accessible MCP tool ID.",
        input_schema: ToolInputSchema {
            kind: SchemaType::Object,
            properties: &[(
                "tool_id",
                ToolParamSchema {
                    kind: SchemaType::String,
                    description: "Opaque ID from search_tools.",
                    default: None,
                },
            )],
            required: &["tool_id"],
        },
    },
    VirtualToolDefinition {
        name: MCP_PROXY_CALL_TOOL_NAME,
        description: "Call an accessible MCP tool by opaque ID with schema-valid arguments.",
        input_schema: ToolInputSchema {
            kind: SchemaType::Object,
            properties: &[
                (
                    "tool_id",
                    ToolParamSchema {
                        kind: SchemaType::String,
                        description: "Opaque ID from search_tools.",
                        default: None,
                    },
                ),
                (
                    "arguments",
                    ToolParamSchema {
                        kind: SchemaType::Object,
                        description: "Arguments validated against the selected tool schema.",
                        default: None,
                    },
                ),
            ],
            required: &["tool_id"],
        },
    },
];
