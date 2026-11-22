use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_secret;
use crate::secret_codec::general_settings;
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    disable_responses_id_security: Option<bool>,
    allow_unmanaged_response_ids: Option<bool>,
    master_key: Option<SecretString>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub disable_responses_id_security: bool,
    pub allow_unmanaged_response_ids: bool,
    // None is "no signing key configured"; Some("") is the empty LITELLM_SALT_KEY, which is still the key
    pub signing_key: Option<SecretString>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let general = self.general_settings;
        let master_key = resolve_secret(general.master_key, "general_settings.master_key", lookup)?;
        // the hook's signing key is LITELLM_SALT_KEY when set, even empty, and the master key otherwise
        let cipher = general_settings(None, master_key, lookup).cipher;
        Ok(Settings {
            disable_responses_id_security: general
                .disable_responses_id_security
                .unwrap_or_default(),
            allow_unmanaged_response_ids: general.allow_unmanaged_response_ids.unwrap_or_default(),
            signing_key: cipher.signing_key,
        })
    }
}
