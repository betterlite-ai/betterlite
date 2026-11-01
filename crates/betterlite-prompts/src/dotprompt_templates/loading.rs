use std::fs;
use std::io;
use std::path::Path;

use thiserror::Error;

use super::frontmatter::{self, FrontmatterError};
use super::template::{PromptTemplate, TemplateError};

#[derive(Debug, Error)]
pub enum PromptManagerError {
    #[error("prompt_id is required when prompt_file is provided")]
    PromptIdRequired,
    #[error("Prompt directory does not exist: {path}")]
    DirectoryMissing { path: String },
    #[error("failed to read prompt file '{path}': {source}")]
    Read {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    Frontmatter(#[from] FrontmatterError),
    #[error(transparent)]
    Template(#[from] TemplateError),
}

pub(super) fn load_prompt_file(
    path: &Path,
    prompt_id: &str,
) -> Result<PromptTemplate, PromptManagerError> {
    let content = fs::read_to_string(path).map_err(|source| PromptManagerError::Read {
        path: path.display().to_string(),
        source,
    })?;
    let (metadata, template_content) = frontmatter::parse_frontmatter(&content)?;
    PromptTemplate::new(
        template_content.trim().to_owned(),
        metadata,
        Some(prompt_id.to_owned()),
    )
    .map_err(PromptManagerError::from)
}

mod tests;
