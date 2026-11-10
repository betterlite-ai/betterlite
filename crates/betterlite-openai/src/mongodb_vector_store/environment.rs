use std::collections::BTreeMap;
use std::net::IpAddr;

use betterlite_models::decisions::Json;
use secrecy::{ExposeSecret, SecretString};
use url::Url;

use super::Params;
use super::error::MongoDbError;
use super::prepare::reject_unknown_params;
use crate::api::vector_store_records::records::vector::{
    BaseVectorStoreAuthCredentials, VectorStoreIndexEndpoints,
};
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
    // urlsplit removes ASCII tab and line breaks before it splits, and its components come from that text
    let cleaned = base.replace(['\t', '\r', '\n'], "");
    // urlsplit raises ValueError only for a bad port or bracketed host, and LiteLLM gives that its own message
    let parsed = Url::parse(&cleaned).map_err(|error| MongoDbError::BadRequest {
        message: parse_error_message(error).to_owned(),
    })?;
    let authority = written_authority(&cleaned);
    let host = checked_host(parsed.scheme(), authority)?;
    if parsed.query().is_some_and(|query| !query.is_empty())
        || parsed
            .fragment()
            .is_some_and(|fragment| !fragment.is_empty())
    {
        return Err(MongoDbError::BadRequest {
            message: STRUCTURED_URL.to_owned(),
        });
    }
    if parsed.scheme() == "http" && !is_loopback(host) {
        return Err(MongoDbError::BadRequest {
            message: HTTPS_REQUIRED.to_owned(),
        });
    }
    Ok(base.trim_end_matches('/').to_owned())
}

// LiteLLM checks the written host and port first, then the credentials, and refuses both with one message
fn checked_host<'authority>(
    scheme: &str,
    authority: Option<Authority<'authority>>,
) -> Result<&'authority str, MongoDbError> {
    let host = authority.map_or("", |value| value.host);
    if !matches!(scheme, "http" | "https") || host.is_empty() {
        return Err(MongoDbError::BadRequest {
            message: STRUCTURED_URL.to_owned(),
        });
    }
    let port = match authority.and_then(|value| value.port) {
        None => None,
        Some(text) => match text.parse::<u16>() {
            Ok(number) => Some(number),
            Err(_error) => {
                return Err(MongoDbError::BadRequest {
                    message: INVALID_URL.to_owned(),
                });
            }
        },
    };
    let credentialed = authority.is_some_and(|value| {
        !value.username.is_empty() || value.password.is_some_and(|password| !password.is_empty())
    });
    if port == Some(0) || credentialed {
        return Err(MongoDbError::BadRequest {
            message: STRUCTURED_URL.to_owned(),
        });
    }
    Ok(host)
}

#[derive(Clone, Copy)]
struct Authority<'base> {
    username: &'base str,
    password: Option<&'base str>,
    host: &'base str,
    port: Option<&'base str>,
}

// urlsplit reads the netloc as written, while url rewrites 127.1 into 127.0.0.1 and ends the authority at a backslash
fn written_authority(base: &str) -> Option<Authority<'_>> {
    let (_, after_scheme) = base.split_once(':')?;
    let rest = after_scheme.strip_prefix("//")?;
    let netloc = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let (userinfo, hostinfo) = netloc.rsplit_once('@').unwrap_or(("", netloc));
    let (username, password) = match userinfo.split_once(':') {
        Some((username, password)) => (username, Some(password)),
        None => (userinfo, None),
    };
    let (host, port) = host_and_port(hostinfo);
    Some(Authority {
        username,
        password,
        host,
        port,
    })
}

fn host_and_port(hostinfo: &str) -> (&str, Option<&str>) {
    if let Some(bracketed) = hostinfo.strip_prefix('[') {
        let (host, after) = bracketed.split_once(']').unwrap_or((bracketed, ""));
        let port = after.split_once(':').map_or("", |(_, port)| port);
        return (host, non_empty_port(port));
    }
    let (host, port) = hostinfo
        .split_once(':')
        .map_or((hostinfo, ""), |(host, port)| (host, port));
    (host, non_empty_port(port))
}

const fn non_empty_port(port: &str) -> Option<&str> {
    if port.is_empty() { None } else { Some(port) }
}

fn is_loopback(host: &str) -> bool {
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => address.is_loopback(),
        Ok(IpAddr::V6(address)) => address
            .to_ipv4_mapped()
            .map_or_else(|| address.is_loopback(), |mapped| mapped.is_loopback()),
        Err(_error) => false,
    }
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

// LiteLLM's get_vector_store_endpoints_by_type adds no read or write route for MongoDB
#[must_use]
pub const fn endpoints() -> VectorStoreIndexEndpoints {
    VectorStoreIndexEndpoints {
        read: Some(Vec::new()),
        write: Some(Vec::new()),
        extra: BTreeMap::new(),
    }
}

// LiteLLM's get_auth_credentials adds nothing to the caller's headers or query parameters
#[must_use]
pub const fn auth_credentials() -> BaseVectorStoreAuthCredentials {
    BaseVectorStoreAuthCredentials {
        headers: None,
        query_params: None,
        extra: BTreeMap::new(),
    }
}
