use secrecy::SecretString;

pub const DEFAULT_BASE_URL: &str = "https://api.vantage.sh";

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error(
        "api_key must be provided for Vantage destination (set VANTAGE_API_KEY env var or pass in destination_config)"
    )]
    MissingApiKey,
    #[error(
        "integration_token must be provided for Vantage destination (set VANTAGE_INTEGRATION_TOKEN env var or pass in destination_config)"
    )]
    MissingIntegrationToken,
}

#[derive(Debug, Default)]
pub struct VantageSettings {
    pub api_key: Option<SecretString>,
    pub integration_token: Option<SecretString>,
    pub base_url: Option<String>,
}
