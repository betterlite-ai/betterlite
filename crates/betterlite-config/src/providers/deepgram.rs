use secrecy::SecretString;

use crate::loader::EnvironmentLookup;
use crate::sections::Sections;

const DEFAULT_API_BASE: &str = "https://api.deepgram.com/v1";
const API_KEY_VAR: &str = "DEEPGRAM_API_KEY";
const API_BASE_VAR: &str = "DEEPGRAM_API_BASE";

#[expect(
    clippy::multiple_inherent_impl,
    reason = "Each provider keeps its settings resolver beside its own module; google_pse holds the first impl"
)]
impl Sections {
    #[must_use]
    pub fn deepgram_transcript_settings(
        lookup: &dyn EnvironmentLookup,
    ) -> (Option<SecretString>, String) {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        (
            present(API_KEY_VAR).map(SecretString::from),
            present(API_BASE_VAR).unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
        )
    }
}

mod tests;
