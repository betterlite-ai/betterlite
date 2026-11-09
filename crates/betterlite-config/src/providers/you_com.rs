use secrecy::SecretString;

use crate::loader::EnvironmentLookup;

#[derive(Debug, Clone)]
pub(crate) struct YouComSearchSettings {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
}

impl YouComSearchSettings {
    #[must_use]
    pub(crate) fn load(lookup: &dyn EnvironmentLookup) -> Self {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        Self {
            api_key: present("YOUCOM_API_KEY").map(SecretString::from),
            api_base: present("YOUCOM_API_BASE"),
        }
    }
}

mod tests;
