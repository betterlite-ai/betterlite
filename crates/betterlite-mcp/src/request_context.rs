pub mod allowlist;
pub mod elicitation;
pub mod legacy_callbacks;

mod tests;

use http::Extensions;

#[derive(Clone, Debug, Default)]
pub struct McpContext {
    // LiteLLM reads this part of the SDK context as request.scope; the session and peer live in the gateway handler
    pub request_scope: Extensions,
    pub active_toolset_id: Option<String>,
    pub initialize_instructions: Option<String>,
    pub gateway_server_name: Option<String>,
    pub proxy_mode: bool,
}
