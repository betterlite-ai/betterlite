use std::collections::BTreeMap;
use std::sync::Arc;

use secrecy::{ExposeSecret as _, SecretString};

use super::source::{CallableTokenProvider, ProviderFailure, TokenReply};

pub const REDACTED: &str = "***REDACTED***";
const AUTH_HEADERS: [&str; 2] = ["api-key", "authorization"];
const RATE_LIMIT_HEADERS: [&str; 4] = [
    "x-ratelimit-limit-requests",
    "x-ratelimit-remaining-requests",
    "x-ratelimit-limit-tokens",
    "x-ratelimit-remaining-tokens",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderLookup {
    Exact,
    CaseInsensitive,
}

pub struct ClientAuthParams {
    pub azure_ad_token: Option<SecretString>,
    pub azure_ad_token_provider: Option<Arc<dyn CallableTokenProvider>>,
    pub api_key: Option<String>,
}

#[must_use]
pub fn process_azure_headers(
    headers: &[(String, String)],
    lookup: HeaderLookup,
) -> BTreeMap<String, String> {
    let mut output: BTreeMap<String, String> = BTreeMap::new();
    for (name, value) in headers {
        let key = match lookup {
            HeaderLookup::Exact => format!("llm_provider-{name}"),
            HeaderLookup::CaseInsensitive => format!("llm_provider-{}", name.to_ascii_lowercase()),
        };
        match output.get_mut(&key) {
            Some(existing) if lookup == HeaderLookup::CaseInsensitive => {
                existing.push_str(", ");
                existing.push_str(value);
            }
            _ => {
                output.insert(key, value.clone());
            }
        }
    }
    for name in RATE_LIMIT_HEADERS {
        if let Some(value) = find_header(headers, name, lookup) {
            output.insert(name.to_owned(), value.to_owned());
        }
    }
    output
}

fn find_header<'headers>(
    headers: &'headers [(String, String)],
    name: &str,
    lookup: HeaderLookup,
) -> Option<&'headers str> {
    headers.iter().find_map(|(candidate, value)| match lookup {
        HeaderLookup::Exact => (candidate == name).then_some(value.as_str()),
        HeaderLookup::CaseInsensitive => candidate
            .eq_ignore_ascii_case(name)
            .then_some(value.as_str()),
    })
}

pub fn get_azure_request_auth_headers(
    headers: &[(String, String)],
    client: &ClientAuthParams,
) -> Result<Vec<(String, String)>, ProviderFailure> {
    let mut output = headers.to_vec();
    let has_auth = headers.iter().any(|(name, _)| {
        AUTH_HEADERS
            .iter()
            .any(|auth| name.eq_ignore_ascii_case(auth))
    });
    if has_auth {
        return Ok(output);
    }
    if let Some(token) = resolve_azure_ad_token(client)? {
        output.push((
            "Authorization".to_owned(),
            format!("Bearer {}", token.expose_secret()),
        ));
        return Ok(output);
    }
    if let Some(api_key) = client.api_key.as_deref().filter(|key| !key.is_empty()) {
        output.push(("api-key".to_owned(), api_key.to_owned()));
    }
    Ok(output)
}

fn resolve_azure_ad_token(
    client: &ClientAuthParams,
) -> Result<Option<SecretString>, ProviderFailure> {
    if let Some(token) = client
        .azure_ad_token
        .as_ref()
        .filter(|token| !token.expose_secret().is_empty())
    {
        return Ok(Some(token.clone()));
    }
    let Some(provider) = client.azure_ad_token_provider.as_ref() else {
        return Ok(None);
    };
    match provider.token()? {
        TokenReply::Text(token) if !token.expose_secret().is_empty() => Ok(Some(token)),
        TokenReply::Text(_) | TokenReply::NonText { .. } => Ok(None),
    }
}

#[must_use]
pub fn redact_azure_auth_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            let secret = AUTH_HEADERS
                .iter()
                .any(|auth| name.eq_ignore_ascii_case(auth));
            (
                name.clone(),
                if secret { REDACTED } else { value }.to_owned(),
            )
        })
        .collect()
}
