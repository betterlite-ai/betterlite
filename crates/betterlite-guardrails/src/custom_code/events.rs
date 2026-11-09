use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CustomCodeEventHook {
    #[serde(rename = "pre_call")]
    PreCall,
    #[serde(rename = "during_call")]
    DuringCall,
    #[serde(rename = "post_call")]
    PostCall,
    #[serde(rename = "pre_mcp_call")]
    PreMcpCall,
    #[serde(rename = "during_mcp_call")]
    DuringMcpCall,
    #[serde(rename = "logging_only")]
    LoggingOnly,
}
