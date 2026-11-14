mod tests;

use std::collections::BTreeMap;

use betterlite_mcp::credential_contracts::auth_config::oauth::ClientCredentialsConfig;
use betterlite_mcp::credential_contracts::errors::{CredError, CredResult};
use betterlite_mcp::credential_contracts::token_endpoint_auth::{
    TokenEndpointClientAuth, build_token_client_auth,
};
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};

use super::token_post::{ExpiresIn, StringField};

const MISSING_PREFIX: &str = "client_credentials config is missing: ";

#[derive(Debug)]
pub struct PreparedGrant {
    pub token_url: String,
    pub form: Vec<(String, SecretString)>,
    pub headers: Vec<(String, SecretString)>,
    pub identity_key: String,
}

struct RequiredFields<'config> {
    token_url: &'config str,
    client_id: &'config str,
    client_secret: &'config SecretString,
}

pub fn prepare_grant(config: &ClientCredentialsConfig) -> CredResult<PreparedGrant> {
    let fields = required_fields(config)?;
    let TokenEndpointClientAuth {
        headers: auth_headers,
        body: auth_body,
    } = build_token_client_auth(
        config.token_endpoint_auth_method,
        Some(fields.client_id),
        Some(fields.client_secret),
    )
    .map_err(|error| CredError::Misconfigured(error.to_string()))?;
    Ok(PreparedGrant {
        token_url: fields.token_url.to_owned(),
        form: grant_form(config, auth_body),
        headers: auth_headers.into_iter().collect(),
        identity_key: identity_key(config),
    })
}

fn required_fields(config: &ClientCredentialsConfig) -> CredResult<RequiredFields<'_>> {
    let token_url = config
        .token_url
        .as_deref()
        .filter(|value| !value.is_empty());
    let client_id = config
        .client_id
        .as_deref()
        .filter(|value| !value.is_empty());
    let client_secret = config
        .client_secret
        .as_ref()
        .filter(|value| !value.expose_secret().is_empty());
    if let (Some(url), Some(id), Some(secret)) = (token_url, client_id, client_secret) {
        return Ok(RequiredFields {
            token_url: url,
            client_id: id,
            client_secret: secret,
        });
    }
    let missing = [
        ("client_id", client_id.is_none()),
        ("client_secret", client_secret.is_none()),
        ("token_url", token_url.is_none()),
    ]
    .into_iter()
    .filter_map(|(name, absent)| absent.then_some(name))
    .collect::<Vec<_>>();
    Err(CredError::Misconfigured(format!(
        "{MISSING_PREFIX}{}",
        missing.join(", ")
    )))
}

fn grant_form(
    config: &ClientCredentialsConfig,
    auth_body: BTreeMap<String, SecretString>,
) -> Vec<(String, SecretString)> {
    let capacity = auth_body.len().saturating_add(4);
    let mut form: Vec<(String, SecretString)> = Vec::with_capacity(capacity);
    form.push((
        "grant_type".to_owned(),
        SecretString::from("client_credentials".to_owned()),
    ));
    form.extend(auth_body);
    let optional = [
        (
            "scope",
            (!config.scopes.is_empty()).then(|| config.scopes.join(" ")),
        ),
        (
            "audience",
            config.audience.clone().filter(|value| !value.is_empty()),
        ),
        (
            "resource",
            config
                .upstream_resource
                .clone()
                .filter(|value| !value.is_empty()),
        ),
    ];
    for (name, optional_value) in optional {
        if let Some(value) = optional_value {
            form.push((name.to_owned(), SecretString::from(value)));
        }
    }
    form
}

// Any rotation of a value that names the client identity yields a new key, so an old token is never served for it.
fn identity_key(config: &ClientCredentialsConfig) -> String {
    let scopes = config.scopes.join(" ");
    let secret = config
        .client_secret
        .as_ref()
        .map_or("", |value| value.expose_secret());
    let method = config
        .token_endpoint_auth_method
        .map_or("", |method| method.as_str());
    let material = [
        config.token_url.as_deref().unwrap_or(""),
        config.client_id.as_deref().unwrap_or(""),
        secret,
        method,
        &scopes,
        config.audience.as_deref().unwrap_or(""),
        config.upstream_resource.as_deref().unwrap_or(""),
    ]
    .join("\n");
    hex::encode(Sha256::digest(material.as_bytes()))
}

#[must_use]
pub fn parse_expires_in(raw: Option<&ExpiresIn>) -> Option<i64> {
    match raw? {
        ExpiresIn::Flag(_) | ExpiresIn::Other(_) => None,
        ExpiresIn::Text(text) => parse_integer(text),
        ExpiresIn::Number(number) => number.as_i64(),
    }
}

// Python's int() on text: surrounding whitespace, one sign, and single underscores between digits.
fn parse_integer(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    let negative = trimmed.starts_with('-');
    let digits = trimmed.strip_prefix(['-', '+']).unwrap_or(trimmed);
    let mut seen_digit = false;
    let mut pending_underscore = false;
    for byte in digits.bytes() {
        match byte {
            b'0'..=b'9' => {
                seen_digit = true;
                pending_underscore = false;
            }
            b'_' if seen_digit && !pending_underscore => pending_underscore = true,
            _ => return None,
        }
    }
    if !seen_digit {
        return None;
    }
    if pending_underscore {
        return None;
    }
    let plain = digits.replace('_', "");
    let sign = if negative { "-" } else { "" };
    format!("{sign}{plain}").parse::<i64>().ok()
}

#[must_use]
pub fn parse_granted_scopes(raw: Option<&StringField>) -> Vec<String> {
    match raw {
        Some(StringField::Text(text)) => text.split_whitespace().map(str::to_owned).collect(),
        Some(StringField::Other(_)) | None => Vec::new(),
    }
}
