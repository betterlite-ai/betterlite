use thiserror::Error;

use crate::bitbucket_client::error::ClientError;
use crate::dotprompt_templates::render::RenderError;
use crate::dotprompt_templates::template::TemplateError;

use super::parse::ParseError;

#[derive(Debug, Error)]
pub enum ManagerError {
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error("Failed to load prompt '{prompt_id}' from BitBucket: {source}")]
    Fetch {
        prompt_id: String,
        #[source]
        source: ClientError,
    },
    #[error("Failed to load prompt '{prompt_id}' from BitBucket: {source}")]
    Parse {
        prompt_id: String,
        #[source]
        source: ParseError,
    },
    #[error("Template '{template_id}' not found")]
    NotFound { template_id: String },
    #[error(transparent)]
    Sandbox(#[from] TemplateError),
    #[error(transparent)]
    Render(#[from] RenderError),
}
