use secrecy::SecretString;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const DEFAULT_DEVICE_CODE_CLIENT_ID: &str = "26be6b09-91c5-47da-9861-d2d2bb7a7e36";

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _environment: (),
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub resource: Option<String>,
    pub id_token: Option<SecretString>,
    pub github_actions: bool,
    pub request_url: Option<String>,
    pub request_token: Option<SecretString>,
    pub device_code_client_id: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            resource: None,
            id_token: None,
            github_actions: false,
            request_url: None,
            request_token: None,
            device_code_client_id: DEFAULT_DEVICE_CODE_CLIENT_ID.to_owned(),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        Ok(Settings {
            resource: present("HF_OIDC_RESOURCE"),
            id_token: present("HF_OIDC_ID_TOKEN").map(SecretString::from),
            github_actions: present("GITHUB_ACTIONS").is_some_and(|value| value == "true"),
            request_url: present("ACTIONS_ID_TOKEN_REQUEST_URL"),
            request_token: present("ACTIONS_ID_TOKEN_REQUEST_TOKEN").map(SecretString::from),
            device_code_client_id: present("HF_DEVICE_CODE_OAUTH_CLIENT_ID")
                .unwrap_or_else(|| DEFAULT_DEVICE_CODE_CLIENT_ID.to_owned()),
        })
    }
}
