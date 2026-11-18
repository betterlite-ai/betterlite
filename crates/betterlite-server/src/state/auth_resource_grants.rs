use secrecy::ExposeSecret as _;

use crate::state::shared::Shared;
use betterlite_config::types::Config;

// LiteLLM runs the vector store check only when proxy_server.prisma_client exists
pub struct Slot {
    pub database: bool,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, _shared: &Shared) -> Self {
        let urls = &config.sections.database_urls;
        Self {
            database: configured(urls.config_url.as_ref())
                || configured(urls.endpoints.database_url.as_ref()),
        }
    }
}

fn configured(url: Option<&secrecy::SecretString>) -> bool {
    url.is_some_and(|value| !value.expose_secret().is_empty())
}
