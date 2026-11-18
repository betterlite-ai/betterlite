use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::azure_search::{
    CONNECTION_ID_ENV, MODEL_ENV, PROJECT_ENDPOINT_ENV, Settings, TOKEN_ENV,
};
use secrecy::ExposeSecret as _;

pub(crate) struct SettingsEnvironment<'settings> {
    pub(crate) settings: &'settings Settings,
}

impl EnvironmentLookup for SettingsEnvironment<'_> {
    fn lookup(&self, name: &str) -> Option<String> {
        match name {
            PROJECT_ENDPOINT_ENV => self.settings.project_endpoint.clone(),
            MODEL_ENV => self.settings.model.clone(),
            CONNECTION_ID_ENV => self.settings.connection_id.clone(),
            TOKEN_ENV => self
                .settings
                .token
                .as_ref()
                .map(|token| token.expose_secret().to_owned()),
            _ => None,
        }
    }
}

// ServerCredential trusts a base only while a server key exists, so the probe supplies a fake token
pub(crate) struct ProbeEnvironment<'settings> {
    pub(crate) settings: &'settings Settings,
}

impl EnvironmentLookup for ProbeEnvironment<'_> {
    fn lookup(&self, name: &str) -> Option<String> {
        if name == TOKEN_ENV {
            return Some(String::from("pending-minted-token"));
        }
        let environment = SettingsEnvironment {
            settings: self.settings,
        };
        environment.lookup(name)
    }
}
