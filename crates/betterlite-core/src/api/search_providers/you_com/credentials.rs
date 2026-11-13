use betterlite_config::loader::EnvironmentLookup;
use betterlite_openai::openai_base_config::Headers;
use secrecy::{ExposeSecret as _, SecretString};

pub(super) struct YouComEnvironment<'settings> {
    pub(super) api_key: Option<&'settings SecretString>,
    pub(super) api_base: Option<&'settings str>,
}

impl EnvironmentLookup for YouComEnvironment<'_> {
    fn lookup(&self, name: &str) -> Option<String> {
        match name {
            "YOUCOM_API_KEY" => self.api_key.map(|key| key.expose_secret().to_owned()),
            "YOUCOM_API_BASE" => self.api_base.map(str::to_owned),
            _ => None,
        }
    }
}

pub(super) fn compose_headers(mut headers: Headers, key: Option<&SecretString>) -> Headers {
    headers.insert("Content-Type".to_owned(), "application/json".to_owned());
    headers
        .entry("Accept-Encoding".to_owned())
        .or_insert_with(|| "identity".to_owned());
    if let Some(credential) = key {
        headers.insert(
            "X-API-Key".to_owned(),
            credential.expose_secret().to_owned(),
        );
    }
    headers
}
