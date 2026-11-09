use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::str::from_utf8;
use std::sync::PoisonError;
use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};

use super::SecretLookup;
use super::SecretValue;
use super::error::SecretLookupError;
use super::github_response::decode_github_token;
use super::ports::OidcRequest;
use crate::oidc::credential_dirs::resolve_oidc_file_path;
use crate::oidc::http_timeouts::OidcHttpTimeouts;
use crate::oidc::token_ttl::{EXPIRY_MARGIN_SECONDS, oidc_token_cache_ttl};

const GOOGLE_IDENTITY_URL: &str =
    "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/identity";
const GOOGLE_MAX_TTL_SECONDS: i64 = 3540;
const GITHUB_CACHE_SECONDS: i64 = 295;

pub(super) fn read(
    lookup: &SecretLookup,
    environment: &BTreeMap<String, String>,
    secret_name: &str,
    oidc_name: &str,
) -> Result<Option<SecretValue>, SecretLookupError> {
    let (provider, audience) = oidc_name
        .split_once('/')
        .ok_or(SecretLookupError::MissingOidcAudience)?;
    let token = provider_token(lookup, environment, provider, audience, secret_name)?;
    Ok(token.map(SecretValue::Text))
}

fn provider_token(
    lookup: &SecretLookup,
    environment: &BTreeMap<String, String>,
    provider: &str,
    audience: &str,
    secret_name: &str,
) -> Result<Option<SecretString>, SecretLookupError> {
    match provider {
        "google" => google(lookup, audience, secret_name).map(Some),
        "github" => github(lookup, environment, audience, secret_name),
        "azure" => azure(lookup, environment, audience).map(Some),
        "circleci" => environment_token(
            environment,
            "CIRCLE_OIDC_TOKEN",
            SecretLookupError::CircleCiTokenMissing,
        )
        .map(Some),
        "circleci_v2" => environment_token(
            environment,
            "CIRCLE_OIDC_TOKEN_V2",
            SecretLookupError::CircleCiTokenV2Missing,
        )
        .map(Some),
        "file" => read_oidc_file(&resolve_oidc_file_path(audience, environment)?).map(Some),
        "env" => environment_token(
            environment,
            audience,
            SecretLookupError::EnvironmentVariableMissing {
                name: audience.to_owned(),
            },
        )
        .map(Some),
        "env_path" => env_path_token(environment, audience).map(Some),
        _ => Err(SecretLookupError::UnsupportedOidcProvider),
    }
}

fn environment_token(
    environment: &BTreeMap<String, String>,
    name: &str,
    missing: SecretLookupError,
) -> Result<SecretString, SecretLookupError> {
    environment.get(name).map_or_else(
        || Err(missing),
        |value| Ok(SecretString::from(value.as_str())),
    )
}

fn read_oidc_file(path: &Path) -> Result<SecretString, SecretLookupError> {
    fs::read_to_string(path)
        .map(SecretString::from)
        .map_err(|source| SecretLookupError::TokenFileRead {
            path: path.to_path_buf(),
            source,
        })
}

fn env_path_token(
    environment: &BTreeMap<String, String>,
    audience: &str,
) -> Result<SecretString, SecretLookupError> {
    let Some(path) = environment.get(audience) else {
        return Err(SecretLookupError::EnvironmentVariableMissing {
            name: audience.to_owned(),
        });
    };
    read_oidc_file(Path::new(path))
}

