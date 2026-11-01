use thiserror::Error;

use super::validation::ValidationError;

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("Prompt '{prompt_id}'{version_suffix} not found. Available prompts: {available}")]
    NotFound {
        prompt_id: String,
        version_suffix: String,
        available: String,
    },
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error("Error rendering template '{prompt_id}': {source}")]
    Template {
        prompt_id: String,
        #[source]
        source: minijinja::Error,
    },
}

pub(super) fn format_available<'ids>(prompt_ids: impl Iterator<Item = &'ids String>) -> String {
    let items: Vec<String> = prompt_ids.map(|id| format!("'{id}'")).collect();
    format!("[{}]", items.join(", "))
}

mod tests;
