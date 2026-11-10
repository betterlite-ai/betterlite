pub mod choices;
pub mod error;
pub mod io;
pub mod merge;
pub mod paths;
pub mod receipts;
pub mod statusline_install;

use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;

// Claude Code owns this file's schema, so it stays an ordered JSON document instead of a typed model
pub type SettingsDocument = BTreeMap<String, JsonValue>;

pub const ENV_KEY: &str = "env";
pub const API_KEY_HELPER_KEY: &str = "apiKeyHelper";
pub const MODEL_KEY: &str = "model";
pub const STATUS_LINE_KEY: &str = "statusLine";
pub const ANTHROPIC_BASE_URL_KEY: &str = "ANTHROPIC_BASE_URL";
pub const ANTHROPIC_AUTH_TOKEN_KEY: &str = "ANTHROPIC_AUTH_TOKEN";
pub const ANTHROPIC_API_KEY_KEY: &str = "ANTHROPIC_API_KEY";
pub const ANTHROPIC_MODEL_KEY: &str = "ANTHROPIC_MODEL";
pub const ENABLE_TOOL_SEARCH_KEY: &str = "ENABLE_TOOL_SEARCH";
pub const ENABLE_TOOL_SEARCH_VALUE: &str = "true";
pub const GATEWAY_DISCOVERY_KEY: &str = "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY";
pub const GATEWAY_DISCOVERY_VALUE: &str = "1";
pub const TIER_MODEL_ENV_KEYS: [&str; 4] = [
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_FABLE_MODEL",
];

pub const OWNED_PATHS: [&str; 9] = [
    "env.ENABLE_TOOL_SEARCH",
    "env.CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY",
    "env.ANTHROPIC_BASE_URL",
    "env.ANTHROPIC_AUTH_TOKEN",
    "env.ANTHROPIC_API_KEY",
    "env.ANTHROPIC_MODEL",
    "apiKeyHelper",
    "model",
    "statusLine",
];
pub const CREDENTIAL_PATHS: [&str; 3] = [
    "env.ANTHROPIC_API_KEY",
    "env.ANTHROPIC_AUTH_TOKEN",
    "apiKeyHelper",
];
pub const MODEL_PATHS: [&str; 2] = ["model", "env.ANTHROPIC_MODEL"];
pub const BASE_URL_PATH: &str = "env.ANTHROPIC_BASE_URL";
