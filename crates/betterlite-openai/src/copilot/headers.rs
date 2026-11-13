use secrecy::{ExposeSecret as _, SecretString};
use uuid::Uuid;

use crate::openai_base_config::Headers;

pub const COPILOT_VERSION: &str = "0.26.7";
pub const EDITOR_PLUGIN_VERSION: &str = "copilot-chat/0.26.7";
pub const USER_AGENT: &str = "GitHubCopilotChat/0.26.7";
pub const API_VERSION: &str = "2025-04-01";
pub const DEFAULT_COPILOT_API_BASE: &str = "https://api.githubcopilot.com";

#[must_use]
pub fn copilot_default_headers(api_key: &SecretString, request_id: Option<Uuid>) -> Headers {
    let identifier = request_id.unwrap_or_else(Uuid::new_v4);
    let mut headers = Headers::new();
    headers.insert(
        String::from("Authorization"),
        format!("Bearer {}", api_key.expose_secret()),
    );
    headers.insert(
        String::from("content-type"),
        String::from("application/json"),
    );
    headers.insert(
        String::from("copilot-integration-id"),
        String::from("vscode-chat"),
    );
    headers.insert(
        String::from("editor-version"),
        String::from("vscode/1.95.0"),
    );
    headers.insert(
        String::from("editor-plugin-version"),
        String::from(EDITOR_PLUGIN_VERSION),
    );
    headers.insert(String::from("user-agent"), String::from(USER_AGENT));
    headers.insert(
        String::from("openai-intent"),
        String::from("conversation-panel"),
    );
    headers.insert(
        String::from("x-github-api-version"),
        String::from(API_VERSION),
    );
    headers.insert(String::from("x-request-id"), identifier.to_string());
    headers.insert(
        String::from("x-vscode-user-agent-library-version"),
        String::from("electron-fetch"),
    );
    headers
}
