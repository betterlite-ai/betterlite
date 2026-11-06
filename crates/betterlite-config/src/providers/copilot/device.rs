use crate::loader::EnvironmentLookup;
use crate::sections::copilot_device::Settings;

const DEFAULT_TOKEN_DIR: &str = "~/.config/litellm/github_copilot";
const DEFAULT_ACCESS_TOKEN_FILE: &str = "access-token";
const DEFAULT_API_KEY_FILE: &str = "api-key.json";
const DEFAULT_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const DEFAULT_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const DEFAULT_ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const DEFAULT_API_KEY_URL: &str = "https://api.github.com/copilot_internal/v2/token";

pub(crate) fn resolve(lookup: &dyn EnvironmentLookup) -> Settings {
    Settings {
        token_dir: lookup
            .lookup("GITHUB_COPILOT_TOKEN_DIR")
            .unwrap_or_else(|| default_token_dir(lookup)),
        access_token_file: lookup
            .lookup("GITHUB_COPILOT_ACCESS_TOKEN_FILE")
            .unwrap_or_else(|| DEFAULT_ACCESS_TOKEN_FILE.to_owned()),
        api_key_file: lookup
            .lookup("GITHUB_COPILOT_API_KEY_FILE")
            .unwrap_or_else(|| DEFAULT_API_KEY_FILE.to_owned()),
        client_id: lookup
            .lookup("GITHUB_COPILOT_CLIENT_ID")
            .unwrap_or_else(|| DEFAULT_CLIENT_ID.to_owned()),
        device_code_url: lookup
            .lookup("GITHUB_COPILOT_DEVICE_CODE_URL")
            .unwrap_or_else(|| DEFAULT_DEVICE_CODE_URL.to_owned()),
        access_token_url: lookup
            .lookup("GITHUB_COPILOT_ACCESS_TOKEN_URL")
            .unwrap_or_else(|| DEFAULT_ACCESS_TOKEN_URL.to_owned()),
        api_key_url: lookup
            .lookup("GITHUB_COPILOT_API_KEY_URL")
            .unwrap_or_else(|| DEFAULT_API_KEY_URL.to_owned()),
    }
}

fn default_token_dir(lookup: &dyn EnvironmentLookup) -> String {
    let home = if let Some(home) = lookup.lookup("HOME") {
        home
    } else {
        let Some(path) = lookup.system_home() else {
            return DEFAULT_TOKEN_DIR.to_owned();
        };
        let Some(home) = path.to_str() else {
            tracing::warn!(
                "the system home is not UTF-8; the Copilot token path keeps its literal default"
            );
            return DEFAULT_TOKEN_DIR.to_owned();
        };
        home.to_owned()
    };
    format!(
        "{}/.config/litellm/github_copilot",
        home.trim_end_matches('/')
    )
}
