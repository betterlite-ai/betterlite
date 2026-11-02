mod tests;

use crate::loader::EnvironmentLookup;

const DEFAULT_TOKEN_DIR: &str = "~/.config/litellm/chatgpt";
const DEFAULT_AUTH_FILE: &str = "auth.json";
const DEFAULT_API_BASE: &str = "https://chatgpt.com/backend-api/codex";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TokenStorePaths {
    pub(crate) token_dir: String,
    pub(crate) auth_file: String,
    pub(crate) api_base: String,
}

pub(crate) fn resolve(lookup: &dyn EnvironmentLookup) -> TokenStorePaths {
    let primary_base = lookup
        .lookup("CHATGPT_API_BASE")
        .filter(|value| !value.is_empty());
    let secondary_base = lookup
        .lookup("OPENAI_CHATGPT_API_BASE")
        .filter(|value| !value.is_empty());
    TokenStorePaths {
        token_dir: lookup
            .lookup("CHATGPT_TOKEN_DIR")
            .unwrap_or_else(|| expand_default_token_dir(lookup)),
        auth_file: lookup
            .lookup("CHATGPT_AUTH_FILE")
            .unwrap_or_else(|| DEFAULT_AUTH_FILE.to_owned()),
        api_base: primary_base
            .or(secondary_base)
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
    }
}

// a HOME the caller did not provide leaves this literal ~ path, matching os.path.expanduser's untouched fallback
fn expand_default_token_dir(lookup: &dyn EnvironmentLookup) -> String {
    let Some(home) = lookup.lookup("HOME") else {
        return DEFAULT_TOKEN_DIR.to_owned();
    };
    format!("{}/.config/litellm/chatgpt", home.trim_end_matches('/'))
}
