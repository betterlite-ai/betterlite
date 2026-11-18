use secrecy::SecretString;

use crate::loader::EnvironmentLookup;

mod tests;

pub const GOOGLE_CLIENT_SECRET_ENV: &str = "GOOGLE_CLIENT_SECRET";

#[derive(Debug, Clone, Default)]
pub struct GoogleSsoSettings {
    pub client_secret: Option<SecretString>,
}

pub fn load_google_sso_settings<L>(lookup: &L) -> GoogleSsoSettings
where
    L: EnvironmentLookup,
{
    GoogleSsoSettings {
        client_secret: lookup
            .lookup(GOOGLE_CLIENT_SECRET_ENV)
            .filter(|value| !value.is_empty())
            .map(SecretString::from),
    }
}
