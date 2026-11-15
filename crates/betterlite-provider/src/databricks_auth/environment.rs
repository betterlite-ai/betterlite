use secrecy::SecretString;

// The caller fills these from the process environment at call time; no config section holds them
#[derive(Debug, Clone)]
pub struct DatabricksEnv {
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub api_base: Option<String>,
    pub user_agent_override: Option<String>,
    pub version: String,
}
