use std::sync::{Arc, Mutex, PoisonError};

use indexmap::IndexMap;
use thiserror::Error;

use crate::arize::phoenix::client::{ClientError, PhoenixClient};
use crate::arize::phoenix::parsing::{ParsingError, parse_prompt_data};
use crate::arize::phoenix::records::PromptTemplate;

#[derive(Debug, Error)]
pub enum LoadError {
    #[error("Failed to load prompt version '{prompt_version_id}' from Arize Phoenix: {source}")]
    Fetch {
        prompt_version_id: String,
        #[source]
        source: ClientError,
    },
    #[error(
        "Failed to load prompt version '{prompt_version_id}' from Arize Phoenix: Prompt version '{prompt_version_id}' not found"
    )]
    NotFound { prompt_version_id: String },
    #[error("Failed to load prompt version '{prompt_version_id}' from Arize Phoenix: {source}")]
    Parse {
        prompt_version_id: String,
        #[source]
        source: ParsingError,
    },
}

// Templates stay for the life of the manager; Phoenix prompt versions are immutable, so there is no expiry.
#[derive(Debug, Default)]
pub struct TemplateCache {
    templates: Mutex<IndexMap<String, Arc<PromptTemplate>>>,
}

impl TemplateCache {
    pub async fn load(
        &self,
        client: &PhoenixClient,
        prompt_version_id: &str,
    ) -> Result<Arc<PromptTemplate>, LoadError> {
        if let Some(cached) = self
            .templates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(prompt_version_id)
        {
            return Ok(Arc::clone(cached));
        }
        let data = client
            .get_prompt_version(prompt_version_id)
            .await
            .map_err(|source| LoadError::Fetch {
                prompt_version_id: prompt_version_id.to_owned(),
                source,
            })?
            .ok_or_else(|| LoadError::NotFound {
                prompt_version_id: prompt_version_id.to_owned(),
            })?;
        let template = Arc::new(
            parse_prompt_data(data, prompt_version_id).map_err(|source| LoadError::Parse {
                prompt_version_id: prompt_version_id.to_owned(),
                source,
            })?,
        );
        self.templates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(prompt_version_id.to_owned(), Arc::clone(&template));
        Ok(template)
    }
}

mod tests;
