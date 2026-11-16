use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use thiserror::Error;

use crate::api::runway::tasks::error::RunwayTaskError;

#[derive(Debug, Error)]
pub enum RunwayVideoError {
    #[error(transparent)]
    Task(#[from] RunwayTaskError),
    #[error(
        "RunwayML API key is required. Set RUNWAYML_API_SECRET environment variable or pass \
         api_key parameter."
    )]
    MissingCredentials,
}

// RunwayMLError: the video error keeps the HTTP status and the response headers it was built from
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("{message}")]
pub struct RunwayMLError {
    pub status_code: u16,
    pub message: String,
    pub headers: BTreeMap<String, Json>,
}

// get_error_class hands every RunwayML call failure to this error, headers included
#[must_use]
pub fn get_error_class(message: &str, status_code: u16, headers: &Headers) -> RunwayMLError {
    RunwayMLError {
        status_code,
        message: message.to_owned(),
        headers: headers
            .iter()
            .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
            .collect(),
    }
}
