use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use secrecy::{ExposeSecret, SecretString};
use url::{Host, Url};

use super::Params;
use super::error::MongoDbError;
use super::prepare::reject_unknown_params;
use crate::api::vector_store_records::records::vector::VectorStoreIndexEndpoints;
use crate::vector_stores::Headers;

const MISSING_PARAMS: &str = "Configure api_base and api_key for the MongoDB BETA sidecar.";
const MISSING_API_KEY: &str =
    "MongoDB sidecar api_key is required. Set api_key or MONGODB_SIDECAR_API_KEY.";
const MISSING_BASE: &str =
    "MongoDB sidecar api_base is required, for example http://127.0.0.1:8080.";
const INVALID_URL: &str = "MongoDB sidecar api_base must be a valid HTTP or HTTPS URL.";
const STRUCTURED_URL: &str = "MongoDB sidecar api_base must be an HTTP or HTTPS URL without credentials, query, or fragment.";
const HTTPS_REQUIRED: &str =
    "MongoDB sidecar requires HTTPS. HTTP is supported only for a loopback IP such as 127.0.0.1.";

#[derive(Debug, Clone, Default)]
pub struct MongoEnv {
    pub api_key: Option<SecretString>,
}

pub fn validate_environment(
    mut headers: Headers,
    litellm_params: Option<&Params>,
    env: &MongoEnv,
) -> Result<Headers, MongoDbError> {
    let params = litellm_params.ok_or_else(|| MongoDbError::BadRequest {
        message: MISSING_PARAMS.to_owned(),
    })?;
    reject_unknown_params(params)?;
    let explicit = explicit_api_key(params)?;
    let from_env = env
        .api_key
        .as_ref()
        .filter(|key| !key.expose_secret().is_empty());
    let key = explicit
        .or_else(|| from_env.map(ExposeSecret::expose_secret))
        .ok_or_else(|| MongoDbError::BadRequest {
            message: MISSING_API_KEY.to_owned(),
        })?;
    headers.insert(
        "Authorization".to_owned(),
        SecretString::from(format!("Bearer {key}")),
    );
    headers.insert(
        "Content-Type".to_owned(),
        SecretString::from("application/json"),
    );
    Ok(headers)
}

fn explicit_api_key(params: &Params) -> Result<Option<&str>, MongoDbError> {
    match params.get("api_key") {
        Some(Json::Text(key)) if !key.is_empty() => Ok(Some(key)),
        None | Some(Json::Null | Json::Text(_)) => Ok(None),
        Some(_) => Err(MongoDbError::BadRequest {
            message: "api_key must be a string".to_owned(),
        }),
    }
}

pub fn get_complete_url(api_base: Option<&str>) -> Result<String, MongoDbError> {
    let Some(base) = api_base.filter(|candidate| !candidate.is_empty()) else {
        return Err(MongoDbError::BadRequest {
            message: MISSING_BASE.to_owned(),
        });
    };
    // urlsplit raises ValueError only for a bad port or bracketed host, and LiteLLM gives that its own message
    let parsed = Url::parse(base).map_err(|error| MongoDbError::BadRequest {
        message: parse_error_message(error).to_owned(),
    })?;
    let valid = matches!(parsed.scheme(), "http" | "https")
        && parsed.host().is_some()
        && parsed.port() != Some(0);
    let credentialed = !parsed.username().is_empty() || parsed.password().is_some();
    if !valid
        || credentialed
        || parsed.query().is_some_and(|query| !query.is_empty())
        || parsed
            .fragment()
            .is_some_and(|fragment| !fragment.is_empty())
    {
        return Err(MongoDbError::BadRequest {
            message: STRUCTURED_URL.to_owned(),
        });
    }
    if parsed.scheme() == "http" && !is_loopback(parsed.host().as_ref()) {
        return Err(MongoDbError::BadRequest {
            message: HTTPS_REQUIRED.to_owned(),
        });
    }
    Ok(base.trim_end_matches('/').to_owned())
}

const fn parse_error_message(error: url::ParseError) -> &'static str {
    if matches!(
        error,
        url::ParseError::InvalidPort | url::ParseError::InvalidIpv6Address
    ) {
        return INVALID_URL;
    }
    STRUCTURED_URL
}

const fn is_loopback(host: Option<&Host<&str>>) -> bool {
    match host {
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        Some(Host::Domain(_)) | None => false,
    }
}

#[must_use]
pub const fn endpoints() -> VectorStoreIndexEndpoints {
    VectorStoreIndexEndpoints {
        read: Some(Vec::new()),
        write: Some(Vec::new()),
        extra: BTreeMap::new(),
    }
}
