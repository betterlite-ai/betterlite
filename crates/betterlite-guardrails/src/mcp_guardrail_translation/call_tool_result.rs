use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallToolResult {
    #[serde(default)]
    pub content: Vec<ContentItem>,
    #[serde(
        rename = "structuredContent",
        alias = "structured_content",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub structured_content: Option<GuardrailValue>,
    // isError, _meta and future result fields reach the client as the MCP server sent them
    #[serde(flatten)]
    pub extra: BTreeMap<String, GuardrailValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContentItem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    // image, audio and resource items carry their payload in fields the guardrail never rewrites
    #[serde(flatten)]
    pub extra: BTreeMap<String, GuardrailValue>,
}
