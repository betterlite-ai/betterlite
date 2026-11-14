use secrecy::SecretString;

use crate::loader::EnvironmentLookup;
use crate::sections::Sections;

pub(crate) const DEFAULT_API_BASE: &str = "https://fal.run";
const API_KEY_VARIABLE: &str = "FAL_AI_API_KEY";
const API_BASE_VARIABLE: &str = "FAL_AI_API_BASE";

#[expect(
    clippy::multiple_inherent_impl,
    reason = "Each provider keeps its settings resolver beside its own module; google_pse holds the first impl"
)]
impl Sections {
    #[must_use]
    pub fn fal_image_environment(lookup: &dyn EnvironmentLookup) -> (Option<SecretString>, String) {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        (
            present(API_KEY_VARIABLE).map(SecretString::from),
            present(API_BASE_VARIABLE).unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
        )
    }
}

mod tests;
