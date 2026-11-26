use serde::{Deserialize, Serialize};

// The litellm_settings keys the shared update serves, the five settings LiteLLM's update routes reach
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingKey {
    DefaultInternalUserParams,
    DefaultTeamParams,
    McpSemanticToolFilter,
    WebsearchInterceptionParams,
    McpToolSearch,
}

impl SettingKey {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DefaultInternalUserParams => "default_internal_user_params",
            Self::DefaultTeamParams => "default_team_params",
            Self::McpSemanticToolFilter => "mcp_semantic_tool_filter",
            Self::WebsearchInterceptionParams => "websearch_interception_params",
            Self::McpToolSearch => "mcp_tool_search",
        }
    }
}
