use betterlite_config::loader::EnvironmentLookup;
use betterlite_openai::openai_base_config::Headers;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::RunwayVideoError;

const AUTHORIZATION: &str = "Authorization";
const VERSION_HEADER: &str = "X-Runway-Version";
const CONTENT_TYPE: &str = "Content-Type";
const JSON_CONTENT_TYPE: &str = "application/json";

// one configured identity: each source is a separate slot so two keys can never share one
#[derive(Clone, Debug, Default)]
pub struct VideoCredentials {
    pub api_key: Option<SecretString>,
    pub params_api_key: Option<SecretString>,
    pub global_api_key: Option<SecretString>,
    pub environment_secret: Option<SecretString>,
    pub environment_key: Option<SecretString>,
}

impl VideoCredentials {
    // get_secret_str reads the two variables raw, so an empty RUNWAYML_API_KEY is still the last candidate
    #[must_use]
    pub fn from_environment(lookup: &dyn EnvironmentLookup) -> Self {
        Self {
            api_key: None,
            params_api_key: None,
            global_api_key: None,
            environment_secret: lookup.lookup("RUNWAYML_API_SECRET").map(SecretString::from),
            environment_key: lookup.lookup("RUNWAYML_API_KEY").map(SecretString::from),
        }
    }
}

// validate_environment: a falsy candidate falls through, and only a missing last candidate fails
pub fn validate_environment(
    headers: &Headers,
    credentials: &VideoCredentials,
    api_version: &str,
) -> Result<Headers, RunwayVideoError> {
    let key = [
        credentials.api_key.as_ref(),
        credentials.params_api_key.as_ref(),
        credentials.global_api_key.as_ref(),
        credentials.environment_secret.as_ref(),
    ]
    .into_iter()
    .flatten()
    .find(|secret| !secret.expose_secret().is_empty())
    .or(credentials.environment_key.as_ref())
    .ok_or(RunwayVideoError::MissingCredentials)?;
    let mut authenticated = headers.clone();
    authenticated.insert(
        AUTHORIZATION.to_owned(),
        format!("Bearer {}", key.expose_secret()),
    );
    authenticated.insert(VERSION_HEADER.to_owned(), api_version.to_owned());
    authenticated.insert(CONTENT_TYPE.to_owned(), JSON_CONTENT_TYPE.to_owned());
    Ok(authenticated)
}
