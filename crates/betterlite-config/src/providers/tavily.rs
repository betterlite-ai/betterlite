use secrecy::SecretString;

use crate::loader::EnvironmentLookup;

pub(crate) const TAVILY_API_BASE: &str = "https://api.tavily.com";

#[derive(Debug, Clone)]
pub(crate) struct TavilySearchSettings {
    pub api_key: Option<SecretString>,
    pub api_base: String,
}

impl TavilySearchSettings {
    #[must_use]
    pub(crate) fn load(lookup: &dyn EnvironmentLookup) -> Self {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        Self {
            api_key: present("TAVILY_API_KEY").map(SecretString::from),
            api_base: present("TAVILY_API_BASE").unwrap_or_else(|| TAVILY_API_BASE.to_owned()),
        }
    }
}

mod tests;
