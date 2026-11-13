use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::Json as ContractJson;

use crate::gitlab_client::config::GitLabConfig;

use super::error::InitializerError;
use super::init::config;

// Callers lock the shared cell directly, as with dotprompt's GlobalPromptDirectory, so there is no getter.
#[derive(Clone, Debug, Default)]
pub struct GlobalGitLabConfig(pub Arc<Mutex<Option<GitLabConfig>>>);

pub fn set_global_gitlab_config(
    global: &GlobalGitLabConfig,
    config: &ContractJson,
) -> Result<(), InitializerError> {
    let parsed = config::parse_gitlab_config(config)?;
    *global.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(parsed);
    Ok(())
}

mod tests;
