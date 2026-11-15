use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;

use crate::mantle_inputs::credentials::resolve_bearer_token;
use crate::mantle_inputs::records::MantleGlobals;

pub const AUTHORIZATION: &str = "Authorization";
pub const PROJECT_HEADER: &str = "OpenAI-Project";

// An absent litellm_params record reads as an empty one, so every field stays optional.
#[derive(Debug, Clone, Copy, Default)]
pub struct EnvironmentParams<'text> {
    pub api_key: Option<&'text SecretString>,
    pub project_id: Option<&'text str>,
}

pub fn validate_environment(
    headers: &mut IndexMap<String, String>,
    params: EnvironmentParams<'_>,
    globals: &MantleGlobals,
) {
    let bearer = resolve_bearer_token(
        params.api_key,
        globals.mantle_api_key.as_ref(),
        globals.aws_bearer_token.as_ref(),
    );
    if let Some(token) = bearer.filter(|value| !value.expose_secret().is_empty()) {
        headers.insert(
            AUTHORIZATION.to_owned(),
            format!("Bearer {}", token.expose_secret()),
        );
    }
    if let Some(project_id) = params.project_id.filter(|value| !value.is_empty()) {
        headers.insert(PROJECT_HEADER.to_owned(), project_id.to_owned());
    }
}
