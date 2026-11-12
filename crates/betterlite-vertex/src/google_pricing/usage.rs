use betterlite_models::usage_sum::Usage;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ServerToolUse {
    pub web_search_requests: Option<i64>,
}

// The usage a Google charge bills: the Gemini usage block plus the server_tool_use counter the shared search calculator also reads.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GoogleUsage {
    pub tokens: Usage,
    pub server_tool_use: ServerToolUse,
}
