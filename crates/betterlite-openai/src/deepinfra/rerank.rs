use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::api::rerank_contracts::records::RerankDocument;

pub mod response;

pub type Params = IndexMap<String, Json>;
pub type Headers = BTreeMap<String, String>;
pub type HeaderMap = BTreeMap<String, Json>;
pub const SUPPORTED_RERANK_PARAMS: [&str; 2] = ["query", "documents"];
const OVERLAY_PARAMS: [&str; 5] = [
    "queries",
    "documents",
    "service_tier",
    "instruction",
    "webhook",
];

#[derive(Debug, Clone, Copy)]
pub struct RerankLimits {
    pub max_body_bytes: usize,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RerankError {
    #[error("Deepinfra API Base is required; set it in the call or in DEEPINFRA_API_BASE")]
    MissingApiBase,
    #[error("Deepinfra API key is required; please set DEEPINFRA_API_KEY")]
    MissingApiKey,
}

pub fn complete_url(api_base: Option<&str>, model: &str) -> Result<String, RerankError> {
    let Some(base) = api_base.filter(|value| !value.is_empty()) else {
        return Err(RerankError::MissingApiBase);
    };
    let cleaned = base.replace("openai", "");
    Ok(format!(
        "{}/inference/{model}",
        cleaned.trim_end_matches('/')
    ))
}

pub struct EnvironmentRequest<'request> {
    pub headers: &'request Headers,
    pub api_key: Option<&'request SecretString>,
    pub environment_key: Option<&'request SecretString>,
}

pub fn validate_environment(request: &EnvironmentRequest<'_>) -> Result<Headers, RerankError> {
    let key = request
        .api_key
        .or(request.environment_key)
        .ok_or(RerankError::MissingApiKey)?;
    let mut headers = Headers::from([
        (
            String::from("Authorization"),
            format!("Bearer {}", key.expose_secret()),
        ),
        (String::from("accept"), String::from("application/json")),
        (
            String::from("content-type"),
            String::from("application/json"),
        ),
    ]);
    for (name, value) in request.headers {
        headers.insert(name.clone(), value.clone());
    }
    Ok(headers)
}

pub struct MapRequest<'request> {
    pub non_default_params: &'request Params,
    pub query: &'request str,
    pub documents: &'request [RerankDocument],
}

#[must_use]
pub fn map_cohere_rerank_params(request: &MapRequest<'_>) -> Params {
    let mut optional = Params::new();
    if !request.query.is_empty() {
        optional.insert(
            String::from("queries"),
            Json::List(vec![
                Json::Text(request.query.to_owned());
                request.documents.len()
            ]),
        );
    }
    for name in OVERLAY_PARAMS {
        if let Some(value) = request.non_default_params.get(name)
            && !matches!(value, Json::Null)
        {
            optional.insert(name.to_owned(), value.clone());
        }
    }
    optional
}

#[must_use]
pub fn transform_rerank_request(optional: Option<&Params>) -> Params {
    match optional {
        Some(params) => params.clone(),
        None => Params::new(),
    }
}
