use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use secrecy::SecretString;

use crate::credentials::AZURE_AI_API_KEY_VAR;
use crate::credentials::agent::AgentTokenRequest;
use crate::credentials::auth_headers::AuthHeaderRequest;
use crate::credentials::host::ApiKeyHeader;
use crate::credentials::model_info::ModelInfoAccessors;

use super::support::FixtureSources;

pub(super) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn object<const N: usize>(entries: [(&str, Json); N]) -> Object {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

pub(super) fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn auth_request<'data>(
    api_key: Option<&'data SecretString>,
    params: Option<&'data Object>,
    environment: &'data BTreeMap<String, String>,
    header: ApiKeyHeader,
    sources: &'data FixtureSources,
) -> AuthHeaderRequest<'data> {
    AuthHeaderRequest {
        api_key,
        params,
        environment,
        api_key_header: header,
        api_key_env_var: AZURE_AI_API_KEY_VAR,
        refresh_enabled: false,
        sources,
    }
}

pub(super) fn agent_request<'data>(
    params: &'data Object,
    environment: &'data BTreeMap<String, String>,
    secrets: &'data SecretLookup,
    sources: &'data FixtureSources,
) -> AgentTokenRequest<'data> {
    AgentTokenRequest {
        params,
        environment,
        secrets,
        sources,
    }
}

pub(super) fn accessors<'data>(
    global_api_base: Option<&'data str>,
    global_api_key: Option<&'data SecretString>,
    environment_api_base: Option<&'data str>,
    environment_api_key: Option<&'data SecretString>,
) -> ModelInfoAccessors<'data> {
    ModelInfoAccessors {
        global_api_base,
        global_api_key,
        environment_api_base,
        environment_api_key,
    }
}
