use secrecy::SecretString;

use crate::loader::EnvironmentLookup;
use crate::sections::Sections;

const DEFAULT_API_BASE: &str = "https://www.googleapis.com/customsearch/v1";
const API_KEY_VAR: &str = "GOOGLE_PSE_API_KEY";
const API_BASE_VAR: &str = "GOOGLE_PSE_API_BASE";
const ENGINE_ID_VAR: &str = "GOOGLE_PSE_ENGINE_ID";

impl Sections {
    #[must_use]
    pub fn google_pse_search_settings(
        lookup: &dyn EnvironmentLookup,
    ) -> (Option<SecretString>, Option<String>, String) {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        (
            present(API_KEY_VAR).map(SecretString::from),
            present(ENGINE_ID_VAR),
            present(API_BASE_VAR).unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
        )
    }
}

mod tests;
