use secrecy::SecretString;

use crate::loader::EnvironmentLookup;
use crate::sections::Sections;

const DEFAULT_API_BASE: &str = "https://api.bfl.ai";
const API_KEY_VAR: &str = "BFL_API_KEY";
const FALLBACK_API_KEY_VAR: &str = "BLACK_FOREST_LABS_API_KEY";
const API_BASE_VAR: &str = "BFL_API_BASE";

impl Sections {
    #[must_use]
    pub fn bfl_generation_settings(
        lookup: &dyn EnvironmentLookup,
    ) -> (Option<SecretString>, String) {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        (
            present(API_KEY_VAR)
                .or_else(|| present(FALLBACK_API_KEY_VAR))
                .map(SecretString::from),
            present(API_BASE_VAR).unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
        )
    }
}

mod tests;
