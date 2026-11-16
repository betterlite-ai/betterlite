pub mod answer_shape;
pub mod blocked_by;
pub mod client_recognition;
pub mod completion_as_chat;
pub mod conversation_hash;
pub mod credential_redaction;
pub mod decision_mapping;
pub mod error;
pub mod first_user_text;
#[cfg(test)]
#[path = "v3/tests/fixtures.rs"]
mod fixtures;
pub mod header_composition;
pub mod messages;
pub mod payload_envelope;
pub mod prompt_messages;
pub mod prompt_text;
pub mod request_body;
pub mod request_header;
pub mod response_mapping;
pub mod route_detection;
pub mod session_resolution;
pub mod system_text;
pub mod user_identity;

pub const DETECT_PATH: &str = "/api/v3/detect";
pub const KEY_PREFIX: &str = "sk_agt_";
pub const SESSION_HEADER: &str = "x-claude-code-session-id";
pub const CLIENT_HEADER: &str = "x-s6r-client";
pub const FORMAT_HEADER: &str = "x-s6r-format";
pub const AGENT_HEADER: &str = "x-s6r-agent";
pub const GATEWAY_NAME: &str = "LiteLLM";
pub const DERIVED_SESSION_PREFIX: &str = "litellm-";
pub const RESPONSE_PHASE: &str = "response-sync";
pub const BLOCK_DECISIONS: [&str; 2] = ["block", "deny"];
pub const UNDECIDED_DECISIONS: [&str; 1] = ["ask"];
pub const BLOCKED_TURN_MEMORY: usize = 10_000;
pub const BLOCKED_TURN_TTL_SECONDS: u64 = 86_400;
pub const PREAMBLE_ROLES: [&str; 2] = ["system", "developer"];
pub const REDACTED_VALUE: &str = "[redacted]";
pub const REDACTED_KEYS: [&str; 2] = ["tools", "mcp_servers"];
// The scrub of these is one level deep on purpose: a function schema that defines a `token` or `headers` property lives under `function.parameters` and must be relayed as sent.
pub const CREDENTIAL_FIELDS: [&str; 3] = ["authorization_token", "authorization", "headers"];
// What the proxy stamps on a master-key call in place of a person; sent onward, each would be recorded as an identity.
pub const PLACEHOLDER_IDENTITIES: [&str; 2] = ["default_user_id", "litellm_proxy_master_key"];
pub const IDENTITY_METADATA_KEYS: [&str; 5] = [
    "user_api_key_end_user_id",
    "user_api_key_user_email",
    "user_api_key_user_id",
    "user_api_key_alias",
    "user_api_key_team_id",
];
// User-Agent prefix, Straiker client value and display name: Straiker recognises a coding agent from the system prompt of its main turns only.
pub const CLIENT_BY_USER_AGENT: [(&str, &str, &str); 1] = [("claude-cli/", "claude", "Claude")];
// An allowlist: the hook's request dict merges the client body with proxy state (`deployment` carries the resolved credential), so only these fields are relayed.
pub const PROVIDER_BODY_KEYS: &[&str] = &[
    "model",
    "messages",
    "tools",
    "tool_choice",
    "functions",
    "function_call",
    "temperature",
    "top_p",
    "n",
    "stream",
    "stream_options",
    "stop",
    "max_tokens",
    "max_completion_tokens",
    "presence_penalty",
    "frequency_penalty",
    "logit_bias",
    "user",
    "response_format",
    "seed",
    "logprobs",
    "top_logprobs",
    "parallel_tool_calls",
    "reasoning_effort",
    "modalities",
    "audio",
    "prediction",
    "store",
    "service_tier",
    "web_search_options",
    "prompt",
    "suffix",
    "echo",
    "best_of",
    "system",
    "stop_sequences",
    "top_k",
    "thinking",
    "container",
    "mcp_servers",
    "context_management",
    "output_format",
    "input",
    "instructions",
    "previous_response_id",
    "truncation",
    "text",
    "include",
    "reasoning",
    "max_output_tokens",
    "background",
    "conversation",
    "session_id",
];

#[cfg(test)]
mod tests;
