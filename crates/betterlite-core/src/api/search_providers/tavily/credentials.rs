use betterlite_config::loader::EnvironmentLookup;
use betterlite_openai::openai_base_config::Headers;
use secrecy::{ExposeSecret as _, SecretString};

pub(super) struct TavilyEnvironment<'settings> {
    pub(super) api_key: Option<&'settings SecretString>,
    pub(super) api_base: &'settings str,
}

impl EnvironmentLookup for TavilyEnvironment<'_> {
    fn lookup(&self, name: &str) -> Option<String> {
        match name {
            "TAVILY_API_KEY" => self.api_key.map(|key| key.expose_secret().to_owned()),
            "TAVILY_API_BASE" => Some(self.api_base.to_owned()),
            _ => None,
        }
    }
}

pub(super) fn auth_headers(mut headers: Headers, key: &SecretString) -> Headers {
    headers.insert(
        "Authorization".to_owned(),
        format!("Bearer {}", key.expose_secret()),
    );
    headers.insert("Content-Type".to_owned(), "application/json".to_owned());
    headers
}
