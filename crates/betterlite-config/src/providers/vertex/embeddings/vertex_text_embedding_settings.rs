use betterlite_env::EnvError;

use crate::loader::EnvironmentLookup;
use crate::sections::Sections;

// LiteLLM's VertexAITextEmbeddingConfig has no file key, so its defaults come from this environment surface
const AUTO_TRUNCATE_ENV: &str = "VERTEX_AI_TEXT_EMBEDDING_AUTO_TRUNCATE";
const TASK_TYPE_ENV: &str = "VERTEX_AI_TEXT_EMBEDDING_TASK_TYPE";
const TITLE_ENV: &str = "VERTEX_AI_TEXT_EMBEDDING_TITLE";

pub(crate) type VertexTextEmbeddingSettings = (Option<bool>, Option<String>, Option<String>);

#[expect(
    clippy::multiple_inherent_impl,
    reason = "Each provider keeps its settings resolver beside its own module; google_pse holds the first impl"
)]
impl Sections {
    pub fn vertex_text_embedding_settings(
        lookup: &dyn EnvironmentLookup,
    ) -> Result<VertexTextEmbeddingSettings, EnvError> {
        let auto_truncate = match lookup.lookup(AUTO_TRUNCATE_ENV) {
            Some(text) => betterlite_env::flag(AUTO_TRUNCATE_ENV, &text)?,
            None => None,
        };
        Ok((
            auto_truncate,
            lookup
                .lookup(TASK_TYPE_ENV)
                .filter(|value| !value.is_empty()),
            lookup.lookup(TITLE_ENV).filter(|value| !value.is_empty()),
        ))
    }
}

mod tests;