fn google(
    lookup: &SecretLookup,
    audience: &str,
    secret_name: &str,
) -> Result<SecretString, SecretLookupError> {
    if let Some(cached) = cached_token(lookup, secret_name) {
        return Ok(cached);
    }
    let outcome = lookup.oidc_http.get(OidcRequest {
        url: GOOGLE_IDENTITY_URL.to_owned(),
        query: vec![("audience".to_owned(), audience.to_owned())],
        headers: vec![("Metadata-Flavor".to_owned(), "Google".to_owned())],
        timeouts: OidcHttpTimeouts::DEFAULT,
    });
    let response = outcome.map_err(|source| SecretLookupError::OidcRequest { source })?;
    if response.status != 200 {
        return Err(SecretLookupError::GoogleOidcFailed);
    }
    let token = from_utf8(&response.body)
        .map_err(|source| SecretLookupError::GoogleTokenUtf8 { source })?;
    let ttl = oidc_token_cache_ttl(token, GOOGLE_MAX_TTL_SECONDS, lookup.clock.unix_seconds());
    if ttl > 0 {
        store_token(lookup, secret_name, token, ttl);
    } else {
        tracing::warn!(
            "Google OIDC token for {secret_name} is already expired or expires within {EXPIRY_MARGIN_SECONDS}s; not caching it"
        );
    }
    Ok(SecretString::from(token))
}

fn github(
    lookup: &SecretLookup,
    environment: &BTreeMap<String, String>,
    audience: &str,
    secret_name: &str,
) -> Result<Option<SecretString>, SecretLookupError> {
    let request_url = environment
        .get("ACTIONS_ID_TOKEN_REQUEST_URL")
        .ok_or(SecretLookupError::GithubRequestMissing)?;
    let request_token = environment
        .get("ACTIONS_ID_TOKEN_REQUEST_TOKEN")
        .ok_or(SecretLookupError::GithubRequestMissing)?;
    if let Some(cached) = cached_token(lookup, secret_name) {
        return Ok(Some(cached));
    }
    let outcome = lookup.oidc_http.get(OidcRequest {
        url: request_url.clone(),
        query: vec![("audience".to_owned(), audience.to_owned())],
        headers: vec![
            (
                "Authorization".to_owned(),
                format!("Bearer {request_token}"),
            ),
            (
                "Accept".to_owned(),
                "application/json; api-version=2.0".to_owned(),
            ),
        ],
        timeouts: OidcHttpTimeouts::DEFAULT,
    });
    let response = outcome.map_err(|source| SecretLookupError::OidcRequest { source })?;
    if response.status != 200 {
        return Err(SecretLookupError::GithubOidcFailed);
    }
    let token = decode_github_token(&response.body)?;
    if let Some(fetched) = &token {
        store_token(
            lookup,
            secret_name,
            fetched.expose_secret(),
            GITHUB_CACHE_SECONDS,
        );
    }
    Ok(token)
}

fn azure(
    lookup: &SecretLookup,
    environment: &BTreeMap<String, String>,
    audience: &str,
) -> Result<SecretString, SecretLookupError> {
    if let Some(path) = environment.get("AZURE_FEDERATED_TOKEN_FILE") {
        return read_oidc_file(Path::new(path));
    }
    tracing::warn!(
        "AZURE_FEDERATED_TOKEN_FILE not found in environment will use Azure AD token provider"
    );
    let token = lookup
        .azure_tokens
        .token(audience)
        .map_err(|source| SecretLookupError::AzureProvider { source })
        .and_then(|token| token.ok_or(SecretLookupError::AzureProviderReturnedNone));
    token.inspect_err(|error| tracing::error!("{error}"))
}

fn cached_token(lookup: &SecretLookup, secret_name: &str) -> Option<SecretString> {
    let now = lookup.clock.now();
    let mut cache = lookup
        .oidc_cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let cached = cache.get(secret_name);
    if let Some((token, expires_at)) = cached
        && *expires_at > now
    {
        return Some(token.clone());
    }
    cache.remove(secret_name);
    None
}

fn store_token(lookup: &SecretLookup, secret_name: &str, token: &str, ttl_seconds: i64) {
    let Some(expires_at) = lookup
        .clock
        .now()
        .checked_add(Duration::from_secs(ttl_seconds.unsigned_abs()))
    else {
        return;
    };
    let mut cache = lookup
        .oidc_cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    cache.insert(
        secret_name.to_owned(),
        (SecretString::from(token), expires_at),
    );
}
