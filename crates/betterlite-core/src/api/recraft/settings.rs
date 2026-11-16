use secrecy::SecretString;

// LiteLLM's Recraft generation and edit configurations read the same two settings from the environment
#[derive(Debug, Clone)]
pub struct RecraftImageSettings {
    pub api_key: Option<SecretString>,
    pub api_base: String,
}
