pub mod agent;
pub mod auth_headers;
pub mod entra;
pub mod error;
pub mod host;
pub mod model_info;
pub mod native;
pub mod params;

mod tests;

pub const AZURE_AI_AGENTS_SCOPE: &str = "https://ai.azure.com/.default";
pub const AZURE_AI_API_BASE_VAR: &str = "AZURE_AI_API_BASE";
pub const AZURE_AI_API_KEY_VAR: &str = "AZURE_AI_API_KEY";
pub const AZURE_ENTRA_CREDENTIAL_HELP: &str = "Set `tenant_id` + `client_id` + `client_secret`, `azure_ad_token` (an `oidc/` token also needs `tenant_id` + `client_id`), or `client_id` + `azure_username` + `azure_password` in the agent's `litellm_params`";
pub const ENTRA_PARAM_KEYS: [&str; 3] = ["azure_ad_token", "client_secret", "azure_password"];
const OS_ENVIRON_PREFIX: &str = "os.environ/";
