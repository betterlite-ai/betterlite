use betterlite_env::EnvError;

use crate::loader::EnvironmentLookup;
use crate::rag::document::chunking::{DEFAULT_CHUNK_OVERLAP, DEFAULT_CHUNK_SIZE};
use crate::sections::Sections;

pub(crate) mod dispatch;
pub(crate) mod embeddings;
pub(crate) mod images;
pub(crate) mod thinking;

const DEFER_SETUP_VARIABLE: &str = "LITELLM_GEMINI_LIVE_DEFER_SETUP";
const DEFAULT_CHUNK_SIZE_ENV: &str = "DEFAULT_CHUNK_SIZE";
const DEFAULT_CHUNK_OVERLAP_ENV: &str = "DEFAULT_CHUNK_OVERLAP";

pub(crate) fn gemini_live_defer_setup(
    configured: Option<bool>,
    lookup: &dyn EnvironmentLookup,
) -> bool {
    // LiteLLM lowercases the value and compares it to "true" without trimming
    configured.unwrap_or_else(|| {
        lookup
            .lookup(DEFER_SETUP_VARIABLE)
            .is_some_and(|value| value.to_lowercase() == "true")
    })
}

#[expect(
    clippy::multiple_inherent_impl,
    reason = "Each provider keeps its settings resolver beside its own module; google_pse holds the first impl"
)]
impl Sections {
    // LiteLLM reads int(os.getenv(name, default)) once at import; the loader reads each value per load
    pub fn vertex_rag_chunk_defaults(
        lookup: &dyn EnvironmentLookup,
    ) -> Result<(i64, i64), EnvError> {
        Ok((
            number_setting(lookup, DEFAULT_CHUNK_SIZE_ENV, DEFAULT_CHUNK_SIZE)?,
            number_setting(lookup, DEFAULT_CHUNK_OVERLAP_ENV, DEFAULT_CHUNK_OVERLAP)?,
        ))
    }
}

// the thinking module's env_number reads a number the same way, but it is private to that
// module and that file is outside this task's scope
fn number_setting(
    lookup: &dyn EnvironmentLookup,
    name: &str,
    default: i64,
) -> Result<i64, EnvError> {
    let Some(text) = lookup.lookup(name) else {
        return Ok(default);
    };
    Ok(betterlite_env::number::<i64>(name, &text)?.unwrap_or(default))
}
