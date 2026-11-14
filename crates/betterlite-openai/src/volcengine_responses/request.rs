use secrecy::{ExposeSecret as _, SecretString};

use crate::openai_base_config::Headers;
use crate::volcengine_common::{DEFAULT_BASE, headers as volcengine_headers};

use super::error::VolcengineResponsesError;

const RESPONSES_SUFFIX: &str = "/responses";
const VERSION_SUFFIX: &str = "/api/v3";

// the four places LiteLLM reads a key from, in the order it tries them
#[derive(Clone, Copy)]
pub struct KeySources<'sources> {
    pub litellm_params: Option<&'sources SecretString>,
    pub global: Option<&'sources SecretString>,
    pub ark: Option<&'sources SecretString>,
    pub volcengine: Option<&'sources SecretString>,
}

// the four places LiteLLM reads a base from, in the order it tries them
#[derive(Clone, Copy)]
pub struct BaseSources<'sources> {
    pub api_base: Option<&'sources str>,
    pub global: Option<&'sources str>,
    pub volcengine: Option<&'sources str>,
    pub ark: Option<&'sources str>,
}

// a missing key is LiteLLM's ValueError, and an empty string counts as unset
pub fn validate_environment(
    headers: &Headers,
    sources: KeySources<'_>,
) -> Result<Headers, VolcengineResponsesError> {
    let key = [
        sources.litellm_params,
        sources.global,
        sources.ark,
        sources.volcengine,
    ]
    .into_iter()
    .flatten()
    .find(|key| !key.expose_secret().is_empty())
    .ok_or(VolcengineResponsesError::MissingApiKey)?;
    Ok(volcengine_headers(key, headers))
}

// the first nonempty base wins, its trailing slashes go, then the version and resource path
#[must_use]
pub fn get_complete_url(sources: BaseSources<'_>) -> String {
    let chosen = [
        sources.api_base,
        sources.global,
        sources.volcengine,
        sources.ark,
    ]
    .into_iter()
    .flatten()
    .find(|base| !base.is_empty())
    .unwrap_or(DEFAULT_BASE);
    let base = chosen.trim_end_matches('/');
    if base.ends_with(RESPONSES_SUFFIX) {
        base.to_owned()
    } else if base.ends_with(VERSION_SUFFIX) {
        format!("{base}{RESPONSES_SUFFIX}")
    } else {
        format!("{base}{VERSION_SUFFIX}{RESPONSES_SUFFIX}")
    }
}
