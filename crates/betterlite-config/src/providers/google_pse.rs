use secrecy::SecretString;

use crate::loader::EnvironmentLookup;

const DEFAULT_API_BASE: &str = "https://www.googleapis.com/customsearch/v1";
const API_KEY_VAR: &str = "GOOGLE_PSE_API_KEY";
const API_BASE_VAR: &str = "GOOGLE_PSE_API_BASE";
const ENGINE_ID_VAR: &str = "GOOGLE_PSE_ENGINE_ID";

#[derive(Debug, Clone)]
pub struct GooglePseSearchSettings {
    pub api_key: Option<SecretString>,
    pub engine_id: Option<String>,
    pub api_base: String,
}

impl GooglePseSearchSettings {
    pub fn load(lookup: &dyn EnvironmentLookup) -> Self {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        Self {
            api_key: present(API_KEY_VAR).map(SecretString::from),
            engine_id: present(ENGINE_ID_VAR),
            api_base: present(API_BASE_VAR).unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
        }
    }
}

mod tests;
