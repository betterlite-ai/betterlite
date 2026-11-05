use crate::loader::EnvironmentLookup;

const DEFAULT_AUTH_FILE: &str = "auth.json";
const DEFAULT_API_BASE: &str = "https://api.x.ai/v1";
const DEFAULT_TOKEN_DIR_UNDER_HOME: &str = ".config/litellm/xai_oauth";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OauthPaths {
    pub(crate) token_dir: String,
    pub(crate) auth_file: String,
    pub(crate) api_base: String,
}

// LiteLLM reads each name with `get_secret_str(...) or default`, so an empty value counts as unset.
pub(crate) fn resolve(lookup: &dyn EnvironmentLookup) -> OauthPaths {
    let set = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
    OauthPaths {
        token_dir: set("XAI_OAUTH_TOKEN_DIR").unwrap_or_else(|| default_token_dir(lookup)),
        auth_file: set("XAI_OAUTH_AUTH_FILE").unwrap_or_else(|| DEFAULT_AUTH_FILE.to_owned()),
        api_base: set("XAI_OAUTH_API_BASE")
            .or_else(|| set("XAI_API_BASE"))
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
    }
}

// os.path.expanduser: HOME wins even when empty, then the account's home, else the "~" stays.
fn default_token_dir(lookup: &dyn EnvironmentLookup) -> String {
    let home = lookup
        .lookup("HOME")
        .or_else(|| lookup.system_home()?.to_str().map(str::to_owned));
    home.map_or_else(
        || format!("~/{DEFAULT_TOKEN_DIR_UNDER_HOME}"),
        |directory| {
            format!(
                "{}/{DEFAULT_TOKEN_DIR_UNDER_HOME}",
                directory.trim_end_matches('/')
            )
        },
    )
}
