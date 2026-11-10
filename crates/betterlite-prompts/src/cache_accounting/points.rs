use serde::{Deserialize, Serialize};

use crate::arguments::json::Json;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InjectionRole {
    User,
    System,
    Assistant,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum InjectionIndex {
    Number(i64),
    Text(String),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct MessageInjectionPoint {
    pub role: Option<InjectionRole>,
    pub index: Option<InjectionIndex>,
    pub control: Option<Json>,
    #[serde(
        rename = "_litellm_openai_dialect",
        skip_serializing_if = "Option::is_none"
    )]
    pub openai_dialect: Option<bool>,
    #[serde(
        rename = "_litellm_external_breakpoints",
        skip_serializing_if = "Option::is_none"
    )]
    pub external_breakpoints: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ToolConfigInjectionPoint {
    pub control: Option<Json>,
    #[serde(
        rename = "_litellm_openai_dialect",
        skip_serializing_if = "Option::is_none"
    )]
    pub openai_dialect: Option<bool>,
    #[serde(
        rename = "_litellm_external_breakpoints",
        skip_serializing_if = "Option::is_none"
    )]
    pub external_breakpoints: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "location", rename_all = "snake_case")]
pub enum CacheControlInjectionPoint {
    Message(MessageInjectionPoint),
    ToolConfig(ToolConfigInjectionPoint),
}

// The prompt hook never receives the tools or the request, so these stamps ride on the points; copying keeps config-owned points untouched across requests.
impl CacheControlInjectionPoint {
    #[must_use]
    pub fn with_dialect(&self, dialect: bool) -> Self {
        match self {
            Self::Message(point) => Self::Message(MessageInjectionPoint {
                openai_dialect: Some(dialect),
                ..point.clone()
            }),
            Self::ToolConfig(point) => Self::ToolConfig(ToolConfigInjectionPoint {
                openai_dialect: Some(dialect),
                ..point.clone()
            }),
        }
    }

    #[must_use]
    pub fn with_external_breakpoints(&self, count: usize) -> Self {
        match self {
            Self::Message(point) => Self::Message(MessageInjectionPoint {
                external_breakpoints: Some(count),
                ..point.clone()
            }),
            Self::ToolConfig(point) => Self::ToolConfig(ToolConfigInjectionPoint {
                external_breakpoints: Some(count),
                ..point.clone()
            }),
        }
    }
}
