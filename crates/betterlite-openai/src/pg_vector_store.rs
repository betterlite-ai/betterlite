use betterlite_http::url_path::encode_segment;
use secrecy::{ExposeSecret as _, SecretString};

use crate::vector_stores::Headers;
use crate::vector_stores::Request;
use crate::vector_stores::store::{Query, SearchParams, search_request as openai_search_request};
use error::PgVectorError;

pub mod error;
mod tests;

#[derive(Debug, Clone, Default)]
pub struct PgVectorEnv {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct PgVectorParams {
    pub api_key: Option<SecretString>,
}

pub fn validate_environment(
    mut headers: Headers,
    litellm_params: Option<&PgVectorParams>,
    env: &PgVectorEnv,
) -> Result<Headers, PgVectorError> {
    let explicit = litellm_params
        .and_then(|params| params.api_key.as_ref())
        .filter(|key| !key.expose_secret().is_empty());
    let api_key = explicit
        .or_else(|| {
            env.api_key
                .as_ref()
                .filter(|key| !key.expose_secret().is_empty())
        })
        .ok_or(PgVectorError::MissingApiKey)?;
    headers.insert(
        "Authorization".to_owned(),
        SecretString::from(format!("Bearer {}", api_key.expose_secret())),
    );
    headers.insert(
        "Content-Type".to_owned(),
        SecretString::from("application/json"),
    );
    Ok(headers)
}

pub fn get_complete_url(
    api_base: Option<&str>,
    env: &PgVectorEnv,
) -> Result<String, PgVectorError> {
    let base = api_base
        .filter(|base| !base.is_empty())
        .or_else(|| env.api_base.as_deref().filter(|base| !base.is_empty()))
        .ok_or(PgVectorError::MissingApiBase)?;
    Ok(format!("{}/v1/vector_stores", base.trim_end_matches('/')))
}

pub fn search_request(
    vector_store_id: &str,
    query: &Query,
    params: &SearchParams,
    api_base: &str,
) -> Result<Request, PgVectorError> {
    let encoded = encode_segment(vector_store_id, "vector_store_id")?;
    // PG Vector keeps the OpenAI search body but builds its own URL from the complete base
    let openai = openai_search_request(vector_store_id, query, params, api_base)
        .map_err(|source| PgVectorError::OpenAi { source })?;
    Ok(Request {
        url: format!("{api_base}/{encoded}/search"),
        body: openai.body,
    })
}
