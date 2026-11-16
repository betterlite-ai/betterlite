use betterlite_config::loader::EnvironmentLookup;
use secrecy::SecretString;
use url::Url;

use super::DEFAULT_TABLE_NAME;

pub const URL_VARIABLE: &str = "SUPABASE_URL";
pub const KEY_VARIABLE: &str = "SUPABASE_KEY";

#[derive(Debug, thiserror::Error)]
pub enum SupabaseSettingsError {
    #[error(
        "LiteLLM Error, trying to use Supabase but url or key not passed. Create a table and set `litellm.supabase_url=<your-url>` and `litellm.supabase_key=<your-key>`"
    )]
    MissingCredentials,
    #[error("SUPABASE_URL is not a valid URL")]
    Url(#[source] url::ParseError),
    #[error("SUPABASE_URL must use the http or https scheme")]
    Scheme,
}

pub struct SupabaseSettings {
    pub url: Url,
    pub key: SecretString,
    pub table_name: String,
}

impl SupabaseSettings {
    // Supabase.__init__ reads both names from os.getenv and rejects a missing one
    pub fn from_environment(
        environment: &dyn EnvironmentLookup,
    ) -> Result<Self, SupabaseSettingsError> {
        let url_value = environment
            .lookup(URL_VARIABLE)
            .filter(|value| !value.is_empty());
        let key_value = environment
            .lookup(KEY_VARIABLE)
            .filter(|value| !value.is_empty());
        let (Some(url), Some(key)) = (url_value, key_value) else {
            return Err(SupabaseSettingsError::MissingCredentials);
        };
        let parsed_url = Url::parse(&url).map_err(SupabaseSettingsError::Url)?;
        if !matches!(parsed_url.scheme(), "http" | "https") {
            return Err(SupabaseSettingsError::Scheme);
        }
        Ok(Self {
            url: parsed_url,
            key: SecretString::from(key),
            table_name: DEFAULT_TABLE_NAME.to_owned(),
        })
    }
}
