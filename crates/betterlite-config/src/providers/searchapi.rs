use secrecy::SecretString;

use crate::loader::EnvironmentLookup;

const DEFAULT_API_BASE: &str = "https://www.searchapi.io/api/v1/search";
const API_KEY_VAR: &str = "SEARCHAPI_API_KEY";
const API_BASE_VAR: &str = "SEARCHAPI_API_BASE";

#[derive(Debug, Clone)]
pub struct SearchApiSearchSettings {
    pub api_key: Option<SecretString>,
    pub api_base: String,
}

impl SearchApiSearchSettings {
    pub fn load(lookup: &dyn EnvironmentLookup) -> Self {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        Self {
            api_key: present(API_KEY_VAR).map(SecretString::from),
            api_base: present(API_BASE_VAR).unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
        }
    }
}

mod tests;
